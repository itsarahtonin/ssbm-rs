// SPDX-License-Identifier: GPL-3.0-or-later
// Command formats follow Dolphin's OpcodeDecoding and VertexLoader (GPL-2.0-or-later).

//! The graphics processor's command stream. For now this tracks just enough state to walk the
//! stream (vertex formats, display lists) and reports the commands that raise interrupts. A
//! recorder can log each frame's commands, to compare two runs' streams (record).

use std::io::Write;

use ssbm_rt::Ctx;

/// A command with an effect outside the GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    /// `PE_DONE`: the frame is drawn.
    Finish,
    /// `PE_TOKEN` or `PE_TOKEN_INT`.
    Token { token: u16, interrupt: bool },
}

#[derive(Default)]
pub(crate) struct Gp {
    /// Bytes of an incomplete command.
    buf: Vec<u8>,
    vcd_lo: u32,
    vcd_hi: u32,
    vat_a: [u32; 8],
    vat_b: [u32; 8],
    vat_c: [u32; 8],
    pub draws: u64,
    record: Option<Record>,
}

/// Logs the command stream frame by frame: per frame drawn (each `PE_DONE`), a line with its
/// commands, draws and a hash of every command's bytes, display lists' expanded where they are
/// called; and every command of one frame, decoded, for comparing two runs command by command.
struct Record {
    frames: Box<dyn Write>,
    detail: Option<(u64, Box<dyn Write>)>,
    frame: u64,
    commands: u64,
    draws: u64,
    hash: u64,
}

const FNV_OFFSET: u64 = 0xCBF2_9CE4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;

fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h = (h ^ u64::from(b)).wrapping_mul(FNV_PRIME);
    }
    h
}

impl Record {
    fn command(&mut self, bytes: &[u8], draw: bool) {
        self.commands += 1;
        self.draws += u64::from(draw);
        self.hash = fnv(self.hash, bytes);
        if let Some((frame, out)) = &mut self.detail
            && *frame == self.frame
        {
            let _ = writeln!(out, "{}", describe(bytes));
        }
    }

    fn end_frame(&mut self) {
        let _ = writeln!(
            self.frames,
            "frame {} commands {} draws {} hash {:016x}",
            self.frame, self.commands, self.draws, self.hash
        );
        self.frame += 1;
        self.commands = 0;
        self.draws = 0;
        self.hash = FNV_OFFSET;
    }
}

