// SPDX-License-Identifier: GPL-3.0-or-later

//! `--window`: plays the game in a window. The main thread owns the window, the GPU, a gamepad
//! (gilrs) with the keyboard, and the audio output (cpal); the game thread renders each frame on
//! the same GPU and paces itself to the video interface's 59.94 fields a second. GameCube
//! controllers on a Wii U adapter (adapter.rs) play their own ports, and the gamepad with the
//! keyboard plays the first port left. With a replay, the replay plays the controllers.
//! SSBM_MUTE=1 leaves the sound out, SSBM_NO_ADAPTER=1 leaves the adapter alone, and every five
//! seconds the rates the game and the window keep go to stderr.

use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::adapter::{self, Adapter};
use ssbm_sdk::{PadStatus, Sdk, hw};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

const FIELD_RATE: f64 = 60_000.0 / 1_001.0;
const AUDIO_RATE: u32 = 32_000;
/// A GameCube drive's average read rate, in bytes a second.
const DISC_RATE: u64 = 3_000_000;
/// Audio queued beyond this many samples is dropped, to keep latency down.
const AUDIO_MAX: usize = AUDIO_RATE as usize / 5;

/// What the two threads share.
pub struct Link {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// The gamepad and the keyboard, as one controller, and the buttons tapped on them since the
    /// game last read it, which it reads as pressed: several polls can come between two reads.
    host: Mutex<(PadStatus, u16)>,
    adapter: OnceLock<Arc<Adapter>>,
    frame: Mutex<Option<ssbm_render::Frame>>,
    /// Stereo samples at 32 kHz, left then right.
    audio: Mutex<VecDeque<[i16; 2]>>,
    quit: AtomicBool,
    /// Wakes the window when a frame is done.
    proxy: OnceLock<winit::event_loop::EventLoopProxy<()>>,
    /// Fields and frames so far, for the rates logged.
    fields: std::sync::atomic::AtomicU64,
    frames: std::sync::atomic::AtomicU64,
}

static LINK: OnceLock<Arc<Link>> = OnceLock::new();

pub fn requested() -> bool {
    std::env::args().any(|a| a == "--window")
}

/// The GPU, on the main thread before the game starts; the window comes once the event loop runs.
pub struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
}

pub fn start() -> Gpu {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .expect("a GPU adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("ssbm-run"),
        required_features: wgpu::Features::DUAL_SOURCE_BLENDING,
        ..Default::default()
    }))
    .expect("a GPU device");
    let link = Arc::new(Link {
        device,
        queue,
        host: Mutex::new((
            PadStatus {
                connected: true,
                ..PadStatus::default()
            },
            0,
        )),
        adapter: OnceLock::new(),
        frame: Mutex::new(None),
        audio: Mutex::new(VecDeque::new()),
        quit: AtomicBool::new(false),
        proxy: OnceLock::new(),
        fields: Default::default(),
        frames: Default::default(),
    });
    LINK.set(link).ok().expect("one window");
    Gpu { instance, adapter }
}

/// The game thread's side: render to the shared GPU, mix sound, read the controllers and keep
/// to real time, each field.
pub fn install(sdk: &Rc<Sdk>, live_input: bool) {
    let link = LINK.get().expect("window::start first").clone();
    // Disc reads at a drive's pace, which the music needs (Hw::set_disc_rate).
    sdk.hw.set_disc_rate(Some(DISC_RATE));
    let mut renderer = ssbm_render::Renderer::with_device(link.device.clone(), link.queue.clone());
    // Frames stay on the GPU, which shows them.
    renderer.xfb_on_gpu = true;
    let frames = link.clone();
    renderer.on_frame(move |f| {
        frames.frames.fetch_add(1, Ordering::Relaxed);
        *frames.frame.lock().unwrap() = Some(f);
        if let Some(proxy) = frames.proxy.get() {
            let _ = proxy.send_event(());
        }
    });
    // On a thread of its own, so the game runs on while it draws.
    sdk.hw
        .set_renderer(Box::new(ssbm_render::Threaded::spawn(renderer)));
    if live_input && std::env::var_os("SSBM_NO_ADAPTER").is_none() {
        let _ = link.adapter.set(adapter::start());
    }
    sdk.dev.mix_audio.set(true);
    let audio = link.clone();
    sdk.hw.set_audio_out(Box::new(move |block| {
        let mut q = audio.audio.lock().unwrap();
        // Big-endian pairs, right first.
        for p in block.chunks_exact(4) {
            q.push_back([
                i16::from_be_bytes([p[2], p[3]]),
                i16::from_be_bytes([p[0], p[1]]),
            ]);
        }
        while q.len() > AUDIO_MAX {
            q.pop_front();
        }
    }));
    let input = live_input.then(Input::default);
    pace(sdk, link, input, Instant::now(), 0);
}

