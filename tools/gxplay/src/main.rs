// SPDX-License-Identifier: GPL-3.0-or-later

//! Plays a Dolphin FIFO log (a run's GX_DFF) through the renderer, as Dolphin's FIFO player
//! does, and saves each frame (copy to the XFB) as a PNG, to compare with Dolphin's software
//! renderer's frames of the same log (tools/gx/dolphin-render.sh).
//!
//!   gxplay LOG.dff OUT
//!   gxplay diff OURS REF [DIFFS]   compares OURS/frame_N.png with REF/framedump_N.png
//!
//! For debugging: GXPLAY_TRACE prints the first frame's draws and their state,
//! GXPLAY_SILHOUETTES draws every draw flat and additive, GXPLAY_OPAQUE draws without
//! blending, and GXPLAY_DRAWS=N draws only a frame's first N draws.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use ssbm_gx::dff::File;
use ssbm_gx::{Draw, Memory, Produced, Sink, State};
use ssbm_render::{Frame, Renderer};

/// Main memory as the log sets it.
const MEM_SIZE: usize = 0x0180_0000;

struct Player {
    ram: RefCell<Vec<u8>>,
    renderer: RefCell<Renderer>,
}

impl Memory for Player {
    fn read(&self, phys: u32, out: &mut [u8]) {
        let ram = self.ram.borrow();
        let start = (phys & 0x01FF_FFFF) as usize;
        for (i, b) in out.iter_mut().enumerate() {
            *b = ram.get(start + i).copied().unwrap_or(0);
        }
    }

    fn draws(&self) -> bool {
        true
    }

    fn draw(&self, state: &State, draw: &Draw<'_>) {
        self.renderer.borrow_mut().draw(state, draw, self);
    }

    fn copy(&self, state: &State, value: u32) {
        self.renderer.borrow_mut().copy(state, value, self);
    }
}

fn save_png(path: &Path, frame: &Frame) -> std::io::Result<()> {
    frame.save_png(path)
}

fn load_png(path: &Path) -> Option<(u32, u32, Vec<u8>)> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let channels = info.color_type.samples();
    let rgb = buf[..info.buffer_size()]
        .chunks_exact(channels)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    Some((info.width, info.height, rgb))
}

/// Compares frames: per frame, PSNR and the share of pixels off by more than 8 and 32 levels
/// in some channel; with DIFFS, an image of where they differ.
fn diff(ours: &Path, reference: &Path, diffs: Option<&Path>) {
    if let Some(d) = diffs {
        std::fs::create_dir_all(d).expect("making the diff directory");
    }
    println!("frame psnr off8% off32%");
    for n in 1.. {
        let (Some(a), Some(b)) = (
            load_png(&ours.join(format!("frame_{n}.png"))),
            load_png(&reference.join(format!("framedump_{n}.png"))),
        ) else {
            break;
        };
        if (a.0, a.1) != (b.0, b.1) {
            println!("{n} size {}x{} vs {}x{}", a.0, a.1, b.0, b.1);
            continue;
        }
        let (mut se, mut off8, mut off32) = (0f64, 0u64, 0u64);
        let mut img = Vec::with_capacity(a.2.len() / 3 * 4);
        for (p, q) in a.2.chunks_exact(3).zip(b.2.chunks_exact(3)) {
            let d: Vec<i32> = (0..3).map(|k| i32::from(p[k]) - i32::from(q[k])).collect();
            let m = d.iter().map(|v| v.abs()).max().unwrap();
            se += d.iter().map(|v| f64::from(v * v)).sum::<f64>();
            off8 += u64::from(m > 8);
            off32 += u64::from(m > 32);
            let v = (m * 4).min(255) as u8;
            img.extend_from_slice(&[v, v, v, 255]);
        }
        let pixels = (a.0 * a.1) as f64;
        let mse = se / (pixels * 3.0);
        let psnr = if mse == 0.0 {
            f64::INFINITY
        } else {
            10.0 * (255.0f64 * 255.0 / mse).log10()
        };
        println!(
            "{n} {psnr:.2} {:.3} {:.3}",
            100.0 * off8 as f64 / pixels,
            100.0 * off32 as f64 / pixels
        );
        if let Some(d) = diffs {
            save_png(
                &d.join(format!("diff_{n}.png")),
                &Frame {
                    width: a.0,
                    height: a.1,
                    rgba: img,
                },
            )
            .expect("saving a diff");
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 4 && args[1] == "diff" {
        diff(
            Path::new(&args[2]),
            Path::new(&args[3]),
            args.get(4).map(Path::new),
        );
        return;
    }
    if args.len() < 3 {
        eprintln!("usage: gxplay LOG.dff OUT");
        std::process::exit(2);
    }
    let log =
        File::read(&std::fs::read(&args[1]).expect("reading the log")).expect("parsing the log");
    let out = PathBuf::from(&args[2]);
    std::fs::create_dir_all(&out).expect("making the output directory");
    let player = Player {
        ram: RefCell::new(vec![0; MEM_SIZE]),
        renderer: RefCell::new(Renderer::new().expect("a GPU")),
    };
    player.renderer.borrow_mut().trace = std::env::var_os("GXPLAY_TRACE").is_some();
    player.renderer.borrow_mut().silhouettes = std::env::var_os("GXPLAY_SILHOUETTES").is_some();
    player.renderer.borrow_mut().opaque = std::env::var_os("GXPLAY_OPAQUE").is_some();
    player.renderer.borrow_mut().draw_limit = std::env::var("GXPLAY_DRAWS")
        .ok()
        .map(|n| n.parse().expect("GXPLAY_DRAWS=N"));
    let mut state = State::load(&log.bp, &log.cp, &log.xf, &log.tmem);
    player.renderer.borrow_mut().clear_efb(&state);
    let mut produced: Vec<Produced> = Vec::new();
    let mut n = 0;
    let start = std::time::Instant::now();
    for frame in &log.frames {
        let mut at = 0usize;
        let mut updates: Vec<_> = frame.updates.iter().collect();
        updates.sort_by_key(|u| u.position);
        for u in updates {
            let pos = (u.position as usize).min(frame.fifo.len());
            if pos > at {
                state.feed(&player, &frame.fifo[at..pos], &mut produced, false);
                at = pos;
            }
            let mut ram = player.ram.borrow_mut();
            let a = (u.address & 0x01FF_FFFF) as usize;
            let end = (a + u.data.len()).min(MEM_SIZE);
            if a < end {
                ram[a..end].copy_from_slice(&u.data[..end - a]);
            }
        }
        state.feed(&player, &frame.fifo[at..], &mut produced, false);
        produced.clear();
        for f in player.renderer.borrow_mut().take_frames() {
            n += 1;
            save_png(&out.join(format!("frame_{n}.png")), &f).expect("saving a frame");
        }
    }
    let r = player.renderer.borrow();
    eprintln!("{n} frames in {:.2?}", start.elapsed());
    let mut notes: Vec<_> = r.unsupported.iter().collect();
    notes.sort();
    for (what, count) in notes {
        eprintln!("not drawn exactly: {what} ({count})");
    }
}
