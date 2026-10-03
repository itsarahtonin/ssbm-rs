// SPDX-License-Identifier: GPL-3.0-or-later
// Follows Dolphin's software renderer (GPL-2.0-or-later) in what it draws.

//! Draws the GameCube GPU's command stream with wgpu. Each draw's vertices go through the
//! transform unit on the CPU (ssbm-gx), as Dolphin's software renderer does; the GPU rasterizes
//! them into an EFB-sized target and runs the TEV per pixel in integers (gx.wgsl), sampling
//! textures decoded on the CPU by hand. Copies to the XFB come back as frames, through the
//! XFB's YUV encoding as Dolphin's software renderer makes it.

mod encode;
mod xfb;

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use ssbm_gx::texture::{self, Image};
use ssbm_gx::xform::{self, Output, Viewport};
use ssbm_gx::{Draw, Memory, Sink, State, vertex};

pub const EFB_WIDTH: u32 = 640;
pub const EFB_HEIGHT: u32 = 528;

const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

fn bits(v: u32, at: u32, width: u32) -> u32 {
    (v >> at) & ((1 << width) - 1)
}

fn sext(v: u32, width: u32) -> i32 {
    ((v << (32 - width)) as i32) >> (32 - width)
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuVertex {
    pos: [f32; 4],
    c0: [f32; 4],
    c1: [f32; 4],
    tex: [[f32; 3]; 8],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    bp: [u32; 256],
    colors: [[i32; 4]; 4],
    konsts: [[i32; 4]; 4],
    depth: [f32; 4],
    fog: [f32; 4],
    levels: [[u32; 4]; 2],
    mode: [u32; 4],
}

/// Uniforms are bound at offsets aligned to 256 bytes.
const UNIFORM_STRIDE: usize = std::mem::size_of::<Uniforms>().next_multiple_of(256);

const MODE_DRAW: u32 = 0;
const MODE_CLEAR: u32 = 1;
const MODE_DEPTH_ONLY: u32 = 2;

/// What a pipeline is made for: blending, which channels it writes, and depth.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct PipelineKey {
    /// None, or (src factor, dst factor, subtract) as BP 0x41 has them, and whether the written
    /// alpha is the constant (destination alpha).
    blend: Option<(u32, u32, bool)>,
    const_alpha: bool,
    write_color: bool,
    write_alpha: bool,
    depth_compare: Option<u32>,
    depth_write: bool,
}

struct Command {
    pipeline: PipelineKey,
    uniform: u32,
    textures: [u32; 8],
    first: u32,
    count: u32,
    scissor: [u32; 4],
}

struct TexEntry {
    view: wgpu::TextureView,
    _texture: wgpu::Texture,
    levels: u32,
    last_used: u64,
}

/// A texture as the cache keys it: its image, its palette, and a hash of their bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct TexKey {
    image: (u32, u32, u32, u32, u32),
    tlut_format: u32,
    hash: u64,
}

/// A frame: a copy to the XFB, as shown.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Frame {
    pub fn save_png(&self, path: &std::path::Path) -> std::io::Result<()> {
        let file = std::io::BufWriter::new(std::fs::File::create(path)?);
        let mut encoder = png::Encoder::new(file, self.width, self.height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        let rgb: Vec<u8> = self
            .rgba
            .chunks_exact(4)
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        writer.write_image_data(&rgb)?;
        Ok(())
    }
}

/// Counts of what the renderer met and doesn't draw yet (or draws approximately), by name.
pub type Unsupported = HashMap<&'static str, u64>;

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    efb: wgpu::Texture,
    efb_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    module: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
    uniform_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    pipelines: HashMap<PipelineKey, wgpu::RenderPipeline>,
    vertices: Vec<GpuVertex>,
    uniforms: Vec<u8>,
    commands: Vec<Command>,
    vertex_buffer: Option<wgpu::Buffer>,
    uniform_buffer: Option<(wgpu::Buffer, wgpu::BindGroup)>,
    textures: Vec<Option<TexEntry>>,
    texture_ids: HashMap<TexKey, u32>,
    bind_groups: HashMap<[u32; 8], wgpu::BindGroup>,
    /// Hashes of memory ranges this frame, so a texture's bytes hash once a frame.
    hashes: HashMap<(u32, u32), u64>,
    /// What copies from the EFB to textures wrote, by address: memory as the GPU then reads it.
    copies: Vec<(u32, Vec<u8>)>,
    normals: vertex::NormalCache,
    frame: u64,
    frames: Vec<Frame>,
    on_frame: Option<Box<dyn FnMut(Frame)>>,
    pub unsupported: Unsupported,
}

