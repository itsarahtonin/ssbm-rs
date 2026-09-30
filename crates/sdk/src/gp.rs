// SPDX-License-Identifier: GPL-3.0-or-later
// Command formats follow Dolphin's OpcodeDecoding and VertexLoader (GPL-2.0-or-later).

//! The graphics processor's command stream. For now this tracks just enough state to walk the
//! stream (vertex formats, display lists) and reports the commands that raise interrupts.

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
            0x45 if value & 0xFF == 2 => out.push(Effect::Finish),
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
                    let (addr, size) = (be32(&rest[1..]), be32(&rest[5..]));
                    self.call_display_list(ctx, addr, size, out);
                    9
                }
                0x61 => {
                    if rest.len() < 5 {
                        break;
                    }
                    self.load_bp(be32(&rest[1..]), out);
                    5
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

    /// Feeds bytes from the FIFO and runs every command they complete.
    pub fn feed(&mut self, ctx: &Ctx, data: &[u8], out: &mut Vec<Effect>) {
        self.buf.extend_from_slice(data);
        let buf = std::mem::take(&mut self.buf);
        let used = self.run(ctx, &buf, out);
        self.buf = buf[used..].to_vec();
    }
}
