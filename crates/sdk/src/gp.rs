// SPDX-License-Identifier: GPL-3.0-or-later

//! The graphics processor: the command stream runs through the GPU's state (ssbm-gx), which
//! reports the commands that signal the CPU. A recorder can log each frame's draws and copies,
//! digested by what they use, to compare two runs' streams.

use std::cell::RefCell;
use std::io::Write;
use std::path::PathBuf;

use ssbm_gx::{Draw, Event, Produced, Sink, State};

use ssbm_gx::dff::{self, Log};
use ssbm_rt::Ctx;

/// A command with an effect outside the GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    /// `PE_DONE`: the frame is drawn.
    Finish,
    /// `PE_TOKEN` or `PE_TOKEN_INT`.
    Token { token: u16, interrupt: bool },
}

/// Main memory as the GPU reads it, by physical address. A FIFO log being recorded takes each
/// command, notes what a command read at the point in the frame where it starts, and ends a
/// frame after its copy to the XFB.
struct GpuMemory<'a> {
    ctx: &'a Ctx,
    log: Option<&'a RefCell<Log>>,
    sink: Option<&'a RefCell<Box<dyn Sink>>>,
    /// The game's FIFO ring.
    fifo: (u32, u32),
}

impl ssbm_gx::Memory for GpuMemory<'_> {
    fn read(&self, phys: u32, out: &mut [u8]) {
        let phys = phys & 0x01FF_FFFF;
        if self.ctx.mem.read_bytes(0x8000_0000 | phys, out).is_err() {
            out.fill(0);
        }
        if let Some(log) = self.log {
            let mut log = log.borrow_mut();
            let position = log.position();
            log.read(phys, out, dff::VERTEX_STREAM, position);
        }
    }

    fn draws(&self) -> bool {
        self.sink.is_some()
    }

    fn draw(&self, state: &State, draw: &Draw<'_>) {
        if let Some(sink) = self.sink {
            sink.borrow_mut().draw(state, draw, self);
        }
    }

    fn copy(&self, state: &State, value: u32) {
        if let Some(sink) = self.sink {
            let mut sink = sink.borrow_mut();
            sink.copy(state, value, self);
            // Memory as the copy left it, as the GPU writes it on the console. Without a
            // renderer, nothing draws what a copy would hold, and memory stays as it was.
            for w in sink.take_written() {
                let at = 0x8000_0000 | (w.address & 0x01FF_FFFF);
                let mut bytes = w.bytes;
                if let Some(before) = &w.before {
                    let mut now = vec![0; bytes.len()];
                    if self.ctx.mem.read_bytes(at, &mut now).is_err() || before.len() != now.len() {
                        continue;
                    }
                    for ((b, n), o) in bytes.iter_mut().zip(&now).zip(before) {
                        if n != o {
                            *b = *n;
                        }
                    }
                }
                let _ = self.ctx.dma_write(at, &bytes);
            }
        }
    }

    fn command(&self, bytes: &[u8]) {
        if let Some(log) = self.log {
            let mut log = log.borrow_mut();
            log.fifo(bytes);
            if bytes[0] == 0x61
                && ssbm_gx::is_xfb_copy(u32::from_be_bytes([
                    bytes[1], bytes[2], bytes[3], bytes[4],
                ]))
            {
                log.end_frame(self.fifo.0, self.fifo.1);
            }
        }
    }
}

/// A FIFO log to record: `count` frames from frame `start`, saved to `path`.
struct DffPlan {
    path: PathBuf,
    start: u64,
    count: usize,
}

pub(crate) struct Gp {
    state: State,
    pub draws: u64,
    finishes: u64,
    record: Option<Record>,
    dff: Option<DffPlan>,
    dff_log: Option<RefCell<Log>>,
    /// Copies to the XFB so far: the frames a FIFO log counts.
    xfb_copies: u64,
    /// The game's FIFO ring, which the FIFO player writes a frame's commands through.
    fifo: (u32, u32),
    /// What draws the stream.
    sink: Option<RefCell<Box<dyn Sink>>>,
}

impl Default for Gp {
    fn default() -> Self {
        Gp {
            state: State::new(),
            draws: 0,
            finishes: 0,
            record: None,
            dff: None,
            dff_log: None,
            xfb_copies: 0,
            fifo: (0, 0),
            sink: None,
        }
    }
}

/// Logs the stream frame by frame: per frame drawn (each `PE_DONE`), a line with its draws and
/// copies and a hash of their digests; and every draw and copy of one frame with its digest by
/// part, for comparing two runs draw by draw.
struct Record {
    frames: Box<dyn Write>,
    detail: Option<(u64, Box<dyn Write>)>,
    frame: u64,
    events: u64,
    draws: u64,
    hash: u64,
}

const FNV_OFFSET: u64 = 0xCBF2_9CE4_8422_2325;

