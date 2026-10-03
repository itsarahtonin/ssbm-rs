// SPDX-License-Identifier: GPL-3.0-or-later
// Follows Dolphin's software renderer's EncodeXFB and its XFB decoder (GPL-2.0-or-later).

// Copies to the XFB on the GPU, as xfb.rs does on the CPU: the EFB through the copy filter and
// gamma to YUV 4:2:2 (encode), then back to RGB as the video interface shows it (decode).

struct Encode {
    left: u32,
    top: u32,
    // The rows the filter's neighbors are clamped to.
    first_row: u32,
    last_row: u32,
    width: u32,
    w_prev: u32,
    w_mid: u32,
    w_next: u32,
    // The gamma table, four entries a vector.
    lut: array<vec4<u32>, 64>,
}

@group(0) @binding(0) var efb: texture_2d<f32>;
@group(0) @binding(1) var<uniform> e: Encode;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn rgb(x: u32, y: u32) -> vec3<u32> {
    return vec3<u32>(round(textureLoad(efb, vec2<u32>(x, y), 0).rgb * 255.0));
}

fn gamma(v: u32) -> u32 {
    return e.lut[v >> 2u][v & 3u];
}

// Pixel `i` of the copied row `y`, through the copy filter and gamma, as YUV.
fn yuv(i: u32, y: u32) -> vec3<i32> {
    let x = e.left + i;
    let p = rgb(x, max(y, e.first_row + 1u) - 1u);
    let c = rgb(x, y);
    let n = rgb(x, min(y + 1u, e.last_row));
    let sum = p * e.w_prev + c * e.w_mid + n * e.w_next;
    let f = min(sum >> vec3<u32>(6u), vec3<u32>(255u));
    let r = i32(gamma(f.r));
    let g = i32(gamma(f.g));
    let b = i32(gamma(f.b));
    let yy = 66 * r + 129 * g + 25 * b;
    let u = -38 * r - 74 * g + 112 * b;
    let v = 112 * r - 94 * g - 18 * b;
    return vec3<i32>(
        (yy >> 8u) + ((yy >> 7u) & 1),
        (u >> 8u) + ((u >> 7u) & 1),
        (v >> 8u) + ((v >> 7u) & 1),
    );
}

// Each pixel's Y and its pair's U (even pixels) or V (odd), from the chroma of the pixels about
// the pair, the row's ends repeated.
@fragment
fn encode(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<u32> {
    let x = u32(pos.x);
    let y = e.top + u32(pos.y);
    let pair = x & ~1u;
    let a = yuv(max(pair, 1u) - 1u, y);
    let b = yuv(pair, y);
    let c = yuv(min(pair + 1u, e.width - 1u), y);
    let odd = x & 1u;
    let luma = select(b.x, c.x, odd == 1u);
    let k = 1u + odd;
    let chroma = 128 + ((a[k] + (b[k] << 1u) + c[k]) >> 2u);
    return vec4<u32>(u32(luma + 16) & 0xFFu, u32(chroma) & 0xFFu, 0u, 0u);
}

struct Decode {
    width: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
    // The decoder's products, by Y + 16 or by U or V + 128, rounded as the CPU rounds them:
    // with one sum each left, the GPU can neither fuse them into multiply-adds nor reorder them,
    // which would round differently.
    y: array<vec4<f32>, 64>,
    rv: array<vec4<f32>, 64>,
    gv: array<vec4<f32>, 64>,
    bu: array<vec4<f32>, 64>,
}

// Entry `i` of a table, given its vector `i / 4`.
fn entry(v: vec4<f32>, i: u32) -> f32 {
    return v[i & 3u];
}

@group(0) @binding(0) var yuyv: texture_2d<u32>;
@group(0) @binding(1) var<uniform> d: Decode;
// The XFB row each row shown reads, by the copy's vertical scale.
@group(0) @binding(2) var<storage, read> rows: array<u32>;
// Green's luma less its U term, by (Y + 16) * 256 + U + 128.
@group(0) @binding(3) var<storage, read> g_yu: array<f32>;

@fragment
fn decode(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let x = u32(pos.x);
    let sy = rows[u32(pos.y)];
    let pair = x & ~1u;
    let first = textureLoad(yuyv, vec2<u32>(pair, sy), 0).rg;
    var second = vec2<u32>(first.r, 128u);
    if pair + 1u < d.width {
        second = textureLoad(yuyv, vec2<u32>(pair + 1u, sy), 0).rg;
    }
    let luma = select(first.r, second.r, (x & 1u) == 1u);
    let y = entry(d.y[luma >> 2u], luma);
    let u = first.g;
    let v = second.g;
    let r = clamp(i32(y + entry(d.rv[v >> 2u], v)), 0, 255);
    let g = clamp(i32(g_yu[luma * 256u + u] - entry(d.gv[v >> 2u], v)), 0, 255);
    let b = clamp(i32(y + entry(d.bu[u >> 2u], u)), 0, 255);
    return vec4<f32>(f32(r), f32(g), f32(b), 255.0) / 255.0;
}
