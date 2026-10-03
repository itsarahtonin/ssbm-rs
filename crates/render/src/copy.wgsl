// SPDX-License-Identifier: GPL-3.0-or-later

// A copy from the EFB to a texture, kept on the GPU: each texel as the texture decoder would
// read back what the copy encoder (encode.rs) writes, for the formats copied this way.

struct Params {
    left: u32,
    top: u32,
    half: u32,
    format: u32,
    yuv: u32,
    six: u32,
    pad0: u32,
    pad1: u32,
}

@group(0) @binding(0) var efb: texture_2d<f32>;
@group(0) @binding(1) var<uniform> p: Params;

const EFB_WIDTH: u32 = 640u;
const EFB_HEIGHT: u32 = 528u;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

// EFB pixel `i`, counting along rows of 640 as the encoder does.
fn pixel(i: u32) -> vec4<u32> {
    if i >= EFB_WIDTH * EFB_HEIGHT {
        return vec4<u32>(0u);
    }
    let c = vec4<u32>(round(textureLoad(efb, vec2<u32>(i % EFB_WIDTH, i / EFB_WIDTH), 0) * 255.0));
    return vec4<u32>(c.rgb, select(255u, c.a, p.six != 0u));
}

// A texel's color: its pixel, or the 2x2 box at it at half scale.
fn texel(s: u32, t: u32) -> vec4<u32> {
    let i = p.top * EFB_WIDTH + p.left + (t << p.half) * EFB_WIDTH + (s << p.half);
    if p.half == 0u {
        return pixel(i);
    }
    let a = pixel(i);
    let b = pixel(i + 1u);
    let c = pixel(i + EFB_WIDTH);
    let d = pixel(i + EFB_WIDTH + 1u);
    if p.six != 0u {
        // Sums of six-bit values, widened.
        let sum = (a >> vec4<u32>(2u)) + (b >> vec4<u32>(2u)) + (c >> vec4<u32>(2u)) + (d >> vec4<u32>(2u));
        return (sum + (sum >> vec4<u32>(6u))) & vec4<u32>(0xFFu);
    }
    let avg = (a + b + c + d) >> vec4<u32>(2u);
    return vec4<u32>(avg.rgb, 255u);
}

fn c3(v: u32) -> u32 {
    return (v << 5u | v << 2u | v >> 1u) & 0xFFu;
}

fn c4(v: u32) -> u32 {
    return (v << 4u | v) & 0xFFu;
}

fn c5(v: u32) -> u32 {
    return (v << 3u | v >> 2u) & 0xFFu;
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<u32> {
    let c = texel(u32(pos.x), u32(pos.y));
    if p.format == 0u {
        // I4: the red channel, or the intensity, to four bits.
        var x = c.r;
        if p.yuv != 0u {
            x = (4096u + 66u * c.r + 129u * c.g + 25u * c.b) >> 8u;
        }
        let i = c4(x >> 4u);
        return vec4<u32>(i, i, i, i);
    }
    // RGB5A3: opaque as 5:5:5, otherwise 4:4:4 with three bits of alpha.
    if c.a >> 5u == 7u {
        return vec4<u32>(c5(c.r >> 3u), c5(c.g >> 3u), c5(c.b >> 3u), 255u);
    }
    return vec4<u32>(c4(c.r >> 4u), c4(c.g >> 4u), c4(c.b >> 4u), c3(c.a >> 5u));
}
