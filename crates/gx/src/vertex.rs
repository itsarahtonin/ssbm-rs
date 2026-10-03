// SPDX-License-Identifier: GPL-3.0-or-later
// Vertex formats follow Dolphin's VertexLoader (GPL-2.0-or-later).

//! Vertex layouts: which attributes a vertex has (the vertex descriptor, VCD) and their formats
//! (the vertex attribute table, VAT), and fetching a draw's vertices, indexed attributes from
//! their arrays in memory.

use crate::{Memory, bits};

/// CP registers.
pub const MATINDEX_A: u8 = 0x30;
pub const MATINDEX_B: u8 = 0x40;
pub const VCD_LO: u8 = 0x50;
pub const VCD_HI: u8 = 0x60;
pub const VAT_A: u8 = 0x70;
pub const VAT_B: u8 = 0x80;
pub const VAT_C: u8 = 0x90;
pub const ARRAY_BASE: u8 = 0xA0;
pub const ARRAY_STRIDE: u8 = 0xB0;

/// Arrays, by attribute: position, normal, two colors, eight texture coordinates; then the four
/// the XF's indexed loads read.
pub const ARRAY_POS: usize = 0;
pub const ARRAY_NRM: usize = 1;
pub const ARRAY_COL0: usize = 2;
pub const ARRAY_TEX0: usize = 4;
pub const ARRAY_XF_A: usize = 12;

/// How an attribute comes: absent, in the stream, or as an 8- or 16-bit index into its array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    None,
    Direct,
    Index8,
    Index16,
}

impl Source {
    fn from(kind: u32) -> Self {
        match kind {
            0 => Source::None,
            1 => Source::Direct,
            2 => Source::Index8,
            _ => Source::Index16,
        }
    }

    /// Bytes it takes in the stream, given the attribute's own size.
    fn stream_size(self, direct: u32, indices: u32) -> u32 {
        match self {
            Source::None => 0,
            Source::Direct => direct,
            Source::Index8 => indices,
            Source::Index16 => 2 * indices,
        }
    }
}

/// Bytes per component of a position, normal or texture coordinate format.
pub fn comp_size(format: u32) -> u32 {
    match format {
        0 | 1 => 1,
        2 | 3 => 2,
        _ => 4,
    }
}

/// Bytes per color of a color format.
pub fn color_size(format: u32) -> u32 {
    match format {
        0 | 3 => 2,
        1 | 4 => 3,
        _ => 4,
    }
}

/// One attribute of a vertex layout.
#[derive(Clone, Copy, Debug)]
pub struct Attr {
    /// Its array, for an indexed one.
    pub array: usize,
    pub source: Source,
    /// Bytes of the attribute itself.
    pub size: u32,
    /// For normals with separate indices for normal, binormal and tangent: 3.
    pub indices: u32,
}

/// The attributes of a vertex in vertex format `vat`, in stream order.
pub fn layout(cp: &[u32; 256], vat: usize) -> Vec<Attr> {
    let lo = cp[VCD_LO as usize];
    let hi = cp[VCD_HI as usize];
    let a = cp[VAT_A as usize + vat];
    let b = cp[VAT_B as usize + vat];
    let c = cp[VAT_C as usize + vat];
    let mut out = Vec::new();
    let direct = |size| Attr { array: 0, source: Source::Direct, size, indices: 1 };
    // Matrix indices: position/normal, then eight texture matrices, a byte each.
    for i in 0..9 {
        if bits(lo, i, 1) != 0 {
            out.push(direct(1));
        }
    }
    let mut push = |array, kind, size, indices| {
        let source = Source::from(kind);
        if source != Source::None {
            out.push(Attr { array, source, size, indices });
        }
    };
    push(ARRAY_POS, bits(lo, 9, 2), (2 + bits(a, 0, 1)) * comp_size(bits(a, 1, 3)), 1);
    let nbt = bits(a, 9, 1) != 0;
    let index3 = nbt && bits(a, 31, 1) != 0;
    let nrm_kind = bits(lo, 11, 2);
    let nrm_size = (if nbt { 9 } else { 3 }) * comp_size(bits(a, 10, 3));
    push(ARRAY_NRM, nrm_kind, nrm_size, if index3 && nrm_kind >= 2 { 3 } else { 1 });
    push(ARRAY_COL0, bits(lo, 13, 2), color_size(bits(a, 14, 3)), 1);
    push(ARRAY_COL0 + 1, bits(lo, 15, 2), color_size(bits(a, 18, 3)), 1);
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
        push(ARRAY_TEX0 + i, bits(hi, 2 * i as u32, 2), (1 + elems) * comp_size(*format), 1);
    }
    out
}

/// Bytes a vertex takes in the stream.
pub fn stream_size(layout: &[Attr]) -> u32 {
    layout.iter().map(|a| a.source.stream_size(a.size, a.indices)).sum()
}

/// Reads `count` vertices from `data` (the stream after the draw's header) into `out`, each
/// attribute as its bytes: an indexed one's from its array. Returns the stream bytes used.
pub fn fetch(
    cp: &[u32; 256],
    layout: &[Attr],
    count: u32,
    data: &[u8],
    mem: &dyn Memory,
    out: &mut Vec<u8>,
) -> usize {
    let mut at = 0;
    for _ in 0..count {
        for attr in layout {
            match attr.source {
                Source::None => {}
                Source::Direct => {
                    out.extend_from_slice(&data[at..at + attr.size as usize]);
                    at += attr.size as usize;
                }
                Source::Index8 | Source::Index16 => {
                    let base = cp[ARRAY_BASE as usize + attr.array] & 0x03FF_FFFF;
                    let stride = cp[ARRAY_STRIDE as usize + attr.array] & 0xFF;
                    let part = attr.size / attr.indices;
                    for k in 0..attr.indices {
                        let index = if attr.source == Source::Index8 {
                            u32::from(data[at])
                        } else {
                            u32::from(u16::from_be_bytes([data[at], data[at + 1]]))
                        };
                        at += if attr.source == Source::Index8 { 1 } else { 2 };
                        let start = out.len();
                        out.resize(start + part as usize, 0);
                        mem.read(base + index * stride + k * part, &mut out[start..]);
                    }
                }
            }
        }
    }
    at
}
