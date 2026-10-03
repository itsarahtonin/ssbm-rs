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
    let direct = |size| Attr {
        array: 0,
        source: Source::Direct,
        size,
        indices: 1,
    };
    // Matrix indices: position/normal, then eight texture matrices, a byte each.
    for i in 0..9 {
        if bits(lo, i, 1) != 0 {
            out.push(direct(1));
        }
    }
    let mut push = |array, kind, size, indices| {
        let source = Source::from(kind);
        if source != Source::None {
            out.push(Attr {
                array,
                source,
                size,
                indices,
            });
        }
    };
    push(
        ARRAY_POS,
        bits(lo, 9, 2),
        (2 + bits(a, 0, 1)) * comp_size(bits(a, 1, 3)),
        1,
    );
    let nbt = bits(a, 9, 1) != 0;
    let index3 = nbt && bits(a, 31, 1) != 0;
    let nrm_kind = bits(lo, 11, 2);
    let nrm_size = (if nbt { 9 } else { 3 }) * comp_size(bits(a, 10, 3));
    push(
        ARRAY_NRM,
        nrm_kind,
        nrm_size,
        if index3 && nrm_kind >= 2 { 3 } else { 1 },
    );
    push(ARRAY_COL0, bits(lo, 13, 2), color_size(bits(a, 14, 3)), 1);
    push(
        ARRAY_COL0 + 1,
        bits(lo, 15, 2),
        color_size(bits(a, 18, 3)),
        1,
    );
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
        push(
            ARRAY_TEX0 + i,
            bits(hi, 2 * i as u32, 2),
            (1 + elems) * comp_size(*format),
            1,
        );
    }
    out
}