/// A fast hash of bytes, for telling textures apart.
fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0x9E37_79B9_7F4A_7C15 ^ bytes.len() as u64;
    let mut chunks = bytes.chunks_exact(8);
    for c in &mut chunks {
        let v = u64::from_le_bytes(c.try_into().unwrap());
        h = (h ^ v).wrapping_mul(0xFF51_AFD7_ED55_8CCD).rotate_left(29);
    }
    for &b in chunks.remainder() {
        h = (h ^ u64::from(b)).wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    }
    h ^ (h >> 33)
}

/// Dolphin's rounding of a float to an integer: truncation, then up from a half or more.
fn iround(x: f32) -> i32 {
    let t = x as i32;
    if x - t as f32 >= 0.5 { t + 1 } else { t }
}

/// A position in the EFB, snapped to the rasterizer's sixteenths and moved so the GPU's pixel
/// centers sample where the GameCube's rasterizer does (9/16 into a pixel, as Dolphin's has it).
fn snap(v: f32) -> f32 {
    (iround(16.0 * v) - 1) as f32 / 16.0
}

fn compare_function(func: u32) -> wgpu::CompareFunction {
    use wgpu::CompareFunction as C;
    match func {
        0 => C::Never,
        1 => C::Less,
        2 => C::Equal,
        3 => C::LessEqual,
        4 => C::Greater,
        5 => C::NotEqual,
        6 => C::GreaterEqual,
        _ => C::Always,
    }
}

/// A blend factor of BP 0x41 for the color or alpha component; src1 is the TEV's alpha.
fn blend_factor(factor: u32, is_src: bool) -> wgpu::BlendFactor {
    use wgpu::BlendFactor as F;
    match factor {
        0 => F::Zero,
        1 => F::One,
        2 => {
            if is_src {
                F::Dst
            } else {
                F::Src
            }
        }
        3 => {
            if is_src {
                F::OneMinusDst
            } else {
                F::OneMinusSrc
            }
        }
        4 => F::Src1Alpha,
        5 => F::OneMinusSrc1Alpha,
        6 => F::DstAlpha,
        _ => F::OneMinusDstAlpha,
    }
}