/// Stops any rumble, then exits.
pub fn exit(code: i32) -> ! {
    if let Some(a) = LINK.get().and_then(|l| l.adapter.get()) {
        a.stop();
    }
    std::process::exit(code);
}

fn pace(sdk: &Rc<Sdk>, link: Arc<Link>, mut input: Option<Input>, start: Instant, field: u64) {
    sdk.schedule(hw::field_start(field), move |ctx| {
        let sdk = ctx.ext::<Sdk>();
        if link.quit.load(Ordering::Relaxed) {
            exit(0);
        }
        if let Some(input) = &mut input {
            let pads = input.read(&link, sdk.dev.motors.get());
            *sdk.dev.pads.borrow_mut() = pads;
        }
        link.fields.fetch_add(1, Ordering::Relaxed);
        let due = start + Duration::from_secs_f64(field as f64 / FIELD_RATE);
        let now = Instant::now();
        // Ahead of real time: wait. Far behind: catch up from now rather than rushing.
        let start = if due > now {
            wait_until(due);
            start
        } else if now - due > Duration::from_millis(100) {
            now - Duration::from_secs_f64(field as f64 / FIELD_RATE)
        } else {
            start
        };
        pace(&sdk, link, input, start, field + 1);
    });
}

/// The live controllers, put together each field.
#[derive(Default)]
struct Input {
    last: [PadStatus; 4],
    /// What plays each port (0 nothing, 1 the gamepad and keyboard, 2 the adapter), for the log.
    layout: [u8; 4],
}

impl Input {
    /// The four ports: the adapter's where it has controllers, the gamepad and keyboard on the
    /// first one left. The motors the game wants on go to the adapter.
    fn read(&mut self, link: &Link, motors: [u32; 4]) -> [PadStatus; 4] {
        let adapter = link.adapter.get();
        let mut ports = adapter.map_or([None; 4], |a| a.ports());
        let mut layout = ports.map(|p| if p.is_some() { 2 } else { 0 });
        if let Some(i) = ports.iter().position(Option::is_none) {
            let mut host = link.host.lock().unwrap();
            let (mut pad, taps) = *host;
            host.1 = 0;
            pad.button |= taps;
            ports[i] = Some(pad);
            layout[i] = 1;
        }
        if layout != self.layout {
            let ports: Vec<_> = (0..4)
                .filter_map(|i| match layout[i] {
                    1 => Some(format!("port {} the gamepad and keyboard", i + 1)),
                    2 => Some(format!("port {} the GameCube controller on the adapter", i + 1)),
                    _ => None,
                })
                .collect();
            eprintln!("window: {}", ports.join(", "));
            self.layout = layout;
        }
        if let Some(a) = adapter {
            // PAD_MOTOR_RUMBLE.
            a.rumble(motors.map(|m| m == 1));
        }
        let pads = ports.map(Option::unwrap_or_default);
        // SSBM_TRACE_PADS=1 logs the controllers as they change.
        if std::env::var_os("SSBM_TRACE_PADS").is_some() {
            for (i, (p, last)) in pads.iter().zip(&self.last).enumerate() {
                if p != last {
                    eprintln!(
                        "pad {}: buttons {:04x}, stick {} {}, c-stick {} {}, triggers {} {}{}",
                        i + 1,
                        p.button,
                        p.stick_x,
                        p.stick_y,
                        p.substick_x,
                        p.substick_y,
                        p.trigger_l,
                        p.trigger_r,
                        if p.connected { "" } else { " (none)" }
                    );
                }
            }
        }
        self.last = pads;
        pads
    }
}