/// One command, decoded enough to compare: register writes with their values, draws with a
/// hash of their vertices.
fn describe(b: &[u8]) -> String {
    match b[0] {
        0x00 => "NOP".into(),
        0x08 => format!("CP {:02X} {:08X}", b[1], be32(&b[2..])),
        0x10 => {
            let head = be32(&b[1..]);
            let values: Vec<String> = b[5..].chunks(4).map(|v| format!("{:08X}", be32(v))).collect();
            format!("XF {:04X} {}", head & 0xFFFF, values.join(" "))
        }
        op @ (0x20 | 0x28 | 0x30 | 0x38) => {
            format!("XF_INDEXED_{} {:08X}", (b'A' + (op - 0x20) / 8) as char, be32(&b[1..]))
        }
        0x40 => format!("CALL_DL {:08X} {}", be32(&b[1..]), be32(&b[5..])),
        0x44 => "UNKNOWN_44".into(),
        0x48 => "INVALIDATE_VERTEX_CACHE".into(),
        0x61 => format!("BP {:02X} {:06X}", b[1], be32(&b[1..]) & 0xFF_FFFF),
        op => format!(
            "DRAW {:02X} vat {} count {} vertices {:016x}",
            op & 0xF8,
            op & 7,
            u16::from_be_bytes([b[1], b[2]]),
            fnv(FNV_OFFSET, &b[3..])
        ),
    }
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Bytes per component of a position, normal or texture coordinate format.
fn comp_size(format: u32) -> u32 {
    match format {
        0 | 1 => 1,
        2 | 3 => 2,
        _ => 4,
    }
}

fn color_size(format: u32) -> u32 {
    match format {
        0 | 3 => 2,
        1 | 4 => 3,
        _ => 4,
    }
}

fn bits(v: u32, at: u32, width: u32) -> u32 {
    (v >> at) & ((1 << width) - 1)
}

impl Gp {
    /// Bytes per vertex for vertex format `vat`.
    fn vertex_size(&self, vat: usize) -> u32 {
        let (lo, hi) = (self.vcd_lo, self.vcd_hi);
        let (a, b, c) = (self.vat_a[vat], self.vat_b[vat], self.vat_c[vat]);
        // Matrix indices: position/normal, then eight texture matrices.
        let mut size = (lo & 1) + (1..=8).map(|i| bits(lo, i, 1)).sum::<u32>();
        let indexed = |kind: u32, direct: u32| match kind {
            0 => 0,
            1 => direct,
            2 => 1,
            _ => 2,
        };
        let pos_kind = bits(lo, 9, 2);
        size += indexed(pos_kind, (2 + bits(a, 0, 1)) * comp_size(bits(a, 1, 3)));
        let nrm_kind = bits(lo, 11, 2);
        let nbt = bits(a, 9, 1) != 0;
        size += match nrm_kind {
            0 => 0,
            1 => (if nbt { 9 } else { 3 }) * comp_size(bits(a, 10, 3)),
            k => {
                let n = if nbt && bits(a, 31, 1) != 0 { 3 } else { 1 };
                n * if k == 2 { 1 } else { 2 }
            }
        };
        size += indexed(bits(lo, 13, 2), color_size(bits(a, 14, 3)));
        size += indexed(bits(lo, 15, 2), color_size(bits(a, 18, 3)));
        // Texture coordinates: (elements bit, format) per coordinate.
        let tex = [
            (bits(a, 21, 1), bits(a, 22, 3)),
            (bits(b, 0, 1), bits(b, 1, 3)),
            (bits(b, 9, 1), bits(b, 10, 3)),
            (bits(b, 18, 1), bits(b, 19, 3)),
            (bits(b, 27, 1), bits(b, 28, 3)),
            (bits(c, 5, 1), bits(c, 6, 3)),
            (bits(c, 14, 1), bits(c, 15, 3)),
            (bits(c, 23, 1), bits(c, 24, 3)),
        ];
        for (i, (elems, format)) in tex.iter().enumerate() {
            size += indexed(bits(hi, 2 * i as u32, 2), (1 + elems) * comp_size(*format));
        }
        size
    }

    fn load_cp(&mut self, reg: u8, value: u32) {
        let n = usize::from(reg & 7);
        match reg & 0xF0 {
            0x50 => self.vcd_lo = value,
            0x60 => self.vcd_hi = value,
            0x70 => self.vat_a[n] = value,
            0x80 => self.vat_b[n] = value,
            0x90 => self.vat_c[n] = value,
            _ => {}
        }
    }

    fn load_bp(&mut self, value: u32, out: &mut Vec<Effect>) {
        match value >> 24 {
            0x45 if value & 0xFF == 2 => {
                if let Some(r) = &mut self.record {
                    r.end_frame();
                }
                out.push(Effect::Finish)
            }
            0x47 => out.push(Effect::Token {
                token: value as u16,
                interrupt: false,
            }),
            0x48 => out.push(Effect::Token {
                token: value as u16,
                interrupt: true,
            }),
            _ => {}
        }
    }

    /// Runs complete commands at the start of `data`; returns how many bytes they used.
    fn run(&mut self, ctx: &Ctx, data: &[u8], out: &mut Vec<Effect>) -> usize {
        let mut at = 0;
        while at < data.len() {
            let rest = &data[at..];
            let op = rest[0];
            let len = match op {
                0x00 | 0x44 | 0x48 => 1,
                0x08 => {
                    if rest.len() < 6 {
                        break;
                    }
                    self.load_cp(rest[1], be32(&rest[2..]));
                    6
                }
                0x10 => {
                    if rest.len() < 5 {
                        break;
                    }
                    let count = ((be32(&rest[1..]) >> 16) & 0xF) as usize + 1;
                    if rest.len() < 5 + 4 * count {
                        break;
                    }
                    5 + 4 * count
                }
                0x20 | 0x28 | 0x30 | 0x38 => {
                    if rest.len() < 5 {
                        break;
                    }
                    5
                }
                0x40 => {
                    if rest.len() < 9 {
                        break;
                    }
                    if let Some(r) = &mut self.record {
                        r.command(&rest[..9], false);
                    }
                    let (addr, size) = (be32(&rest[1..]), be32(&rest[5..]));
                    self.call_display_list(ctx, addr, size, out);
                    at += 9;
                    continue;
                }
                0x61 => {
                    if rest.len() < 5 {
                        break;
                    }
                    // Logged before it runs, so the frame a finish ends holds it.
                    if let Some(r) = &mut self.record {
                        r.command(&rest[..5], false);
                    }
                    self.load_bp(be32(&rest[1..]), out);
                    at += 5;
                    continue;
                }
                0x80..=0xBF => {
                    if rest.len() < 3 {
                        break;
                    }
                    let count = u32::from(u16::from_be_bytes([rest[1], rest[2]]));
                    let len = 3 + (count * self.vertex_size(usize::from(op & 7))) as usize;
                    if rest.len() < len {
                        break;
                    }
                    self.draws += 1;
                    len
                }
                _ => panic!(
                    "unknown GX command {op:#04X}; next bytes {:02X?}",
                    &rest[..rest.len().min(16)]
                ),
            };
            if let Some(r) = &mut self.record {
                r.command(&rest[..len], op >= 0x80);
            }
            at += len;
        }
        at
    }

    fn call_display_list(&mut self, ctx: &Ctx, addr: u32, size: u32, out: &mut Vec<Effect>) {
        let base = 0x8000_0000 | (addr & 0x01FF_FFFF);
        let data: Vec<u8> = (0..size).map(|i| ctx.read_u8(base + i)).collect();
        // A display list holds whole commands; the parse state of the FIFO is untouched.
        let used = self.run(ctx, &data, out);
        assert!(
            data[used..].iter().all(|&b| b == 0),
            "display list at {addr:#010X} ends in the middle of a command"
        );
    }

    /// Logs the command stream to `frames`, and frame `detail`'s commands to its writer.
    pub fn record(&mut self, frames: Box<dyn Write>, detail: Option<(u64, Box<dyn Write>)>) {
        self.record = Some(Record {
            frames,
            detail,
            frame: 0,
            commands: 0,
            draws: 0,
            hash: FNV_OFFSET,
        });
    }

    /// Feeds bytes from the FIFO and runs every command they complete.
    pub fn feed(&mut self, ctx: &Ctx, data: &[u8], out: &mut Vec<Effect>) {
        self.buf.extend_from_slice(data);
        let buf = std::mem::take(&mut self.buf);
        let used = self.run(ctx, &buf, out);
        self.buf = buf[used..].to_vec();
    }
}