impl Renderer {
    /// A renderer on a headless device.
    pub fn new() -> Result<Self, String> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| format!("no GPU adapter: {e}"))?;
        Self::with_adapter(&adapter)
    }

    pub fn with_adapter(adapter: &wgpu::Adapter) -> Result<Self, String> {
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ssbm-render"),
            required_features: wgpu::Features::DUAL_SOURCE_BLENDING,
            required_limits: wgpu::Limits::default(),
            ..Default::default()
        }))
        .map_err(|e| format!("no GPU device: {e}"))?;
        Ok(Self::with_device(device, queue))
    }

    pub fn with_device(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let size = wgpu::Extent3d {
            width: EFB_WIDTH,
            height: EFB_HEIGHT,
            depth_or_array_layers: 1,
        };
        let efb = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("efb"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COLOR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("efb depth"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gx"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gx.wgsl").into()),
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("state"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Uniforms>() as u64),
                },
                count: None,
            }],
        });
        let texture_entries: Vec<_> = (0..8)
            .map(|i| wgpu::BindGroupLayoutEntry {
                binding: i,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            })
            .collect();
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("textures"),
            entries: &texture_entries,
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gx"),
            bind_group_layouts: &[Some(&uniform_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let mut r = Renderer {
            efb_view: efb.create_view(&Default::default()),
            depth_view: depth.create_view(&Default::default()),
            efb,
            device,
            queue,
            module,
            layout,
            uniform_layout,
            texture_layout,
            pipelines: HashMap::new(),
            vertices: Vec::new(),
            uniforms: Vec::new(),
            commands: Vec::new(),
            vertex_buffer: None,
            uniform_buffer: None,
            textures: Vec::new(),
            texture_ids: HashMap::new(),
            bind_groups: HashMap::new(),
            hashes: HashMap::new(),
            copies: Vec::new(),
            normals: [[0.0; 3]; 3],
            frame: 0,
            frames: Vec::new(),
            on_frame: None,
            unsupported: HashMap::new(),
        };
        // Texture 0: a 1x1 black texture for maps nothing samples.
        r.add_texture(1, 1, &[vec![0; 4]]);
        r
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Hands each frame to `f` as it's finished, instead of keeping it for `take_frames`.
    pub fn on_frame(&mut self, f: impl FnMut(Frame) + 'static) {
        self.on_frame = Some(Box::new(f));
    }

    /// The frames finished since last asked.
    pub fn take_frames(&mut self) -> Vec<Frame> {
        std::mem::take(&mut self.frames)
    }

    fn note(&mut self, what: &'static str) {
        *self.unsupported.entry(what).or_default() += 1;
    }

    fn add_texture(&mut self, width: u32, height: u32, levels: &[Vec<u8>]) -> u32 {
        let mip_count = levels.len() as u32;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: mip_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, rgba) in levels.iter().enumerate() {
            // Our level's size, as Dolphin's software renderer has it, against wgpu's.
            let (ow, oh) = (((width - 1) >> level) + 1, ((height - 1) >> level) + 1);
            let (gw, gh) = ((width >> level).max(1), (height >> level).max(1));
            let (w, h) = (ow.min(gw), oh.min(gh));
            let mut data = Vec::with_capacity((w * h * 4) as usize);
            for row in 0..h {
                let start = (row * ow * 4) as usize;
                data.extend_from_slice(&rgba[start..start + (w * 4) as usize]);
            }
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w * 4),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
        }
        let entry = TexEntry {
            view: texture.create_view(&Default::default()),
            _texture: texture,
            levels: mip_count,
            last_used: self.frame,
        };
        let free = self
            .textures
            .iter()
            .skip(1)
            .position(Option::is_none)
            .map(|i| i + 1);
        match free {
            Some(i) => {
                self.textures[i] = Some(entry);
                i as u32
            }
            None => {
                self.textures.push(Some(entry));
                (self.textures.len() - 1) as u32
            }
        }
    }

    /// Memory as the GPU reads it: with what copies from the EFB wrote over it.
    fn read_memory(&self, mem: &dyn Memory, address: u32, len: u32) -> Vec<u8> {
        let mut buf = vec![0; len as usize];
        mem.read(address, &mut buf);
        let (start, end) = (address as u64, address as u64 + len as u64);
        for (at, bytes) in &self.copies {
            let (cs, ce) = (*at as u64, *at as u64 + bytes.len() as u64);
            let (from, to) = (start.max(cs), end.min(ce));
            if from < to {
                buf[(from - start) as usize..(to - start) as usize]
                    .copy_from_slice(&bytes[(from - cs) as usize..(to - cs) as usize]);
            }
        }
        buf
    }

    fn hash_memory(&mut self, mem: &dyn Memory, address: u32, len: u32) -> u64 {
        if let Some(&h) = self.hashes.get(&(address, len)) {
            return h;
        }
        let buf = self.read_memory(mem, address, len);
        let h = hash_bytes(&buf);
        self.hashes.insert((address, len), h);
        h
    }

    /// The texture texture map `map` samples, made if new.
    fn texture(&mut self, state: &State, map: usize, mem: &dyn Memory) -> (u32, u32) {
        let image = Image::of(&state.bp, map);
        let tlut_reg = state.bp[texture::reg(texture::SETTLUT, map)];
        let tlut_offset = ((tlut_reg & 0x3FF) << 9) as usize;
        let tlut_format = bits(tlut_reg, 10, 2);
        let max_levels = 32 - image.width.max(image.height).leading_zeros();
        let image = Image {
            levels: image.levels.clamp(1, max_levels),
            ..image
        };
        let size = image.size();
        let mut hash = self.hash_memory(mem, image.address, size);
        let tlut = state.tmem.get(tlut_offset..).unwrap_or(&[]);
        if let Some(entries) = texture::palette_entries(image.format) {
            let len = (entries as usize * 2).min(tlut.len());
            hash ^= hash_bytes(&tlut[..len]).rotate_left(17);
        }
        let key = TexKey {
            image: (
                image.address,
                image.width,
                image.height,
                image.format,
                image.levels,
            ),
            tlut_format,
            hash,
        };
        if let Some(&id) = self.texture_ids.get(&key)
            && let Some(Some(entry)) = self.textures.get_mut(id as usize)
        {
            entry.last_used = self.frame;
            return (id, entry.levels);
        }
        let data = self.read_memory(mem, image.address, size);
        let levels = texture::decode(
            &data,
            image.width,
            image.height,
            image.levels,
            image.format,
            tlut,
            tlut_format,
        );
        let id = self.add_texture(image.width, image.height, &levels);
        self.texture_ids.insert(key, id);
        (id, image.levels)
    }

    fn bind_group(&mut self, ids: [u32; 8]) {
        if self.bind_groups.contains_key(&ids) {
            return;
        }
        let views: Vec<_> = ids
            .iter()
            .map(|&id| {
                &self.textures[id as usize]
                    .as_ref()
                    .expect("bound texture")
                    .view
            })
            .collect();
        let entries: Vec<_> = views
            .iter()
            .enumerate()
            .map(|(i, v)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: wgpu::BindingResource::TextureView(v),
            })
            .collect();
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.texture_layout,
            entries: &entries,
        });
        self.bind_groups.insert(ids, group);
    }

    fn uniforms_of(state: &State, levels: [u32; 8], mode: [u32; 4]) -> Uniforms {
        let color = |ra: u32, bg: u32| {
            [
                sext(ra & 0x7FF, 11),
                sext(bg >> 12 & 0x7FF, 11),
                sext(bg & 0x7FF, 11),
                sext(ra >> 12 & 0x7FF, 11),
            ]
        };
        let mut u = Uniforms::zeroed();
        u.bp = state.bp;
        for i in 0..4 {
            u.colors[i] = color(state.color_ra[i], state.color_bg[i]);
            u.konsts[i] = color(state.konst_ra[i], state.konst_bg[i]);
        }
        let vp = Viewport::of(&state.xf);
        u.depth = [vp.z_range, vp.far_z, 0.0, 0.0];
        let float = |v: u32| {
            f32::from_bits(bits(v, 19, 1) << 31 | bits(v, 11, 8) << 23 | bits(v, 0, 11) << 12)
        };
        let (a, c) = (state.bp[0xEE], state.bp[0xF1]);
        let nan_case =
            bits(a, 11, 8) == 255 && bits(c, 11, 8) == 255 && bits(a, 19, 1) == bits(c, 19, 1);
        let (fa, fc) = if nan_case {
            (
                0.0,
                if bits(a, 19, 1) == 0 {
                    f32::NEG_INFINITY
                } else {
                    f32::INFINITY
                },
            )
        } else {
            (float(a), float(c))
        };
        u.fog = [fa, fc, vp.wd, 0.0];
        u.levels = [
            [levels[0], levels[1], levels[2], levels[3]],
            [levels[4], levels[5], levels[6], levels[7]],
        ];
        u.mode = mode;
        u
    }

    fn push_uniforms(&mut self, u: &Uniforms) -> u32 {
        let bytes = bytemuck::bytes_of(u);
        // The same state as the last draw's shares its uniforms.
        if self.uniforms.len() >= UNIFORM_STRIDE {
            let last = self.uniforms.len() - UNIFORM_STRIDE;
            if &self.uniforms[last..last + bytes.len()] == bytes {
                return last as u32;
            }
        }
        let at = self.uniforms.len();
        self.uniforms.extend_from_slice(bytes);
        self.uniforms.resize(at + UNIFORM_STRIDE, 0);
        at as u32
    }

    /// The scissor rectangle in the EFB, cut to the viewport (which the GameCube clips to), and
    /// the scissor's offset.
    fn scissor(state: &State, vp: &Viewport) -> ([u32; 4], (f32, f32)) {
        let tl = state.bp[0x20];
        let br = state.bp[0x21];
        let off = state.bp[0x59];
        let (xo, yo) = ((bits(off, 0, 9) * 2) as i32, (bits(off, 10, 9) * 2) as i32);
        let mut left = bits(tl, 12, 11) as i32 - xo;
        let mut top = bits(tl, 0, 11) as i32 - yo;
        let mut right = bits(br, 12, 11) as i32 - xo + 1;
        let mut bottom = bits(br, 0, 11) as i32 - yo + 1;
        let (vl, vr) = (
            vp.x_orig - vp.wd.abs() - xo as f32,
            vp.x_orig + vp.wd.abs() - xo as f32,
        );
        let (vt, vb) = (
            vp.y_orig - vp.ht.abs() - yo as f32,
            vp.y_orig + vp.ht.abs() - yo as f32,
        );
        left = left.max(vl.round() as i32);
        right = right.min(vr.round() as i32);
        top = top.max(vt.round() as i32);
        bottom = bottom.min(vb.round() as i32);
        let left = left.clamp(0, EFB_WIDTH as i32) as u32;
        let right = right.clamp(0, EFB_WIDTH as i32) as u32;
        let top = top.clamp(0, EFB_HEIGHT as i32) as u32;
        let bottom = bottom.clamp(0, EFB_HEIGHT as i32) as u32;
        (
            [
                left,
                top,
                right.saturating_sub(left),
                bottom.saturating_sub(top),
            ],
            (xo as f32, yo as f32),
        )
    }

    fn gpu_vertex(o: &Output, screen: [f32; 3], off: (f32, f32)) -> GpuVertex {
        let w = o.clip[3];
        let x = snap(screen[0] - off.0) / (EFB_WIDTH as f32 / 2.0) - 1.0;
        let y = 1.0 - snap(screen[1] - off.1) / (EFB_HEIGHT as f32 / 2.0);
        let c = |c: [u8; 4]| {
            [
                f32::from(c[0]),
                f32::from(c[1]),
                f32::from(c[2]),
                f32::from(c[3]),
            ]
        };
        GpuVertex {
            pos: [x * w, y * w, o.clip[2] + w, w],
            c0: c(o.color[0]),
            c1: c(o.color[1]),
            tex: o.tex,
        }
    }

    /// The pipeline and uniforms' mode for the state's blending and depth.
    fn pipeline_key(state: &State) -> PipelineKey {
        let zmode = state.bp[0x40];
        let blend = state.bp[0x41];
        let format = bits(state.bp[0x43], 0, 3);
        let has_alpha = format == 1;
        let test = bits(zmode, 0, 1) != 0;
        PipelineKey {
            blend: (bits(blend, 0, 1) != 0).then(|| {
                (
                    bits(blend, 8, 3),
                    bits(blend, 5, 3),
                    bits(blend, 11, 1) != 0,
                )
            }),
            const_alpha: bits(state.bp[0x42], 8, 1) != 0,
            write_color: bits(blend, 3, 1) != 0,
            write_alpha: bits(blend, 4, 1) != 0 && has_alpha,
            depth_compare: test.then(|| bits(zmode, 1, 3)),
            depth_write: test && bits(zmode, 4, 1) != 0,
        }
    }

    fn pipeline(&mut self, key: PipelineKey) {
        if self.pipelines.contains_key(&key) {
            return;
        }
        let blend = key.blend.map(|(src, dst, subtract)| {
            let component = |is_alpha: bool| {
                if subtract {
                    wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::ReverseSubtract,
                    }
                } else if is_alpha && key.const_alpha {
                    wgpu::BlendComponent::REPLACE
                } else {
                    wgpu::BlendComponent {
                        src_factor: blend_factor(src, true),
                        dst_factor: blend_factor(dst, false),
                        operation: wgpu::BlendOperation::Add,
                    }
                }
            };
            wgpu::BlendState {
                color: component(false),
                alpha: component(true),
            }
        });
        let mut mask = wgpu::ColorWrites::empty();
        if key.write_color {
            mask |= wgpu::ColorWrites::COLOR;
        }
        if key.write_alpha {
            mask |= wgpu::ColorWrites::ALPHA;
        }
        let attrs = wgpu::vertex_attr_array![
            0 => Float32x4, 1 => Float32x4, 2 => Float32x4,
            3 => Float32x3, 4 => Float32x3, 5 => Float32x3, 6 => Float32x3,
            7 => Float32x3, 8 => Float32x3, 9 => Float32x3, 10 => Float32x3
        ];
        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(&self.layout),
                vertex: wgpu::VertexState {
                    module: &self.module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<GpuVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attrs,
                    })],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(key.depth_write),
                    depth_compare: Some(
                        key.depth_compare
                            .map_or(wgpu::CompareFunction::Always, compare_function),
                    ),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &self.module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend,
                        write_mask: mask,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        self.pipelines.insert(key, pipeline);
    }

    /// Clears `rect` of the EFB as a copy with the clear bit does: color, alpha and depth as
    /// their updates are enabled.
    fn clear(&mut self, state: &State, rect: [u32; 4]) {
        let blend = state.bp[0x41];
        let format = bits(state.bp[0x43], 0, 3);
        let mut color = state.bp[0x4F] << 16 | state.bp[0x50] & 0xFFFF;
        // As ARGB from the registers; to RGBA, dropping the precision the format lacks.
        let (a, r, g, b) = (
            color >> 24 & 0xFF,
            color >> 16 & 0xFF,
            color >> 8 & 0xFF,
            color & 0xFF,
        );
        let six = |v: u32| (v & 0xFC) | (v >> 6);
        color = match format {
            1 => six(r) << 24 | six(g) << 16 | six(b) << 8 | six(a),
            _ => r << 24 | g << 16 | b << 8 | a,
        };
        let key = PipelineKey {
            blend: None,
            const_alpha: false,
            write_color: bits(blend, 3, 1) != 0,
            write_alpha: bits(blend, 4, 1) != 0 && format == 1,
            depth_compare: Some(7),
            depth_write: bits(state.bp[0x40], 4, 1) != 0,
        };
        let z = state.bp[0x51] & 0xFF_FFFF;
        self.push_rect(state, key, rect, [MODE_CLEAR, color, z, 0]);
    }

    fn push_rect(&mut self, state: &State, key: PipelineKey, rect: [u32; 4], mode: [u32; 4]) {
        let [x, y, w, h] = rect;
        if w == 0 || h == 0 {
            return;
        }
        let first = self.vertices.len() as u32;
        let corner = |px: u32, py: u32| {
            let nx = px as f32 / (EFB_WIDTH as f32 / 2.0) - 1.0;
            let ny = 1.0 - py as f32 / (EFB_HEIGHT as f32 / 2.0);
            GpuVertex {
                pos: [nx, ny, 0.5, 1.0],
                ..GpuVertex::zeroed()
            }
        };
        let (a, b, c, d) = (
            corner(x, y),
            corner(x + w, y),
            corner(x + w, y + h),
            corner(x, y + h),
        );
        self.vertices.extend_from_slice(&[a, b, c, a, c, d]);
        let u = Self::uniforms_of(state, [1; 8], mode);
        let uniform = self.push_uniforms(&u);
        self.pipeline(key);
        self.bind_group([0; 8]);
        self.commands.push(Command {
            pipeline: key,
            uniform,
            textures: [0; 8],
            first,
            count: 6,
            scissor: rect,
        });
    }

    /// Draws what's batched into the EFB.
    fn flush(&mut self) {
        if self.commands.is_empty() {
            return;
        }
        let vbytes: &[u8] = bytemuck::cast_slice(&self.vertices);
        if self
            .vertex_buffer
            .as_ref()
            .is_none_or(|b| b.size() < vbytes.len() as u64)
        {
            self.vertex_buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("vertices"),
                size: (vbytes.len() as u64).next_power_of_two(),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        if self
            .uniform_buffer
            .as_ref()
            .is_none_or(|(b, _)| b.size() < self.uniforms.len() as u64)
        {
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("uniforms"),
                size: (self.uniforms.len() as u64).next_power_of_two(),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.uniform_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &buffer,
                        offset: 0,
                        size: wgpu::BufferSize::new(std::mem::size_of::<Uniforms>() as u64),
                    }),
                }],
            });
            self.uniform_buffer = Some((buffer, group));
        }
        let vbuf = self.vertex_buffer.as_ref().unwrap();
        let (ubuf, ugroup) = self.uniform_buffer.as_ref().unwrap();
        self.queue.write_buffer(vbuf, 0, vbytes);
        self.queue.write_buffer(ubuf, 0, &self.uniforms);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("efb"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.efb_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_vertex_buffer(0, vbuf.slice(..));
            let mut last: Option<PipelineKey> = None;
            for c in &self.commands {
                if last != Some(c.pipeline) {
                    pass.set_pipeline(&self.pipelines[&c.pipeline]);
                    last = Some(c.pipeline);
                }
                pass.set_bind_group(0, ugroup, &[c.uniform]);
                pass.set_bind_group(1, &self.bind_groups[&c.textures], &[]);
                let [x, y, w, h] = c.scissor;
                pass.set_scissor_rect(x, y, w, h);
                pass.draw(c.first..c.first + c.count, 0..1);
            }
        }
        self.queue.submit([encoder.finish()]);
        self.vertices.clear();
        self.uniforms.clear();
        self.commands.clear();
    }

    /// The EFB's color, read back: RGBA rows of 640.
    fn read_efb(&mut self) -> Vec<u8> {
        let row = EFB_WIDTH * 4;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("efb readback"),
            size: u64::from(row * EFB_HEIGHT),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.efb,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(EFB_HEIGHT),
                },
            },
            wgpu::Extent3d {
                width: EFB_WIDTH,
                height: EFB_HEIGHT,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("GPU readback");
        let data = slice.get_mapped_range().expect("mapped readback").to_vec();
        buffer.unmap();
        data
    }

    /// The FIFO player's clear before a log's first frame: all of the EFB.
    pub fn clear_efb(&mut self, state: &State) {
        self.clear(state, [0, 0, EFB_WIDTH, EFB_HEIGHT]);
    }

    /// Drops textures unused for a while.
    fn evict(&mut self) {
        let frame = self.frame;
        let mut dropped = false;
        for slot in self.textures.iter_mut().skip(1) {
            if slot.as_ref().is_some_and(|e| e.last_used + 120 < frame) {
                *slot = None;
                dropped = true;
            }
        }
        if dropped {
            let textures = &self.textures;
            self.texture_ids
                .retain(|_, id| textures[*id as usize].is_some());
            self.bind_groups
                .retain(|ids, _| ids.iter().all(|&id| textures[id as usize].is_some()));
        }
    }
}