/// Waits until `due`: sleeps while far from it (Windows sleeps in steps as long as 15.6 ms), then
/// yields the rest.
fn wait_until(due: Instant) {
    if std::env::var_os("SSBM_UNPACED").is_some() {
        return;
    }
    loop {
        let now = Instant::now();
        if now >= due {
            return;
        }
        if due - now > Duration::from_millis(20) {
            std::thread::sleep(due - now - Duration::from_millis(18));
        } else {
            std::thread::yield_now();
        }
    }
}

/// The main thread's side, until the window closes.
pub fn run(gpu: Gpu) {
    let link = LINK.get().expect("window::start first").clone();
    let _audio = if std::env::var_os("SSBM_MUTE").is_some() {
        None
    } else {
        start_audio(link.clone())
    };
    let event_loop = EventLoop::with_user_event().build().expect("an event loop");
    let _ = link.proxy.set(event_loop.create_proxy());
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App {
        gpu,
        link,
        window: None,
        gilrs: {
            let g = gilrs::Gilrs::new().ok();
            match g.as_ref().and_then(|g| g.gamepads().next()) {
                Some((_, gp)) => eprintln!("window: gamepad {}", gp.name()),
                None => eprintln!("window: no gamepad found, only the keyboard"),
            }
            g
        },
        keys: Default::default(),
        tapped: Default::default(),
        presented: 0,
        since: Instant::now(),
    };
    event_loop
        .run_app(&mut app)
        .expect("the window's event loop");
}

struct Shown {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// The frame shown, when the renderer left it on the GPU.
    frame_group: Option<wgpu::BindGroup>,
}

struct App {
    gpu: Gpu,
    link: Arc<Link>,
    window: Option<Shown>,
    gilrs: Option<gilrs::Gilrs>,
    keys: std::collections::HashSet<KeyCode>,
    /// Keys pressed since the last poll, held or not, so a tap between polls still counts.
    tapped: std::collections::HashSet<KeyCode>,
    presented: u64,
    since: Instant,
}

const BLIT: &str = r#"
@group(0) @binding(0) var frame: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;

struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: Out;
    o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(p.x, 1.0 - p.y);
    return o;
}

@fragment
fn fs(i: Out) -> @location(0) vec4<f32> {
    return textureSample(frame, smp, i.uv);
}
"#;

impl App {
    fn show(&mut self, event_loop: &ActiveEventLoop) -> Shown {
        let attrs = Window::default_attributes()
            .with_title("ssbm-rs")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 960.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("a window"));
        let surface = self
            .gpu
            .instance
            .create_surface(window.clone())
            .expect("a surface");
        let size = window.inner_size();
        let caps = surface.get_capabilities(&self.gpu.adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        let device = &self.link.device;
        surface.configure(device, &config);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("xfb"),
            size: wgpu::Extent3d {
                width: 640,
                height: 480,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT.into()),
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let view = texture.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Shown {
            window,
            surface,
            config,
            texture,
            bind_group,
            pipeline,
            layout,
            sampler,
            frame_group: None,
        }
    }

