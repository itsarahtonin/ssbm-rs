// SPDX-License-Identifier: GPL-3.0-or-later
// Follows Dolphin's software renderer's TransformUnit (GPL-2.0-or-later).

//! The transform unit: a vertex's position through its matrix and the projection, its normals,
//! its colors through the lighting channels, and its texture coordinates through the texture
//! coordinate generators, as the XF's memory and registers set them.

use crate::bits;
use crate::vertex::Input;

/// XF memory: matrices, lights, and registers.
pub const POS_MATRICES: usize = 0x000;
pub const NORMAL_MATRICES: usize = 0x400;
pub const POST_MATRICES: usize = 0x500;
pub const LIGHTS: usize = 0x600;
pub const AMB_COLOR: usize = 0x100A;
pub const MAT_COLOR: usize = 0x100C;
pub const COLOR_CHAN: usize = 0x100E;
pub const ALPHA_CHAN: usize = 0x1010;
pub const DUAL_TEX: usize = 0x1012;
pub const MATINDEX_A: usize = 0x1018;
pub const VIEWPORT: usize = 0x101A;
pub const PROJECTION: usize = 0x1020;
pub const NUM_TEXGENS: usize = 0x103F;
pub const TEXMTX_INFO: usize = 0x1040;
pub const POSTMTX_INFO: usize = 0x1050;

/// BP: texture coordinate scales (SU_SSIZE, SU_TSIZE), two per coordinate.
pub const SU_SSIZE: usize = 0x30;

type Vec3 = [f32; 3];

/// A transformed vertex.
#[derive(Clone, Copy, Debug, Default)]
pub struct Output {
    /// In view space.
    pub mv: Vec3,
    /// In clip space.
    pub clip: [f32; 4],
    pub normal: [Vec3; 3],
    /// RGBA.
    pub color: [[u8; 4]; 2],
    /// Scaled to texels; the third is q.
    pub tex: [Vec3; 8],
}

fn f(xf: &[u32], at: usize) -> f32 {
    f32::from_bits(xf[at])
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: Vec3, s: f32) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn normalized(a: Vec3) -> Vec3 {
    let len = dot(a, a).sqrt();
    [a[0] / len, a[1] / len, a[2] / len]
}

fn vec3(xf: &[u32], at: usize) -> Vec3 {
    [f(xf, at), f(xf, at + 1), f(xf, at + 2)]
}

/// A 3x4 matrix at `at` times (v, 1).
fn mat34(xf: &[u32], at: usize, v: Vec3) -> Vec3 {
    let m = |i| f(xf, at + i);
    [
        m(0) * v[0] + m(1) * v[1] + m(2) * v[2] + m(3),
        m(4) * v[0] + m(5) * v[1] + m(6) * v[2] + m(7),
        m(8) * v[0] + m(9) * v[1] + m(10) * v[2] + m(11),
    ]
}

/// Transforms vertex `v` by the XF's state, and scales its texture coordinates by the BP's.
pub fn transform(xf: &[u32], bp: &[u32; 256], v: &Input) -> Output {
    let mut out = Output::default();
    position(xf, v, &mut out);
    let nm = NORMAL_MATRICES + usize::from(v.pos_mtx & 31) * 3;
    for k in 0..3 {
        let n = v.normal[k];
        let m = |i| f(xf, nm + i);
        out.normal[k] = [
            m(0) * n[0] + m(1) * n[1] + m(2) * n[2],
            m(3) * n[0] + m(4) * n[1] + m(5) * n[2],
            m(6) * n[0] + m(7) * n[1] + m(8) * n[2],
        ];
    }
    out.normal[0] = normalized(out.normal[0]);
    colors(xf, v, &mut out);
    tex_coords(xf, bp, v, &mut out);
    out
}

fn position(xf: &[u32], v: &Input, out: &mut Output) {
    out.mv = mat34(xf, POS_MATRICES + usize::from(v.pos_mtx) * 4, v.position);
    let p = |i| f(xf, PROJECTION + i);
    let mv = out.mv;
    out.clip = if xf[PROJECTION + 6] == 0 {
        [
            p(0) * mv[0] + p(1) * mv[2],
            p(2) * mv[1] + p(3) * mv[2],
            (p(4) * mv[2] + p(5)) * (1.0 - 1e-7),
            -mv[2],
        ]
    } else {
        [
            p(0) * mv[0] + p(1),
            p(2) * mv[1] + p(3),
            p(4) * mv[2] + p(5),
            1.0,
        ]
    };
}

/// A light: color (RGBA), angle and distance attenuation, position and direction.
struct Light {
    color: [u8; 4],
    cos_att: Vec3,
    dist_att: Vec3,
    pos: Vec3,
    dir: Vec3,
}