impl Sink for Renderer {
    fn draw(&mut self, state: &State, draw: &Draw<'_>, mem: &dyn Memory) {
        let count = usize::from(draw.count);
        let xf_index = [state.xf[xform::MATINDEX_A], state.xf[xform::MATINDEX_A + 1]];
        let inputs = vertex::decode(
            &state.cp,
            xf_index,
            usize::from(draw.vat),
            draw.data,
            count,
            &mut self.normals,
        );
        let vp = Viewport::of(&state.xf);
        let (scissor, off) = Self::scissor(state, &vp);
        if scissor[2] == 0 || scissor[3] == 0 {
            return;
        }
        let outs: Vec<Output> = inputs
            .iter()
            .map(|v| xform::transform(&state.xf, &state.bp, v))
            .collect();
        let screens: Vec<[f32; 3]> = outs.iter().map(|o| vp.screen(o.clip)).collect();
        let first = self.vertices.len() as u32;
        let cull = bits(state.bp[0x00], 14, 2);
        let tri = |r: &mut Self, a: usize, b: usize, c: usize| {
            let backface = xform::is_backface(&state.xf, outs[a].clip, outs[b].clip, outs[c].clip);
            let culled = if backface {
                cull == 2 || cull == 3
            } else {
                cull == 1 || cull == 3
            };
            if !culled {
                for i in [a, b, c] {
                    r.vertices.push(Self::gpu_vertex(&outs[i], screens[i], off));
                }
            }
        };
        let n = count;
        match draw.primitive {
            0x80 | 0x88 => {
                let mut i = 3;
                while i < n {
                    tri(self, i - 3, i - 2, i - 1);
                    tri(self, i - 3, i - 1, i);
                    i += 4;
                }
                if i == n {
                    tri(self, n - 3, n - 2, n - 1);
                }
            }
            0x90 => {
                let mut i = 2;
                while i < n {
                    tri(self, i - 2, i - 1, i);
                    i += 3;
                }
            }
            0x98 => {
                for i in 2..n {
                    if i % 2 == 0 {
                        tri(self, i - 2, i - 1, i);
                    } else {
                        tri(self, i - 2, i, i - 1);
                    }
                }
            }
            0xA0 => {
                for i in 2..n {
                    tri(self, 0, i - 1, i);
                }
            }
            0xA8 | 0xB0 => {
                let pairs: Vec<(usize, usize)> = if draw.primitive == 0xA8 {
                    (0..n / 2).map(|i| (2 * i, 2 * i + 1)).collect()
                } else {
                    (1..n).map(|i| (i - 1, i)).collect()
                };
                for (a, b) in pairs {
                    self.line(state, &outs[a], screens[a], &outs[b], screens[b], off);
                }
            }
            _ => {
                for i in 0..n {
                    self.point(state, &outs[i], screens[i], off);
                }
            }
        }
        let count = self.vertices.len() as u32 - first;
        if count == 0 {
            return;
        }

        // The texture maps the TEV and indirect stages read.
        let genmode = state.bp[0x00];
        let mut ids = [0u32; 8];
        let mut levels = [1u32; 8];
        let mut maps = Vec::new();
        for n in 0..=bits(genmode, 10, 4) {
            let order = state.bp[0x28 + (n as usize >> 1)];
            let shift = (n & 1) * 12;
            if bits(order, shift + 6, 1) != 0 {
                maps.push(bits(order, shift, 3) as usize);
            }
        }
        for i in 0..bits(genmode, 16, 3) {
            maps.push(bits(state.bp[0x27], 6 * i, 3) as usize);
        }
        if bits(genmode, 16, 3) != 0 {
            self.note("indirect texturing");
        }
        for map in maps {
            if ids[map] == 0 {
                let (id, l) = self.texture(state, map, mem);
                ids[map] = id;
                levels[map] = l;
            }
        }
        self.bind_group(ids);

        let key = Self::pipeline_key(state);
        let blend = state.bp[0x41];
        if bits(blend, 0, 1) == 0 && bits(blend, 1, 1) != 0 && bits(blend, 12, 4) != 3 {
            self.note("logic op");
        }
        if bits(state.bp[0x43], 0, 3) > 1 {
            self.note("EFB format other than RGB8 and RGBA6");
        }
        // A z test before texturing still updates depth where the alpha test then fails: a
        // depth pass, then color where depth now matches.
        let early = bits(state.bp[0x43], 6, 1) != 0;
        let at = state.bp[0xF3];
        let alpha_can_fail = !(0..=255).all(|a| alpha_passes(at, a));
        if early && key.depth_write && alpha_can_fail {
            let depth_key = PipelineKey {
                blend: None,
                write_color: false,
                write_alpha: false,
                ..key
            };
            let u = Self::uniforms_of(state, levels, [MODE_DEPTH_ONLY, 0, 0, 0]);
            let uniform = self.push_uniforms(&u);
            self.pipeline(depth_key);
            self.commands.push(Command {
                pipeline: depth_key,
                uniform,
                textures: ids,
                first,
                count,
                scissor,
            });
            let color_key = PipelineKey {
                depth_compare: Some(2),
                depth_write: false,
                ..key
            };
            let u = Self::uniforms_of(state, levels, [MODE_DRAW, 0, 0, 0]);
            let uniform = self.push_uniforms(&u);
            self.pipeline(color_key);
            self.commands.push(Command {
                pipeline: color_key,
                uniform,
                textures: ids,
                first,
                count,
                scissor,
            });
        } else {
            let u = Self::uniforms_of(state, levels, [MODE_DRAW, 0, 0, 0]);
            let uniform = self.push_uniforms(&u);
            self.pipeline(key);
            self.commands.push(Command {
                pipeline: key,
                uniform,
                textures: ids,
                first,
                count,
                scissor,
            });
        }
    }