    /// Presents the newest frame, letterboxed to 4:3.
    fn draw(&mut self) {
        let elapsed = self.since.elapsed();
        if elapsed >= Duration::from_secs(5) {
            let s = elapsed.as_secs_f64();
            let fields = self.link.fields.swap(0, Ordering::Relaxed);
            let frames = self.link.frames.swap(0, Ordering::Relaxed);
            eprintln!(
                "window: {:.2} fields/s, {:.2} game frames/s, {:.1} presents/s",
                fields as f64 / s,
                frames as f64 / s,
                self.presented as f64 / s
            );
            self.presented = 0;
            self.since = Instant::now();
        }
        let Some(shown) = &mut self.window else {
            return;
        };
        let device = &self.link.device;
        let queue = &self.link.queue;
        let next = self.link.frame.lock().unwrap().take();
        if let Some(texture) = next.as_ref().and_then(|f| f.texture.as_ref()) {
            let view = texture.create_view(&Default::default());
            shown.frame_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &shown.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&shown.sampler),
                    },
                ],
            }));
        } else if let Some(f) = next {
            shown.frame_group = None;
            let (w, h) = (f.width.min(640), f.height.min(480));
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &shown.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &f.rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(f.width * 4),
                    rows_per_image: Some(f.height),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
        }
        let frame = match shown.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => {
                shown.surface.configure(device, &shown.config);
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("present"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            let (sw, sh) = (shown.config.width as f32, shown.config.height as f32);
            let scale = (sw / 4.0).min(sh / 3.0);
            let (w, h) = (4.0 * scale, 3.0 * scale);
            pass.set_viewport((sw - w) / 2.0, (sh - h) / 2.0, w, h, 0.0, 1.0);
            pass.set_pipeline(&shown.pipeline);
            pass.set_bind_group(0, shown.frame_group.as_ref().unwrap_or(&shown.bind_group), &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
        queue.present(frame);
        self.presented += 1;
    }

    /// The gamepad and the keyboard, as one controller.
    fn poll_input(&mut self) {
        let mut taps = 0;
        let mut pad = PadStatus {
            connected: true,
            ..PadStatus::default()
        };
        if let Some(g) = &mut self.gilrs {
            // Buttons pressed since the last poll count as held for it, so a tap isn't lost.
            let mut tapped = Vec::new();
            while let Some(e) = g.next_event() {
                if let gilrs::EventType::ButtonPressed(b, _) = e.event {
                    tapped.push(b);
                }
            }
            if let Some((_, gp)) = g.gamepads().next() {
                use gilrs::{Axis, Button};
                let bits = [
                    (Button::South, 0x0100),        // A
                    (Button::West, 0x0200),         // B
                    (Button::East, 0x0400),         // X
                    (Button::North, 0x0800),        // Y
                    (Button::RightTrigger, 0x0010), // Z
                    (Button::Start, 0x1000),
                    (Button::DPadLeft, 0x0001),
                    (Button::DPadRight, 0x0002),
                    (Button::DPadDown, 0x0004),
                    (Button::DPadUp, 0x0008),
                ];
                for (b, bit) in bits {
                    if gp.is_pressed(b) || tapped.contains(&b) {
                        pad.button |= bit;
                    }
                    if tapped.contains(&b) {
                        taps |= bit;
                    }
                }
                let axis = |a: Axis| (gp.value(a).clamp(-1.0, 1.0) * 127.0) as i8;
                pad.stick_x = axis(Axis::LeftStickX);
                pad.stick_y = axis(Axis::LeftStickY);
                pad.substick_x = axis(Axis::RightStickX);
                pad.substick_y = axis(Axis::RightStickY);
                let trigger = |b: Button| gp.button_data(b).map_or(0.0, |d| d.value());
                let (l, r) = (
                    trigger(Button::LeftTrigger2),
                    trigger(Button::RightTrigger2),
                );
                pad.trigger_l = (l * 255.0) as u8;
                pad.trigger_r = (r * 255.0) as u8;
                if l > 0.95 {
                    pad.button |= 0x0040;
                }
                if r > 0.95 {
                    pad.button |= 0x0020;
                }
            }
        }
        // The keyboard adds to the gamepad: its buttons, and its directions where it has any.
        {
            let k = |c: KeyCode| self.keys.contains(&c) || self.tapped.contains(&c);
            let bits = [
                (KeyCode::KeyX, 0x0100),
                (KeyCode::KeyZ, 0x0200),
                (KeyCode::KeyC, 0x0400),
                (KeyCode::KeyS, 0x0800),
                (KeyCode::KeyD, 0x0010),
                (KeyCode::Enter, 0x1000),
                (KeyCode::KeyQ, 0x0040),
                (KeyCode::KeyW, 0x0020),
            ];
            for (c, bit) in bits {
                if k(c) {
                    pad.button |= bit;
                }
                if self.tapped.contains(&c) {
                    taps |= bit;
                }
            }
            let dir = |neg: KeyCode, pos: KeyCode| -> i8 {
                match (k(neg), k(pos)) {
                    (true, false) => -127,
                    (false, true) => 127,
                    _ => 0,
                }
            };
            let set = |axis: &mut i8, v: i8| {
                if v != 0 {
                    *axis = v;
                }
            };
            set(
                &mut pad.stick_x,
                dir(KeyCode::ArrowLeft, KeyCode::ArrowRight),
            );
            set(&mut pad.stick_y, dir(KeyCode::ArrowDown, KeyCode::ArrowUp));
            set(&mut pad.substick_x, dir(KeyCode::KeyJ, KeyCode::KeyL));
            set(&mut pad.substick_y, dir(KeyCode::KeyK, KeyCode::KeyI));
            if pad.button & 0x0040 != 0 {
                pad.trigger_l = 255;
            }
            if pad.button & 0x0020 != 0 {
                pad.trigger_r = 255;
            }
        }
        self.tapped.clear();
        let mut host = self.link.host.lock().unwrap();
        host.0 = pad;
        host.1 |= taps;
    }
}

impl ApplicationHandler<()> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let shown = self.show(event_loop);
            self.window = Some(shown);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if std::env::var_os("SSBM_TRACE_EVENTS").is_some()
            && !matches!(event, WindowEvent::RedrawRequested)
        {
            eprintln!("window event: {event:?}");
        }
        match event {
            WindowEvent::CloseRequested => {
                self.link.quit.store(true, Ordering::Relaxed);
                event_loop.exit();
                exit(0);
            }
            WindowEvent::Resized(size) => {
                if let Some(shown) = &mut self.window {
                    shown.config.width = size.width.max(1);
                    shown.config.height = size.height.max(1);
                    shown.surface.configure(&self.link.device, &shown.config);
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(code),
                        state,
                        ..
                    },
                ..
            } => {
                if state == ElementState::Pressed {
                    self.keys.insert(code);
                    self.tapped.insert(code);
                } else {
                    self.keys.remove(&code);
                }
            }
            WindowEvent::RedrawRequested => self.draw(),
            _ => {}
        }
    }

    /// A frame is done: read the controllers for the next and show it.
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _: ()) {
        self.poll_input();
        if let Some(shown) = &self.window {
            shown.window.request_redraw();
        }
    }
}

/// The audio output: the default device, fed from the queue the game fills, resampled from
/// 32 kHz.
fn start_audio(link: Arc<Link>) -> Option<cpal::Stream> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let device = cpal::default_host().default_output_device()?;
    let config = device.default_output_config().ok()?;
    let rate = config.sample_rate() as f64;
    let channels = usize::from(config.channels());
    let step = f64::from(AUDIO_RATE) / rate;
    let mut pos = 0.0f64;
    let mut last = [0i16; 2];
    let stream = device
        .build_output_stream(
            config.config(),
            move |out: &mut [f32], _| {
                let mut q = link.audio.lock().unwrap();
                for frame in out.chunks_mut(channels) {
                    pos += step;
                    while pos >= 1.0 {
                        if let Some(s) = q.pop_front() {
                            last = s;
                        }
                        pos -= 1.0;
                    }
                    for (c, o) in frame.iter_mut().enumerate() {
                        *o = f32::from(last[c.min(1)]) / 32768.0;
                    }
                }
            },
            |e| eprintln!("audio: {e}"),
            None,
        )
        .ok()?;
    stream.play().ok()?;
    Some(stream)
}
