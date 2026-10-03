// SPDX-License-Identifier: GPL-3.0-or-later

//! Copies from the EFB to textures kept on the GPU (copy.wgsl), for the formats Melee copies and
//! samples back as it copied them: its shadows (I4) and a few effects (RGB5A3). The texture holds
//! what the texture decoder would make of what the copy encoder writes, so frames come out as
//! from a copy read back and encoded into memory, without waiting on the GPU for it.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use ssbm_gx::{Memory, State};

use crate::{bits, hash_bytes};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    left: u32,
    top: u32,
    half: u32,
    format: u32,
    yuv: u32,
    six: u32,
    pad: [u32; 2],
}

/// A copy to make on the GPU: where it goes and the texture it makes there.
pub(crate) struct Request {
    params: Params,
    pub format: u32,
    /// The texture's size: the copied rectangle, in texels, as the game samples it.
    pub width: u32,
    pub height: u32,
    pub footprint: Footprint,
}

/// The memory a copy from the EFB writes: rows of whole blocks, `stride` apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Footprint {
    pub address: u32,
    pub stride: u32,
    pub row_bytes: u32,
    pub rows: u32,
}

impl Footprint {
    /// Each row's address and length.
    fn rows(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        (0..self.rows).map(|r| (self.address + r * self.stride, self.row_bytes))
    }

    /// The memory from its first byte to past its last.
    pub fn span(&self) -> (u32, u32) {
        let end = self.address + self.rows.saturating_sub(1) * self.stride + self.row_bytes;
        (self.address, end)
    }
}

/// Block width and height (log 2) and bytes per block of a copy's texture format, as the
/// encoder lays it out; None for a format copies don't make.
fn blocks(format: u32) -> Option<(u32, u32, u32)> {
    match format {
        0 => Some((3, 3, 32)),
        1 | 2 | 7 | 8 | 9 | 10 => Some((3, 2, 32)),
        3 | 4 | 5 | 11 | 12 => Some((2, 2, 32)),
        6 => Some((2, 2, 64)),
        _ => None,
    }
}

/// The copy's texture format (BP 0x52's), and whether it halves the rectangle.
fn copy_format(value: u32) -> (u32, u32) {
    let tp = bits(value, 3, 4);
    (tp / 2 + (tp & 1) * 8, bits(value, 9, 1))
}

/// The memory a copy to a texture (BP 0x52 value `value`) writes, as encode.rs lays it out;
/// None for one it doesn't encode.
pub(crate) fn footprint(state: &State, value: u32) -> Option<Footprint> {
    let (format, half) = copy_format(value);
    if bits(state.bp[0x43], 0, 3) > 1 {
        return None;
    }
    let (lw, lh, block) = blocks(format)?;
    let wh = state.bp[0x4A];
    let (width, height) = (bits(wh, 0, 10) >> half, bits(wh, 10, 10) >> half);
    Some(Footprint {
        address: (state.bp[0x4B] & 0x00FF_FFFF) << 5,
        stride: bits(state.bp[0x4D], 0, 10) << 5,
        row_bytes: ((width >> lw) + 1) * block,
        rows: (height >> lh) + 1,
    })
}

/// The copy BP 0x52 value `value` asks for, if it is one kept on the GPU.
pub(crate) fn request(state: &State, value: u32) -> Option<Request> {
    let pixel_format = bits(state.bp[0x43], 0, 3);
    let (format, half) = copy_format(value);
    if !matches!(format, 0 | 5) {
        return None;
    }
    let footprint = footprint(state, value)?;
    let (tl, wh) = (state.bp[0x49], state.bp[0x4A]);
    Some(Request {
        params: Params {
            left: bits(tl, 0, 10),
            top: bits(tl, 10, 10),
            half,
            format,
            yuv: u32::from(bits(value, 15, 1) != 0 && bits(value, 16, 1) != 0),
            six: u32::from(pixel_format == 1),
            pad: [0; 2],
        },
        format,
        width: (bits(wh, 0, 10) >> half) + 1,
        height: (bits(wh, 10, 10) >> half) + 1,
        footprint,
    })
}

