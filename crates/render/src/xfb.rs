// SPDX-License-Identifier: GPL-3.0-or-later
// Follows Dolphin's software renderer's EncodeXFB and its XFB decoder (GPL-2.0-or-later).

//! Copies to the XFB: the EFB through the copy filter (anti-aliasing and deflicker), gamma, and
//! YUV 4:2:2, then back to RGB as the video interface shows it.

use rayon::prelude::*;
use ssbm_gx::State;

use crate::{EFB_WIDTH, Frame, bits};

#[derive(Clone, Copy, Default)]
struct Yuv {
    y: u8,
    u: i8,
    v: i8,
}

fn to_yuv(r: u8, g: u8, b: u8) -> Yuv {
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    let y = (66 * r + 129 * g + 25 * b) as u16;
    let u = (-38 * r - 74 * g + 112 * b) as i16;
    let v = (112 * r - 94 * g - 18 * b) as i16;
    Yuv {
        y: ((y >> 8) + ((y >> 7) & 1)) as u8,
        u: ((u >> 8) + ((u >> 7) & 1)) as i8,
        v: ((v >> 8) + ((v >> 7) & 1)) as i8,
    }
}

/// Copies `rect` (left, top, right, bottom) of the EFB, read back as RGBA rows of 640, to the
/// XFB as BP 0x52 value `value` asks, and shows it.
pub(crate) fn copy(state: &State, value: u32, efb: &[u8], rect: [u32; 4]) -> Frame {
    let [left, top, right, bottom] = rect;
    let format = bits(state.bp[0x43], 0, 3);
    let clamp_top = bits(value, 0, 1) != 0;
    let clamp_bottom = bits(value, 1, 1) != 0;
    let gamma = [1.0f32, 1.7, 2.2, 1.0][bits(value, 7, 2) as usize];
    let gamma_rcp = 1.0 / gamma;
    let lo = state.bp[0x53];
    let hi = state.bp[0x54];
    let w = [
        bits(lo, 0, 6),
        bits(lo, 6, 6),
        bits(lo, 12, 6),
        bits(lo, 18, 6),
        bits(hi, 0, 6),
        bits(hi, 6, 6),
        bits(hi, 12, 6),
    ];
    let (w_prev, w_mid, w_next) = (w[0] + w[1], w[2] + w[3] + w[4], w[5] + w[6]);
    let lut: Vec<u8> = (0..256)
        .map(|c| ((c as f32 / 255.0).powf(gamma_rcp) * 255.0).clamp(0.0, 255.0) as u8)
        .collect();
    let color = |x: u32, y: u32| -> [u32; 3] {
        let at = ((y * EFB_WIDTH + x) * 4) as usize;
        let p = &efb[at..at + 3];
        // Formats without alpha read their color as is; RGBA6's is already six bits a channel.
        let _ = format;
        [u32::from(p[0]), u32::from(p[1]), u32::from(p[2])]
    };
    let width = right - left;
    let height = bottom - top;
    // A row of YUYV pairs (Y, U or V) per pixel, from the EFB's rows about it.
    let encode = |y: u32, out: &mut [(u8, u8)], scanline: &mut [Yuv]| {
        let y_prev = (y as i32 - 1).max(if clamp_top { top as i32 } else { 0 }) as u32;
        let y_next = (y + 1).min(
            if clamp_bottom {
                bottom
            } else {
                crate::EFB_HEIGHT
            } - 1,
        );
        for (i, x) in (left..right).enumerate() {
            let (p, c, n) = (color(x, y_prev), color(x, y), color(x, y_next));
            let mut rgb = [0u8; 3];
            for k in 0..3 {
                let sum = p[k] * w_prev + c[k] * w_mid + n[k] * w_next;
                rgb[k] = lut[(sum >> 6).min(255) as usize];
            }
            scanline[i + 1] = to_yuv(rgb[0], rgb[1], rgb[2]);
        }
        scanline[0] = scanline[1];
        scanline[(right + 1) as usize] = scanline[right as usize];
        let mut i = 1usize;
        let mut x = 0usize;
        while x + 1 < width as usize + 1 && i + 1 < scanline.len() {
            let (a, b, c) = (scanline[i - 1], scanline[i], scanline[i + 1]);
            let u = 128i32 + ((i32::from(a.u) + (i32::from(b.u) << 1) + i32::from(c.u)) >> 2);
            let v = 128i32 + ((i32::from(a.v) + (i32::from(b.v) << 1) + i32::from(c.v)) >> 2);
            if x < width as usize {
                out[x] = (b.y.wrapping_add(16), u as u8);
            }
            if x + 1 < width as usize {
                out[x + 1] = (c.y.wrapping_add(16), v as u8);
            }
            i += 2;
            x += 2;
        }
    };
    // Rows are independent: rayon's threads share them.
    let mut yuyv = vec![(0u8, 0u8); (width * height) as usize];
    yuyv.par_chunks_mut(width as usize)
        .enumerate()
        .for_each_init(
            || vec![Yuv::default(); (EFB_WIDTH + 2) as usize],
            |scanline, (r, out)| encode(top + r as u32, out, scanline),
        );
    let y_scale = if bits(value, 10, 1) != 0 {
        256.0 / bits(state.bp[0x4E], 0, 9) as f32
    } else {
        bits(state.bp[0x4E], 0, 9) as f32 / 256.0
    };
    let out_height = ((height as f32) * y_scale) as u32;
    // Back to RGB as the video interface shows it.
    let decode = |oy: u32, out: &mut [u8]| {
        let sy = ((oy as f32 / y_scale) as u32).min(height - 1);
        let row = (sy * width) as usize;
        let mut x = 0;
        while x < width as usize {
            let (y1, u) = yuyv[row + x];
            let (y2, v) = if x + 1 < width as usize {
                yuyv[row + x + 1]
            } else {
                (y1, 128)
            };
            let (y1, y2) = (i32::from(y1) - 16, i32::from(y2) - 16);
            let (u, v) = (i32::from(u) - 128, i32::from(v) - 128);
            for (k, yy) in [y1, y2]
                .iter()
                .take((width as usize - x).min(2))
                .enumerate()
            {
                let y = *yy as f32;
                let r = ((1.164 * y + 1.596 * v as f32) as i32).clamp(0, 255);
                let g = ((1.164 * y - 0.392 * u as f32 - 0.813 * v as f32) as i32).clamp(0, 255);
                let b = ((1.164 * y + 2.017 * u as f32) as i32).clamp(0, 255);
                out[(x + k) * 4..(x + k) * 4 + 4]
                    .copy_from_slice(&[r as u8, g as u8, b as u8, 255]);
            }
            x += 2;
        }
    };
    let mut rgba = vec![0u8; (width * out_height * 4) as usize];
    rgba.par_chunks_mut(width as usize * 4)
        .enumerate()
        .for_each(|(r, out)| decode(r as u32, out));
    Frame {
        width,
        height: out_height,
        rgba,
        texture: None,
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct EncodeParams {
    left: u32,
    top: u32,
    first_row: u32,
    last_row: u32,
    width: u32,
    w_prev: u32,
    w_mid: u32,
    w_next: u32,
    lut: [[u32; 4]; 64],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DecodeParams {
    width: u32,
    pad: [u32; 3],
    y: [[f32; 4]; 64],
    rv: [[f32; 4]; 64],
    gv: [[f32; 4]; 64],
    bu: [[f32; 4]; 64],
}

impl DecodeParams {
    /// The products `decode` in `copy` makes, by luma (biased by 16) or chroma (by 128).
    fn new(width: u32) -> Self {
        let table = |k: f32, bias: i32| {
            let mut t = [[0f32; 4]; 64];
            for i in 0..256 {
                t[i / 4][i % 4] = k * (i as i32 - bias) as f32;
            }
            t
        };
        Self {
            width,
            pad: [0; 3],
            y: table(1.164, 16),
            rv: table(1.596, 128),
            gv: table(0.813, 128),
            bu: table(2.017, 128),
        }
    }
}

/// Copies to the XFB on the GPU (xfb.wgsl), leaving the frame in a texture, as `copy` makes it.
pub(crate) struct GpuXfb {
    encode: wgpu::RenderPipeline,
    decode: wgpu::RenderPipeline,
    decode_layout: wgpu::BindGroupLayout,
    encode_params: wgpu::Buffer,
    decode_params: wgpu::Buffer,
    encode_group: wgpu::BindGroup,
    /// Green's luma term less its U term, as `copy` rounds them, by luma * 256 + U.
    g_yu: wgpu::Buffer,
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn storage_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    entry: &str,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("xfb"),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("xfb"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    })
}

impl GpuXfb {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, efb: &wgpu::TextureView) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xfb"),
            source: wgpu::ShaderSource::Wgsl(include_str!("xfb.wgsl").into()),
        });
        let encode_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("xfb encode"),
            entries: &[
                texture_entry(wgpu::TextureSampleType::Float { filterable: false }),
                uniform_entry(1),
            ],
        });
        let decode_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("xfb decode"),
            entries: &[
                texture_entry(wgpu::TextureSampleType::Uint),
                uniform_entry(1),
                storage_entry(2),
                storage_entry(3),
            ],
        });
        let g_yu: Vec<f32> = (0..256 * 256)
            .map(|i| 1.164 * ((i >> 8) - 16) as f32 - 0.392 * ((i & 255) - 128) as f32)
            .collect();
        let g_yu_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("xfb green"),
            size: (g_yu.len() * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&g_yu_buffer, 0, bytemuck::cast_slice(&g_yu));
        let buffer = |size: usize| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("xfb"),
                size: size as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let encode_params = buffer(std::mem::size_of::<EncodeParams>());
        let decode_params = buffer(std::mem::size_of::<DecodeParams>());
        let encode_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xfb encode"),
            layout: &encode_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(efb),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: encode_params.as_entire_binding(),
                },
            ],
        });
        Self {
            encode: pipeline(
                device,
                &module,
                &encode_layout,
                "encode",
                wgpu::TextureFormat::Rg8Uint,
            ),
            decode: pipeline(
                device,
                &module,
                &decode_layout,
                "decode",
                wgpu::TextureFormat::Rgba8Unorm,
            ),
            decode_layout,
            encode_params,
            decode_params,
            encode_group,
            g_yu: g_yu_buffer,
        }
    }

    /// Copies `rect` (left, top, right, bottom) of the EFB to the XFB as BP 0x52 value `value`
    /// asks, after everything already submitted, and returns the frame shown with its size.
    pub fn copy(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        state: &State,
        value: u32,
        rect: [u32; 4],
    ) -> (wgpu::Texture, u32, u32) {
        let [left, top, right, bottom] = rect;
        let (width, height) = (right - left, bottom - top);
        let gamma_rcp = 1.0 / [1.0f32, 1.7, 2.2, 1.0][bits(value, 7, 2) as usize];
        let (lo, hi) = (state.bp[0x53], state.bp[0x54]);
        let w = [
            bits(lo, 0, 6),
            bits(lo, 6, 6),
            bits(lo, 12, 6),
            bits(lo, 18, 6),
            bits(hi, 0, 6),
            bits(hi, 6, 6),
            bits(hi, 12, 6),
        ];
        let mut lut = [[0u32; 4]; 64];
        for c in 0..256 {
            lut[c / 4][c % 4] =
                u32::from(((c as f32 / 255.0).powf(gamma_rcp) * 255.0).clamp(0.0, 255.0) as u8);
        }
        let encode = EncodeParams {
            left,
            top,
            first_row: if bits(value, 0, 1) != 0 { top } else { 0 },
            last_row: if bits(value, 1, 1) != 0 {
                bottom
            } else {
                crate::EFB_HEIGHT
            } - 1,
            width,
            w_prev: w[0] + w[1],
            w_mid: w[2] + w[3] + w[4],
            w_next: w[5] + w[6],
            lut,
        };
        let y_scale = if bits(value, 10, 1) != 0 {
            256.0 / bits(state.bp[0x4E], 0, 9) as f32
        } else {
            bits(state.bp[0x4E], 0, 9) as f32 / 256.0
        };
        let out_height = ((height as f32) * y_scale) as u32;
        let rows: Vec<u32> = (0..out_height)
            .map(|oy| ((oy as f32 / y_scale) as u32).min(height - 1))
            .collect();
        let texture = |label, w: u32, h: u32, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let yuyv = texture(
            "xfb yuyv",
            width,
            height,
            wgpu::TextureFormat::Rg8Uint,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let shown = texture(
            "xfb",
            width,
            out_height,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let rows_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("xfb rows"),
            size: (rows.len().max(1) * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&rows_buffer, 0, bytemuck::cast_slice(&rows));
        queue.write_buffer(&self.encode_params, 0, bytemuck::bytes_of(&encode));
        queue.write_buffer(
            &self.decode_params,
            0,
            bytemuck::bytes_of(&DecodeParams::new(width)),
        );
        let yuyv_view = yuyv.create_view(&Default::default());
        let shown_view = shown.create_view(&Default::default());
        let decode_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xfb decode"),
            layout: &self.decode_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&yuyv_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.decode_params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: rows_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.g_yu.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        for (view, pipeline, group) in [
            (&yuyv_view, &self.encode, &self.encode_group),
            (&shown_view, &self.decode, &decode_group),
        ] {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("xfb"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
        (shown, width, out_height)
    }
}