impl Light {
    fn of(xf: &[u32], n: usize) -> Self {
        let at = LIGHTS + n * 16;
        Light {
            color: xf[at + 3].to_be_bytes(),
            cos_att: vec3(xf, at + 4),
            dist_att: vec3(xf, at + 7),
            pos: vec3(xf, at + 10),
            dir: vec3(xf, at + 13),
        }
    }
}

fn safe_divide(n: f32, d: f32) -> f32 {
    if d == 0.0 {
        if n > 0.0 { 1.0 } else { 0.0 }
    } else {
        n / d
    }
}

/// A light's attenuation for a channel `chan`, and the direction to it (normalized).
fn attenuation(light: &Light, ldir: &mut Vec3, normal: Vec3, chan: u32) -> f32 {
    match bits(chan, 9, 2) {
        // Spec
        1 => {
            *ldir = normalized(*ldir);
            let attn = if dot(*ldir, normal) >= 0.0 {
                dot(light.dir, normal).max(0.0)
            } else {
                0.0
            };
            let len = [1.0, attn, attn * attn];
            let dist = if bits(chan, 7, 2) != 0 {
                normalized(light.dist_att)
            } else {
                light.dist_att
            };
            safe_divide(dot(len, light.cos_att).max(0.0), dot(len, dist))
        }
        // Spot
        3 => {
            let dist2 = dot(*ldir, *ldir);
            let dist = dist2.sqrt();
            *ldir = scale(*ldir, 1.0 / dist);
            let attn = dot(*ldir, light.dir).max(0.0);
            let c = light.cos_att;
            let d = light.dist_att;
            let cos_att = c[0] + c[1] * attn + c[2] * attn * attn;
            let dist_att = d[0] + d[1] * dist + d[2] * dist2;
            safe_divide(cos_att.max(0.0), dist_att)
        }
        // None and Dir
        _ => {
            *ldir = normalized(*ldir);
            if *ldir == [0.0; 3] {
                *ldir = normal;
            }
            1.0
        }
    }
}

/// A light's contribution's scale (its color to be multiplied by it) to channel `chan`.
fn contribution(light: &Light, out: &Output, chan: u32) -> f32 {
    let mut ldir = sub(light.pos, out.mv);
    let attn = attenuation(light, &mut ldir, out.normal[0], chan);
    let dif = dot(ldir, out.normal[0]);
    match bits(chan, 7, 2) {
        0 => attn,
        1 => attn * dif,
        _ => attn * dif.max(0.0),
    }
}

fn light_mask(chan: u32) -> u32 {
    if bits(chan, 1, 1) != 0 {
        bits(chan, 2, 4) | bits(chan, 11, 4) << 4
    } else {
        0
    }
}

fn lit(material: u8, light: f32) -> u8 {
    let l = (light as i32).clamp(0, 255);
    ((i32::from(material) * (l + (l >> 7))) >> 8) as u8
}

fn colors(xf: &[u32], v: &Input, out: &mut Output) {
    for ch in 0..2 {
        let cc = xf[COLOR_CHAN + ch];
        let ac = xf[ALPHA_CHAN + ch];
        let mat_reg = xf[MAT_COLOR + ch].to_be_bytes();
        let amb_reg = xf[AMB_COLOR + ch].to_be_bytes();
        let mut color = [0u8; 4];
        let mat = if bits(cc, 0, 1) != 0 {
            v.color[ch]
        } else {
            mat_reg
        };
        if bits(cc, 1, 1) != 0 {
            let amb = if bits(cc, 6, 1) != 0 {
                v.color[ch]
            } else {
                amb_reg
            };
            let mut light = [f32::from(amb[0]), f32::from(amb[1]), f32::from(amb[2])];
            let mask = light_mask(cc);
            for i in 0..8 {
                if mask & (1 << i) != 0 {
                    let l = Light::of(xf, i);
                    let s = contribution(&l, out, cc);
                    for k in 0..3 {
                        light[k] += f32::from(l.color[k]) * s;
                    }
                }
            }
            for k in 0..3 {
                color[k] = lit(mat[k], light[k]);
            }
        } else {
            color[..3].copy_from_slice(&mat[..3]);
        }
        let mat_a = if bits(ac, 0, 1) != 0 {
            v.color[ch][3]
        } else {
            mat_reg[3]
        };
        color[3] = if bits(ac, 1, 1) != 0 {
            let mut light = if bits(ac, 6, 1) != 0 {
                f32::from(v.color[ch][3])
            } else {
                f32::from(amb_reg[3])
            };
            let mask = light_mask(ac);
            for i in 0..8 {
                if mask & (1 << i) != 0 {
                    let l = Light::of(xf, i);
                    light += f32::from(l.color[3]) * contribution(&l, out, ac);
                }
            }
            lit(mat_a, light)
        } else {
            mat_a
        };
        out.color[ch] = color;
    }
}

