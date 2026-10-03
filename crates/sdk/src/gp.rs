// SPDX-License-Identifier: GPL-3.0-or-later

//! The graphics processor: the command stream runs through the GPU's state (ssbm-gx), which
//! reports the commands that signal the CPU. A recorder can log each frame's draws and copies,
//! digested by what they use, to compare two runs' streams.

use std::io::Write;

use ssbm_gx::{Event, Produced, State};
use ssbm_rt::Ctx;

/// A command with an effect outside the GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    /// `PE_DONE`: the frame is drawn.
    Finish,
    /// `PE_TOKEN` or `PE_TOKEN_INT`.
    Token { token: u16, interrupt: bool },
}

/// Main memory as the GPU reads it, by physical address.
struct GpuMemory<'a>(&'a Ctx);

impl ssbm_gx::Memory for GpuMemory<'_> {
    fn read(&self, phys: u32, out: &mut [u8]) {
        if self.0.mem.read_bytes(0x8000_0000 | (phys & 0x01FF_FFFF), out).is_err() {
            out.fill(0);
        }
    }
}

pub(crate) struct Gp {
    state: State,
    pub draws: u64,
    finishes: u64,
    record: Option<Record>,
}

impl Default for Gp {
    fn default() -> Self {
        Gp {
            state: State::new(),
            draws: 0,
            finishes: 0,
            record: None,
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
            Event::Draw { primitive, vat, count } => {
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

    /// Feeds bytes from the FIFO and runs every command they complete.
    pub fn feed(&mut self, ctx: &Ctx, data: &[u8], out: &mut Vec<Effect>) {
        let mut produced = Vec::new();
        self.state.feed(&GpuMemory(ctx), data, &mut produced, self.record.is_some());
        for p in produced {
            if let Some(r) = &mut self.record {
                r.event(&p);
            }
            match p.event {
                Event::Draw { .. } => self.draws += 1,
                Event::Copy(_) => {}
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
    }
}