/// Bytes a vertex takes in the stream.
pub fn stream_size(layout: &[Attr]) -> u32 {
    layout
        .iter()
        .map(|a| a.source.stream_size(a.size, a.indices))
        .sum()
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

/// A vertex as the transform unit takes it (Dolphin's software renderer's InputVertexData):
/// its matrix indices, position, normal with binormals, colors as RGBA and texture coordinates.
#[derive(Clone, Copy, Debug, Default)]
pub struct Input {
    pub pos_mtx: u8,
    pub tex_mtx: [u8; 8],
    pub position: [f32; 3],
    pub normal: [[f32; 3]; 3],
    pub color: [[u8; 4]; 2],
    pub tex: [[f32; 2]; 8],
}

/// What a vertex without normals takes: the normal and binormals of the last vertex of the
/// last draw that had them.
pub type NormalCache = [[f32; 3]; 3];

fn component(data: &[u8], at: &mut usize, format: u32, scale: f32) -> f32 {
    let b = &data[*at..];
    let (v, n) = match format {
        0 => (f32::from(b[0]) * scale, 1),
        1 => (f32::from(b[0] as i8) * scale, 1),
        2 => (f32::from(u16::from_be_bytes([b[0], b[1]])) * scale, 2),
        3 => (f32::from(i16::from_be_bytes([b[0], b[1]])) * scale, 2),
        _ => (
            f32::from_bits(u32::from_be_bytes([b[0], b[1], b[2], b[3]])),
            4,
        ),
    };
    *at += n;
    v
}

/// The fixed scale of a normal's integer format.
fn normal_scale(format: u32) -> f32 {
    match format {
        0 => 1.0 / 128.0,
        1 => 1.0 / 64.0,
        2 => 1.0 / 32768.0,
        3 => 1.0 / 16384.0,
        _ => 1.0,
    }
}

fn expand(v: u32, bits: u32) -> u8 {
    match bits {
        4 => (v << 4 | v) as u8,
        5 => (v << 3 | v >> 2) as u8,
        _ => (v << 2 | v >> 4) as u8,
    }
}

fn color(data: &[u8], at: &mut usize, format: u32) -> [u8; 4] {
    let b = &data[*at..];
    *at += color_size(format) as usize;
    match format {
        0 => {
            let v = u32::from(u16::from_be_bytes([b[0], b[1]]));
            [
                expand(v >> 11, 5),
                expand(v >> 5 & 63, 6),
                expand(v & 31, 5),
                255,
            ]
        }
        1 | 2 => [b[0], b[1], b[2], 255],
        3 => [
            expand(u32::from(b[0] >> 4), 4),
            expand(u32::from(b[0] & 15), 4),
            expand(u32::from(b[1] >> 4), 4),
            expand(u32::from(b[1] & 15), 4),
        ],
        4 => {
            let v = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
            [
                expand(v >> 18 & 63, 6),
                expand(v >> 12 & 63, 6),
                expand(v >> 6 & 63, 6),
                expand(v & 63, 6),
            ]
        }
        _ => [b[0], b[1], b[2], b[3]],
    }
}

/// Decodes `count` vertices of format `vat` from `data`, a draw's vertices as `fetch` gives
/// them. Matrix indices a vertex doesn't carry come from the XF's matrix index registers
/// (`xf_index`: MATINDEX_A and B), missing colors are white, and missing normals come from
/// `normals`, which the last vertex with them updates.
pub fn decode(
    cp: &[u32; 256],
    xf_index: [u32; 2],
    vat: usize,
    data: &[u8],
    count: usize,
    normals: &mut NormalCache,
    out: &mut Vec<Input>,
) {
    let lo = cp[VCD_LO as usize];
    let hi = cp[VCD_HI as usize];
    let a = cp[VAT_A as usize + vat];
    let b = cp[VAT_B as usize + vat];
    let c = cp[VAT_C as usize + vat];
    // (elements bit, format, frac) per texture coordinate.
    let tex = [
        (bits(a, 21, 1), bits(a, 22, 3), bits(a, 25, 5)),
        (bits(b, 0, 1), bits(b, 1, 3), bits(b, 4, 5)),
        (bits(b, 9, 1), bits(b, 10, 3), bits(b, 13, 5)),
        (bits(b, 18, 1), bits(b, 19, 3), bits(b, 22, 5)),
        (bits(b, 27, 1), bits(b, 28, 3), bits(c, 0, 5)),
        (bits(c, 5, 1), bits(c, 6, 3), bits(c, 9, 5)),
        (bits(c, 14, 1), bits(c, 15, 3), bits(c, 18, 5)),
        (bits(c, 23, 1), bits(c, 24, 3), bits(c, 27, 5)),
    ];
    let pos_scale = 1.0 / (1u32 << bits(a, 4, 5)) as f32;
    let nbt = bits(a, 9, 1) != 0;
    let has_nrm = bits(lo, 11, 2) != 0;
    let (has_col0, has_col1) = (bits(lo, 13, 2) != 0, bits(lo, 15, 2) != 0);
    out.reserve(count);
    let mut at = 0;
    for _ in 0..count {
        let mut v = Input {
            pos_mtx: (xf_index[0] & 0x3F) as u8,
            normal: *normals,
            color: [[255; 4]; 2],
            ..Input::default()
        };
        for i in 0..8 {
            let reg = xf_index[usize::from(i >= 4)];
            v.tex_mtx[i] = (reg >> (6 * ((i % 4) + usize::from(i < 4))) & 0x3F) as u8;
        }
        if bits(lo, 0, 1) != 0 {
            v.pos_mtx = data[at] & 0x3F;
            at += 1;
        }
        for i in 0..8 {
            if bits(lo, 1 + i, 1) != 0 {
                v.tex_mtx[i as usize] = data[at] & 0x3F;
                at += 1;
            }
        }
        if bits(lo, 9, 2) != 0 {
            let n = 2 + bits(a, 0, 1);
            let format = bits(a, 1, 3);
            for k in 0..n as usize {
                v.position[k] = component(data, &mut at, format, pos_scale);
            }
        }
        if has_nrm {
            let format = bits(a, 10, 3);
            let scale = normal_scale(format);
            for k in 0..if nbt { 3 } else { 1 } {
                for j in 0..3 {
                    v.normal[k][j] = component(data, &mut at, format, scale);
                }
            }
        }
        // Only one color goes to channel 0.
        let col0 = has_col0.then(|| color(data, &mut at, bits(a, 14, 3)));
        let col1 = has_col1.then(|| color(data, &mut at, bits(a, 18, 3)));
        match (col0, col1) {
            (Some(c0), c1) => {
                v.color[0] = c0;
                v.color[1] = c1.unwrap_or([255; 4]);
            }
            (None, Some(c1)) => v.color[0] = c1,
            (None, None) => {}
        }
        for (i, (elems, format, frac)) in tex.iter().enumerate() {
            if bits(hi, 2 * i as u32, 2) != 0 {
                let scale = 1.0 / (1u32 << frac) as f32;
                for k in 0..=*elems as usize {
                    v.tex[i][k] = component(data, &mut at, *format, scale);
                }
            }
        }
        if has_nrm {
            let n = if nbt { 3 } else { 1 };
            normals[..n].copy_from_slice(&v.normal[..n]);
        }
        out.push(v);
    }
}