    fn copy(&mut self, state: &State, value: u32, _mem: &dyn Memory) {
        let tl = state.bp[0x49];
        let wh = state.bp[0x4A];
        let left = bits(tl, 0, 10);
        let top = bits(tl, 10, 10);
        let right = (left + bits(wh, 0, 10) + 1).min(EFB_WIDTH);
        let bottom = (top + bits(wh, 10, 10) + 1).min(EFB_HEIGHT);
        self.flush();
        if bits(value, 14, 1) != 0 {
            let efb = self.read_efb();
            let frame = xfb::copy(state, value, &efb, [left, top, right, bottom]);
            match &mut self.on_frame {
                Some(f) => f(frame),
                None => self.frames.push(frame),
            }
            self.frame += 1;
            self.hashes.clear();
            self.evict();
        } else {
            let efb = self.read_efb();
            match encode::encode(state, value, &efb) {
                Some(e) => {
                    for (i, row) in e.rows.into_iter().enumerate() {
                        let at = (e.address + i as u32 * e.stride) & 0x01FF_FFFF;
                        let end = at as u64 + row.len() as u64;
                        self.copies.retain(|(a, b)| {
                            !(*a as u64 >= at as u64 && *a as u64 + b.len() as u64 <= end)
                        });
                        self.copies.push((at, row));
                    }
                    self.hashes.clear();
                }
                None => self.note("EFB copy to a texture in a format not encoded"),
            }
        }
        if bits(value, 11, 1) != 0 {
            self.clear(
                state,
                [
                    left,
                    top,
                    right.saturating_sub(left),
                    bottom.saturating_sub(top),
                ],
            );
        }
    }
}