impl Record {
    fn event(&mut self, p: &Produced) {
        let Some(d) = p.digest else { return };
        self.events += 1;
        self.hash = ssbm_gx::fnv(self.hash, &d.total().to_le_bytes());
        let line = match &p.event {
            Event::Draw {
                primitive,
                vat,
                count,
            } => {
                self.draws += 1;
                format!("DRAW {primitive:02X} vat {vat} count {count}")
            }
            Event::Copy(v) => format!("COPY {v:06X}"),
            _ => return,
        };
        if let Some((frame, out)) = &mut self.detail
            && *frame == self.frame
        {
            let _ = writeln!(
                out,
                "{line} bp {:016x} cp {:016x} xf {:016x} tev {:016x} vertices {:016x} textures {:016x}",
                d.bp, d.cp, d.xf, d.tev, d.vertices, d.textures
            );
        }
    }

    fn end_frame(&mut self) {
        let _ = writeln!(
            self.frames,
            "frame {} commands {} draws {} hash {:016x}",
            self.frame, self.events, self.draws, self.hash
        );
        self.frame += 1;
        self.events = 0;
        self.draws = 0;
        self.hash = FNV_OFFSET;
    }
}

impl Gp {
    /// The frame being drawn, counting finishes.
    pub fn frame(&self) -> u64 {
        self.finishes
    }

    /// Logs the stream to `frames`, and frame `detail`'s draws and copies to its writer.
    pub fn record(&mut self, frames: Box<dyn Write>, detail: Option<(u64, Box<dyn Write>)>) {
        self.record = Some(Record {
            frames,
            detail,
            frame: 0,
            events: 0,
            draws: 0,
            hash: FNV_OFFSET,
        });
    }

    /// Records a Dolphin FIFO log of `count` frames from frame `start` to `path` (dff.rs).
    pub fn record_dff(&mut self, path: PathBuf, start: u64, count: usize) {
        self.dff = Some(DffPlan { path, start, count });
        if start == 0 {
            self.begin_dff();
        }
    }

    fn begin_dff(&mut self) {
        let s = &self.state;
        self.dff_log = Some(RefCell::new(Log::new(&s.bp, &s.cp, &s.xf, &s.tmem)));
    }

    /// Draws the stream with `sink` from now on.
    /// Lets the sink finish what it was given.
    pub fn finish(&mut self) {
        if let Some(sink) = &self.sink {
            sink.borrow_mut().finish();
        }
    }

    pub fn set_sink(&mut self, sink: Box<dyn Sink>) {
        if sink.tracks_changes() {
            self.state.track_changes();
        }
        self.sink = Some(RefCell::new(sink));
    }

    /// The game's FIFO ring, as the CP's registers set it.
    pub fn set_fifo(&mut self, start: u32, end: u32) {
        self.fifo = (start, end);
    }

    /// Feeds bytes from the FIFO and runs every command they complete.
    pub fn feed(&mut self, ctx: &Ctx, data: &[u8], out: &mut Vec<Effect>) {
        let mut produced = Vec::new();
        let mem = GpuMemory {
            ctx,
            log: self.dff_log.as_ref(),
            sink: self.sink.as_ref(),
            fifo: self.fifo,
        };
        // A FIFO log needs what draws read, which digesting them reads.
        let digests = self.record.is_some() || self.dff_log.is_some();
        self.state.feed(&mem, data, &mut produced, digests);
        for p in produced {
            if let Some(r) = &mut self.record {
                r.event(&p);
            }
            match p.event {
                Event::Draw { .. } => self.draws += 1,
                Event::Copy(v) => {
                    if ssbm_gx::is_xfb_copy(v | (ssbm_gx::TRIGGER_EFB_COPY as u32) << 24) {
                        self.xfb_copies += 1;
                    }
                }
                Event::Finish => {
                    self.finishes += 1;
                    if let Some(r) = &mut self.record {
                        r.end_frame();
                    }
                    out.push(Effect::Finish);
                }
                Event::Token { token, interrupt } => out.push(Effect::Token { token, interrupt }),
            }
        }
        self.dff_progress();
    }
}

impl Gp {
    /// A FIFO log starts once its first frame's XFB copy is past, from the state then, and is
    /// saved once it holds its frames.
    fn dff_progress(&mut self) {
        let Some(plan) = &self.dff else { return };
        let (start, count, path) = (plan.start, plan.count, plan.path.clone());
        let Some(log) = &self.dff_log else {
            if self.xfb_copies >= start {
                self.begin_dff();
            }
            return;
        };
        let log = log.borrow();
        if log.frames() < count {
            return;
        }
        let file =
            std::fs::File::create(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        log.save(&mut std::io::BufWriter::new(file))
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        eprintln!(
            "FIFO log of {} frames saved to {}",
            log.frames(),
            path.display()
        );
        drop(log);
        self.dff_log = None;
        self.dff = None;
    }
}
