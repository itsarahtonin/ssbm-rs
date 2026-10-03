// SPDX-License-Identifier: GPL-3.0-or-later

//! Copies from the EFB to textures kept on the GPU (copy.wgsl), for the formats Melee copies and
//! samples back as it copied them: its shadows (I4) and a few effects (RGB5A3). The texture holds
//! what the texture decoder would make of what the copy encoder writes, so frames come out as
//! from a copy read back and encoded into memory, without waiting on the GPU for it.

use bytemuck::{Pod, Zeroable};
use ssbm_gx::State;

use crate::bits;

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
    pub address: u32,
    pub format: u32,
    /// The texture's size: the copied rectangle, rounded up to whole blocks as encoded.
    pub width: u32,
    pub height: u32,
    /// The memory the encoder would write: rows of blocks, `stride` apart.
    pub stride: u32,
    pub row_bytes: u32,
    pub rows: u32,
}

/// The copy BP 0x52 value `value` asks for, if it is one kept on the GPU.
pub(crate) fn request(state: &State, value: u32) -> Option<Request> {
    let pixel_format = bits(state.bp[0x43], 0, 3);
    let tp = bits(value, 3, 4);
    let format = tp / 2 + (tp & 1) * 8;
    if pixel_format > 1 || !matches!(format, 0 | 5) {
        return None;
    }
    let half = bits(value, 9, 1);
    let (tl, wh) = (state.bp[0x49], state.bp[0x4A]);
    let (width, height) = (bits(wh, 0, 10) >> half, bits(wh, 10, 10) >> half);
    // Blocks of 8x8 texels for I4, 4x4 for RGB5A3; 32 bytes either way.
    let (lw, lh) = if format == 0 { (3, 3) } else { (2, 2) };
    let (s_blocks, t_blocks) = ((width >> lw) + 1, (height >> lh) + 1);
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
        address: (state.bp[0x4B] & 0x00FF_FFFF) << 5,
        format,
        width: s_blocks << lw,
        height: t_blocks << lh,
        stride: bits(state.bp[0x4D], 0, 10) << 5,
        row_bytes: s_blocks * 32,
        rows: t_blocks,
    })
}

/// A texture a copy made, by the address it copied to.
pub(crate) struct Copied {
    pub id: u32,
    pub format: u32,
    pub width: u32,
    pub height: u32,
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