fn alpha_passes(at: u32, a: u32) -> bool {
    let cmp = |r: u32, f: u32| match f {
        0 => false,
        1 => a < r,
        2 => a == r,
        3 => a <= r,
        4 => a > r,
        5 => a != r,
        6 => a >= r,
        _ => true,
    };
    let c0 = cmp(bits(at, 0, 8), bits(at, 16, 3));
    let c1 = cmp(bits(at, 8, 8), bits(at, 19, 3));
    match bits(at, 22, 2) {
        0 => c0 && c1,
        1 => c0 || c1,
        2 => c0 != c1,
        _ => c0 == c1,
    }
}

impl Renderer {
    /// A line: a quad its width wide, its caps vertical or horizontal by its slope.
    fn line(
        &mut self,
        state: &State,
        a: &Output,
        sa: [f32; 3],
        b: &Output,
        sb: [f32; 3],
        off: (f32, f32),
    ) {
        let half = bits(state.bp[0x22], 0, 8) as f32 / 12.0;
        let (dx, dy) = (sb[0] - sa[0], sb[1] - sa[1]);
        let (px, py) = if dx.abs() > dy.abs() {
            (0.0, if dx > 0.0 { -1.0 } else { 1.0 })
        } else {
            (if dy > 0.0 { 1.0 } else { -1.0 }, 0.0)
        };
        let at = |o: &Output, s: [f32; 3], sign: f32| {
            Self::gpu_vertex(
                o,
                [s[0] + sign * px * half, s[1] + sign * py * half, s[2]],
                off,
            )
        };
        if bits(state.bp[0x22], 16, 3) != 0 {
            self.note("line texture offsets");
        }
        let (a0, b0, b1, a1) = (
            at(a, sa, 1.0),
            at(b, sb, 1.0),
            at(b, sb, -1.0),
            at(a, sa, -1.0),
        );
        self.vertices.extend_from_slice(&[b1, b0, a0, a0, a1, b1]);
    }

    /// A point: a square its size wide.
    fn point(&mut self, state: &State, o: &Output, s: [f32; 3], off: (f32, f32)) {
        let r = bits(state.bp[0x22], 8, 8) as f32 / 12.0;
        let at = |x: f32, y: f32| Self::gpu_vertex(o, [s[0] + x * r, s[1] + y * r, s[2]], off);
        if bits(state.bp[0x22], 19, 3) != 0 {
            self.note("point texture offsets");
        }
        let (ll, lr, ur, ul) = (at(-1.0, -1.0), at(1.0, -1.0), at(1.0, 1.0), at(-1.0, 1.0));
        self.vertices.extend_from_slice(&[ll, ul, lr, ur, lr, ul]);
    }
}