fn tex_coords(xf: &[u32], bp: &[u32; 256], v: &Input, out: &mut Output) {
    let count = (xf[NUM_TEXGENS] & 0xF).min(8) as usize;
    for n in 0..count {
        let info = xf[TEXMTX_INFO + n];
        match bits(info, 4, 3) {
            // Emboss map: offsets another coordinate by the light's direction along the binormals.
            1 => {
                let light = Light::of(xf, bits(info, 15, 3) as usize);
                let ldir = normalized(sub(light.pos, out.mv));
                let src = out.tex[bits(info, 12, 3) as usize];
                out.tex[n] = [
                    src[0] + dot(ldir, out.normal[1]),
                    src[1] + dot(ldir, out.normal[2]),
                    src[2],
                ];
            }
            2 | 3 => {
                let c = out.color[bits(info, 4, 3) as usize - 2];
                out.tex[n] = [f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, 1.0];
            }
            _ => out.tex[n] = regular(xf, info, n, v),
        }
    }
    for n in 0..count {
        out.tex[n][0] *= (bp[SU_SSIZE + 2 * n] & 0xFFFF) as f32 + 1.0;
        out.tex[n][1] *= (bp[SU_SSIZE + 2 * n + 1] & 0xFFFF) as f32 + 1.0;
    }
}

fn regular(xf: &[u32], info: u32, n: usize, v: &Input) -> Vec3 {
    let mut src = match bits(info, 7, 5) {
        0 => v.position,
        1 => v.normal[0],
        3 => v.normal[1],
        4 => v.normal[2],
        row @ 5..=12 => {
            let t = v.tex[row as usize - 5];
            [t[0], t[1], 1.0]
        }
        _ => [0.0, 0.0, 1.0],
    };
    for c in &mut src {
        if c.is_nan() {
            *c = 1.0;
        }
    }
    let m = POS_MATRICES + usize::from(v.tex_mtx[n]) * 4;
    let mf = |i| f(xf, m + i);
    // AB11 takes (a, b, 1, 1); ABC1 (a, b, c, 1).
    let ab11 = bits(info, 2, 1) == 0;
    let row = |r: usize| {
        let third = if ab11 {
            mf(4 * r + 2)
        } else {
            mf(4 * r + 2) * src[2]
        };
        mf(4 * r) * src[0] + mf(4 * r + 1) * src[1] + third + mf(4 * r + 3)
    };
    let stq = bits(info, 1, 1) != 0;
    let mut dst = [row(0), row(1), if stq { row(2) } else { 1.0 }];
    if xf[DUAL_TEX] & 1 != 0 {
        let post = xf[POSTMTX_INFO + n];
        let tmp = if bits(post, 8, 1) != 0 {
            normalized(dst)
        } else {
            dst
        };
        dst = mat34(xf, POST_MATRICES + bits(post, 0, 6) as usize * 4, tmp);
    }
    if dst[2] == 0.0 {
        dst[0] = (dst[0] / 2.0).clamp(-1.0, 1.0);
        dst[1] = (dst[1] / 2.0).clamp(-1.0, 1.0);
    }
    dst
}

/// The viewport: width and height halves, depth range, origin, and far depth.
#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub wd: f32,
    pub ht: f32,
    pub z_range: f32,
    pub x_orig: f32,
    pub y_orig: f32,
    pub far_z: f32,
}

impl Viewport {
    pub fn of(xf: &[u32]) -> Self {
        Viewport {
            wd: f(xf, VIEWPORT),
            ht: f(xf, VIEWPORT + 1),
            z_range: f(xf, VIEWPORT + 2),
            x_orig: f(xf, VIEWPORT + 3),
            y_orig: f(xf, VIEWPORT + 4),
            far_z: f(xf, VIEWPORT + 5),
        }
    }

    /// A clip-space position in screen space (with the scissor's offset still in it).
    pub fn screen(&self, clip: [f32; 4]) -> Vec3 {
        let inv = 1.0 / clip[3];
        [
            clip[0] * inv * self.wd + self.x_orig,
            clip[1] * inv * self.ht + self.y_orig,
            clip[2] * inv * self.z_range + self.far_z,
        ]
    }
}

/// Whether a triangle faces back, as the GPU judges it (Dolphin's Clipper::IsBackface).
pub fn is_backface(xf: &[u32], v0: [f32; 4], v1: [f32; 4], v2: [f32; 4]) -> bool {
    let (x0, y0, w0) = (v0[0], v0[1], v0[3]);
    let (x1, y1, w1) = (v1[0], v1[1], v1[3]);
    let (x2, y2, w2) = (v2[0], v2[1], v2[3]);
    let normal_z = (x0 * w2 - x2 * w0) * y1 + (x2 * y0 - x0 * y2) * w1 + (y2 * w0 - y0 * w2) * x1;
    let backface = normal_z <= 0.0;
    if f(xf, VIEWPORT + 1) > 0.0 {
        !backface
    } else {
        backface
    }
}