/// The bytes the encoder would have written for a copy kept on the GPU, from its texels read
/// back (RGBA, `width` by `height`): the texture decoder makes the same texels of them. Texels
/// of its last blocks past the copied rectangle, which the GPU copy doesn't keep, are zero.
pub(crate) fn encode_texels(c: &Copied, texels: &[u8]) -> Vec<(u32, Vec<u8>)> {
    let (lw, lh, block) = blocks(c.format).expect("a format copies make");
    let (bw, bh) = (1u32 << lw, 1u32 << lh);
    let texel = |s: u32, t: u32| -> [u32; 4] {
        if s >= c.width || t >= c.height {
            return [0; 4];
        }
        let i = ((t * c.width + s) * 4) as usize;
        let p = &texels[i..i + 4];
        [p[0], p[1], p[2], p[3]].map(u32::from)
    };
    c.footprint
        .rows()
        .enumerate()
        .map(|(tb, (at, len))| {
            let mut row = vec![0u8; len as usize];
            for (sb, dst) in row.chunks_mut(block as usize).enumerate() {
                for t in 0..bh {
                    for s in 0..bw {
                        let [r, g, b, a] = texel(sb as u32 * bw + s, tb as u32 * bh + t);
                        let i = (t * bw + s) as usize;
                        if c.format == 0 {
                            // I4: what decoded as c4(n) holds n in its top four bits.
                            dst[i / 2] |= if i % 2 == 0 { (r & 0xF0) as u8 } else { (r >> 4) as u8 };
                        } else {
                            // RGB5A3: opaque texels decoded with alpha 255, the rest below it.
                            let v = if a == 255 {
                                0x8000 | (r >> 3) << 10 | (g >> 3) << 5 | b >> 3
                            } else {
                                (a >> 5) << 12 | (r >> 4) << 8 | (g >> 4) << 4 | b >> 4
                            };
                            dst[2 * i..2 * i + 2].copy_from_slice(&(v as u16).to_be_bytes());
                        }
                    }
                }
            }
            (at & 0x01FF_FFFF, row)
        })
        .collect()
}

/// Where copies from the EFB went, with a hash of the memory under each as the copy was made.
/// The game never sees what a copy writes (the renderer keeps it), so memory that later hashes
/// differently was written over by the game, and on hardware the copy would be gone.
#[derive(Default)]
pub(crate) struct Footprints(HashMap<u32, (Footprint, u64)>);

fn hash_footprint(f: &Footprint, mem: &dyn Memory) -> u64 {
    let mut h = 0;
    let mut bytes = Vec::new();
    for (at, len) in f.rows() {
        bytes.resize(len as usize, 0);
        mem.read(at & 0x01FF_FFFF, &mut bytes);
        h = (h ^ hash_bytes(&bytes)).rotate_left(7);
    }
    h
}

impl Footprints {
    /// Notes a copy to `f`, with the memory under it as it stands.
    pub fn record(&mut self, f: Footprint, mem: &dyn Memory) {
        let hash = hash_footprint(&f, mem);
        self.0.insert(f.address, (f, hash));
    }

    /// Forgets the copy at `address` if the game has written its memory since, returning its
    /// span.
    pub fn take_if_overwritten(&mut self, address: u32, mem: &dyn Memory) -> Option<(u32, u32)> {
        let (f, hash) = self.0.get(&address)?;
        if hash_footprint(f, mem) == *hash {
            return None;
        }
        let span = f.span();
        self.0.remove(&address);
        Some(span)
    }

    /// Forgets the copies whose memory the game has written since, returning their spans.
    pub fn take_overwritten(&mut self, mem: &dyn Memory) -> Vec<(u32, u32)> {
        let mut gone = Vec::new();
        self.0.retain(|_, (f, hash)| {
            let same = hash_footprint(f, mem) == *hash;
            if !same {
                gone.push(f.span());
            }
            same
        });
        gone
    }
}

/// A texture a copy made, by the address it copied to.
pub(crate) struct Copied {
    pub id: u32,
    pub format: u32,
    pub width: u32,
    pub height: u32,
    pub footprint: Footprint,
}

pub(crate) struct Copier {
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    params: wgpu::Buffer,
}

impl Copier {
    pub fn new(device: &wgpu::Device, efb: &wgpu::TextureView) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("efb copy"),
            source: wgpu::ShaderSource::Wgsl(include_str!("copy.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("efb copy"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("efb copy"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("efb copy"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(efb),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params.as_entire_binding(),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("efb copy"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("efb copy"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Uint,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            group,
            params,
        }
    }

    /// Makes the copy into `target`, after everything already submitted.
    pub fn copy(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        request: &Request,
    ) {
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&request.params));
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("efb copy"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.group, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
    }
}
