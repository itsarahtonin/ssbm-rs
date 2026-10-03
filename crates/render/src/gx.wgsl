// SPDX-License-Identifier: GPL-3.0-or-later
// Follows Dolphin's software renderer (Tev, TextureSampler, Rasterizer; GPL-2.0-or-later): the
// TEV in integers, textures sampled by hand from decoded texels, and fog, per pixel.

enable dual_source_blending;

struct State {
    bp: array<vec4<u32>, 64>,
    // TEV registers: prev, c0, c1, c2; then the konstant colors. RGBA, 11-bit signed.
    colors: array<vec4<i32>, 4>,
    konsts: array<vec4<i32>, 4>,
    // Viewport z range and far z, fog A and C, viewport half width.
    depth: vec4<f32>,
    fog: vec4<f32>,
    // Mip levels of each texture map's texture.
    levels: array<vec4<u32>, 2>,
    // x: draw mode (0 draw, 1 clear, 2 depth only); y: clear color (RGBA8); z: clear depth.
    mode: vec4<u32>,
}

@group(0) @binding(0) var<uniform> st: State;
@group(1) @binding(0) var tex0: texture_2d<u32>;
@group(1) @binding(1) var tex1: texture_2d<u32>;
@group(1) @binding(2) var tex2: texture_2d<u32>;
@group(1) @binding(3) var tex3: texture_2d<u32>;
@group(1) @binding(4) var tex4: texture_2d<u32>;
@group(1) @binding(5) var tex5: texture_2d<u32>;
@group(1) @binding(6) var tex6: texture_2d<u32>;
@group(1) @binding(7) var tex7: texture_2d<u32>;

struct Vertex {
    @location(0) pos: vec4<f32>,
    @location(1) c0: vec4<f32>,
    @location(2) c1: vec4<f32>,
    @location(3) t0: vec3<f32>,
    @location(4) t1: vec3<f32>,
    @location(5) t2: vec3<f32>,
    @location(6) t3: vec3<f32>,
    @location(7) t4: vec3<f32>,
    @location(8) t5: vec3<f32>,
    @location(9) t6: vec3<f32>,
    @location(10) t7: vec3<f32>,
}

struct Varyings {
    @builtin(position) pos: vec4<f32>,
    // Colors interpolate linearly in screen space, texture coordinates in perspective.
    @location(0) @interpolate(linear) c0: vec4<f32>,
    @location(1) @interpolate(linear) c1: vec4<f32>,
    @location(2) t0: vec3<f32>,
    @location(3) t1: vec3<f32>,
    @location(4) t2: vec3<f32>,
    @location(5) t3: vec3<f32>,
    @location(6) t4: vec3<f32>,
    @location(7) t5: vec3<f32>,
    @location(8) t6: vec3<f32>,
    @location(9) t7: vec3<f32>,
}

@vertex
fn vs_main(v: Vertex) -> Varyings {
    var o: Varyings;
    o.pos = v.pos;
    o.c0 = v.c0;
    o.c1 = v.c1;
    o.t0 = v.t0;
    o.t1 = v.t1;
    o.t2 = v.t2;
    o.t3 = v.t3;
    o.t4 = v.t4;
    o.t5 = v.t5;
    o.t6 = v.t6;
    o.t7 = v.t7;
    return o;
}

struct Out {
    @location(0) @blend_src(0) color: vec4<f32>,
    @location(0) @blend_src(1) alpha: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

fn bp(i: u32) -> u32 {
    return st.bp[i >> 2u][i & 3u];
}

fn bits(v: u32, at: u32, width: u32) -> u32 {
    return (v >> at) & ((1u << width) - 1u);
}

/// A texture map's register `base` (one of the SET* bases).
fn tex_reg(base: u32, map: u32) -> u32 {
    return base + (map & 3u) + select(0u, 0x20u, map >= 4u);
}

fn load(map: u32, x: i32, y: i32, mip: i32) -> vec4<u32> {
    let c = vec2<i32>(x, y);
    switch map {
        case 0u: { return textureLoad(tex0, c, mip); }
        case 1u: { return textureLoad(tex1, c, mip); }
        case 2u: { return textureLoad(tex2, c, mip); }
        case 3u: { return textureLoad(tex3, c, mip); }
        case 4u: { return textureLoad(tex4, c, mip); }
        case 5u: { return textureLoad(tex5, c, mip); }
        case 6u: { return textureLoad(tex6, c, mip); }
        default: { return textureLoad(tex7, c, mip); }
    }
}

fn wrap(coord: i32, mode: u32, size: i32) -> i32 {
    switch mode {
        case 1u: { return coord & (size - 1); }
        case 2u: {
            var c = coord;
            if (c & size) != 0 {
                c = ~c;
            }
            return c & (size - 1);
        }
        default: { return clamp(coord, 0, size - 1); }
    }
}

fn texel(map: u32, s: i32, t: i32, mip: i32, w: i32, h: i32) -> vec4<i32> {
    let mode0 = bp(tex_reg(0x80u, map));
    let x = wrap(s, bits(mode0, 0u, 2u), w);
    let y = wrap(t, bits(mode0, 2u, 2u), h);
    let levels = max(i32(st.levels[map >> 2u][map & 3u]), 1);
    let m = min(mip, levels - 1);
    let dims = vec2<i32>(textureDimensionsOf(map, m));
    return vec4<i32>(load(map, min(x, dims.x - 1), min(y, dims.y - 1), m));
}

fn textureDimensionsOf(map: u32, mip: i32) -> vec2<u32> {
    switch map {
        case 0u: { return textureDimensions(tex0, mip); }
        case 1u: { return textureDimensions(tex1, mip); }
        case 2u: { return textureDimensions(tex2, mip); }
        case 3u: { return textureDimensions(tex3, mip); }
        case 4u: { return textureDimensions(tex4, mip); }
        case 5u: { return textureDimensions(tex5, mip); }
        case 6u: { return textureDimensions(tex6, mip); }
        default: { return textureDimensions(tex7, mip); }
    }
}

fn sample_mip(map: u32, s_in: i32, t_in: i32, mip: i32, linear: bool) -> vec4<i32> {
    let img0 = bp(tex_reg(0x88u, map));
    let w = (i32(bits(img0, 0u, 10u)) >> u32(mip)) + 1;
    let h = (i32(bits(img0, 10u, 10u)) >> u32(mip)) + 1;
    var s = s_in >> u32(mip);
    var t = t_in >> u32(mip);
    if linear {
        s -= 64;
        t -= 64;
        let is = s >> 7u;
        let it = t >> 7u;
        let fs = s & 0x7f;
        let ft = t & 0x7f;
        var sum = texel(map, is, it, mip, w, h) * ((128 - fs) * (128 - ft));
        sum += texel(map, is + 1, it, mip, w, h) * (fs * (128 - ft));
        sum += texel(map, is, it + 1, mip, w, h) * ((128 - fs) * ft);
        sum += texel(map, is + 1, it + 1, mip, w, h) * (fs * ft);
        return sum >> vec4<u32>(14u);
    }
    return texel(map, s >> 7u, t >> 7u, mip, w, h);
}

fn sample(map: u32, s: i32, t: i32, lod: i32, linear: bool) -> vec4<i32> {
    let mode0 = bp(tex_reg(0x80u, map));
    let mip_filter = bits(mode0, 5u, 2u);
    var base = 0;
    var mip_linear = false;
    let frac = lod & 0xf;
    if lod > 0 && mip_filter != 0u {
        base = lod >> 4u;
        mip_linear = frac != 0 && mip_filter == 2u;
        if mip_filter == 1u && frac >= 8 {
            base += 1;
        }
    }
    if mip_linear {
        let a = sample_mip(map, s, t, base, linear) * (16 - frac);
        let b = sample_mip(map, s, t, base + 1, linear) * frac;
        return (a + b) >> vec4<u32>(4u);
    }
    return sample_mip(map, s, t, base, linear);
}

fn fixed_log2(f: f32) -> i32 {
    let x = bitcast<u32>(f);
    let int_part = i32((x & 0x7F800000u) >> 19u) - 2032;
    let frac = i32((x & 0x007fffffu) >> 19u);
    return int_part + frac;
}

struct Lod {
    lod: i32,
    linear: bool,
}

/// A texture map's level of detail from its coordinate's change across the 2x2 block.
fn lod_of(map: u32, dx: vec2<f32>, dy: vec2<f32>) -> Lod {
    let mode0 = bp(tex_reg(0x80u, map));
    let mode1 = bp(tex_reg(0x84u, map));
    var sd: f32;
    var td: f32;
    if bits(mode0, 8u, 1u) != 0u {
        // Diagonal.
        sd = abs(dx.x) + abs(dy.x);
        td = abs(dx.y) + abs(dy.y);
    } else {
        sd = max(abs(dx.x), abs(dy.x));
        td = max(abs(dx.y), abs(dy.y));
    }
    var lod = fixed_log2(max(sd, td));
    let bias = (i32(bits(mode0, 9u, 8u)) << 24u) >> 24u;
    lod += bias >> 1u;
    var o: Lod;
    o.linear = (lod > 0 && bits(mode0, 7u, 1u) != 0u) || (lod <= 0 && bits(mode0, 4u, 1u) != 0u);
    let min_lod = i32(bits(mode1, 0u, 8u));
    let max_lod = i32(bits(mode1, 8u, 8u));
    if lod > max_lod {
        lod = max_lod;
    } else if lod < min_lod {
        lod = min_lod;
    }
    o.lod = lod;
    return o;
}

fn swap(c: vec4<i32>, table: u32) -> vec4<i32> {
    let rg = bp(0xF6u + table * 2u);
    let ba = bp(0xF7u + table * 2u);
    return vec4<i32>(c[bits(rg, 0u, 2u)], c[bits(rg, 2u, 2u)], c[bits(ba, 0u, 2u)], c[bits(ba, 2u, 2u)]);
}

fn konst(sel: u32) -> vec4<i32> {
    if sel < 8u {
        var v = array<i32, 8>(255, 223, 191, 159, 128, 96, 64, 32);
        return vec4<i32>(v[sel]);
    }
    if sel < 12u {
        return vec4<i32>(0);
    }
    if sel < 16u {
        return vec4<i32>(st.konsts[sel - 12u].rgb, 0);
    }
    let k = st.konsts[(sel - 16u) & 3u];
    return vec4<i32>(k[(sel - 16u) >> 2u]);
}

fn compare(a: i32, b: i32, func: u32) -> bool {
    switch func {
        case 0u: { return false; }
        case 1u: { return a < b; }
        case 2u: { return a == b; }
        case 3u: { return a <= b; }
        case 4u: { return a > b; }
        case 5u: { return a != b; }
        case 6u: { return a >= b; }
        default: { return true; }
    }
}

fn alpha_test(a: i32) -> bool {
    let at = bp(0xF3u);
    let c0 = compare(a, i32(bits(at, 0u, 8u)), bits(at, 16u, 3u));
    let c1 = compare(a, i32(bits(at, 8u, 8u)), bits(at, 19u, 3u));
    switch bits(at, 22u, 2u) {
        case 0u: { return c0 && c1; }
        case 1u: { return c0 || c1; }
        case 2u: { return c0 != c1; }
        default: { return c0 == c1; }
    }
}


/// One component's regular combination: (d + bias) + lerp(a, b, c), scaled.
fn combine(a: i32, b: i32, c_in: i32, d: i32, env: u32, is_alpha: bool) -> i32 {
    let scale = bits(env, 20u, 2u);
    let lshift = select(scale, 0u, scale == 3u);
    let rshift = select(0u, 1u, scale == 3u);
    let bias_sel = bits(env, 16u, 2u);
    let bias = select(select(0, -128, bias_sel == 2u), 128, bias_sel == 1u);
    let sub = bits(env, 18u, 1u) != 0u;
    let c = c_in + (c_in >> 7u);
    var temp = a * (256 - c) + b * c;
    temp <<= lshift;
    temp += select(select(128, 127, sub), 0, scale == 3u);
    if is_alpha {
        temp = select(temp >> 8u, (-temp) >> 8u, sub);
    } else {
        temp >>= 8u;
        temp = select(temp, -temp, sub);
    }
    let result = ((d + bias) << lshift) + temp;
    return result >> rshift;
}

fn compare_value(v: vec4<i32>, mode: u32, i: u32) -> u32 {
    let u = vec4<u32>(v & vec4<i32>(255));
    switch mode {
        case 0u: { return u.r; }
        case 1u: { return (u.g << 8u) | u.r; }
        case 2u: { return (u.b << 16u) | (u.g << 8u) | u.r; }
        default: { return u[i]; }
    }
}

fn color_arg(arg: u32, regs: array<vec4<i32>, 4>, tex: vec4<i32>, ras: vec4<i32>, k: vec4<i32>) -> vec3<i32> {
    switch arg {
        case 0u: { return regs[0].rgb; }
        case 1u: { return vec3<i32>(regs[0].a); }
        case 2u: { return regs[1].rgb; }
        case 3u: { return vec3<i32>(regs[1].a); }
        case 4u: { return regs[2].rgb; }
        case 5u: { return vec3<i32>(regs[2].a); }
        case 6u: { return regs[3].rgb; }
        case 7u: { return vec3<i32>(regs[3].a); }
        case 8u: { return tex.rgb; }
        case 9u: { return vec3<i32>(tex.a); }
        case 10u: { return ras.rgb; }
        case 11u: { return vec3<i32>(ras.a); }
        case 12u: { return vec3<i32>(255); }
        case 13u: { return vec3<i32>(128); }
        case 14u: { return k.rgb; }
        default: { return vec3<i32>(0); }
    }
}

fn alpha_arg(arg: u32, regs: array<vec4<i32>, 4>, tex: vec4<i32>, ras: vec4<i32>, k: vec4<i32>) -> i32 {
    switch arg {
        case 0u: { return regs[0].a; }
        case 1u: { return regs[1].a; }
        case 2u: { return regs[2].a; }
        case 3u: { return regs[3].a; }
        case 4u: { return tex.a; }
        case 5u: { return ras.a; }
        case 6u: { return k.a; }
        default: { return 0; }
    }
}

fn wrap_ind(coord: i32, mode: u32) -> i32 {
    switch mode {
        case 0u: { return coord; }
        case 1u: { return coord & ((256 << 7u) - 1); }
        case 2u: { return coord & ((128 << 7u) - 1); }
        case 3u: { return coord & ((64 << 7u) - 1); }
        case 4u: { return coord & ((32 << 7u) - 1); }
        case 5u: { return coord & ((16 << 7u) - 1); }
        default: { return 0; }
    }
}

fn sext(v: u32, width: u32) -> i32 {
    return i32(v << (32u - width)) >> (32u - width);
}

@fragment
fn fs_main(in: Varyings) -> Out {
    var out: Out;
    let ex = i32(in.pos.x);
    let ey = i32(in.pos.y);
    // Screen z from the GPU's, then as the EFB holds it.
    let screen_z = st.depth.y + st.depth.x * (in.pos.z - 1.0);
    var z = i32(clamp(screen_z, 0.0, 16777215.0));
    if st.mode.x == 1u {
        let c = st.mode.y;
        out.color = vec4<f32>(f32(c >> 24u), f32((c >> 16u) & 255u), f32((c >> 8u) & 255u), f32(c & 255u)) / 255.0;
        out.alpha = out.color;
        out.depth = f32(st.mode.z) / 16777216.0;
        return out;
    }

    let genmode = bp(0u);
    let ntex = bits(genmode, 0u, 4u);
    let nstages = bits(genmode, 10u, 4u) + 1u;
    let nind = bits(genmode, 16u, 3u);

    // Texture coordinates in texels, and their changes across the 2x2 block, in uniform control
    // flow.
    let ts = array<vec3<f32>, 8>(in.t0, in.t1, in.t2, in.t3, in.t4, in.t5, in.t6, in.t7);
    var uv: array<vec2<f32>, 8>;
    var dx: array<vec2<f32>, 8>;
    var dy: array<vec2<f32>, 8>;
    for (var i = 0u; i < 8u; i++) {
        let t = ts[i];
        var p = t.xy;
        if t.z != 0.0 {
            p = t.xy / t.z;
        }
        uv[i] = p;
        dx[i] = dpdxCoarse(p);
        dy[i] = dpdyCoarse(p);
    }
    var fixed_uv: array<vec2<i32>, 8>;
    for (var i = 0u; i < 8u; i++) {
        fixed_uv[i] = vec2<i32>(uv[i] * 128.0);
    }

    let ras0 = vec4<i32>(clamp(in.c0, vec4<f32>(0.0), vec4<f32>(255.0)));
    let ras1 = vec4<i32>(clamp(in.c1, vec4<f32>(0.0), vec4<f32>(255.0)));

    // Indirect stages' lookups.
    var ind: array<vec4<i32>, 4>;
    let iref = bp(0x27u);
    for (var i = 0u; i < nind; i++) {
        var coord = bits(iref, 6u * i + 3u, 3u);
        let map = bits(iref, 6u * i, 3u);
        let lod = lod_of(map, dx[coord], dy[coord]);
        if coord >= ntex {
            coord = 0u;
        }
        let scale = bp(0x25u + (i >> 1u));
        let odd = (i & 1u) * 8u;
        let ss = bits(scale, odd, 4u);
        let tsc = bits(scale, odd + 4u, 4u);
        ind[i] = sample(map, fixed_uv[coord].x >> ss, fixed_uv[coord].y >> tsc, lod.lod, lod.linear);
    }

    var regs = st.colors;
    var raw_tex = vec4<i32>(0);
    var tex_coord = vec2<i32>(0);
    for (var n = 0u; n < nstages; n++) {
        let order = bp(0x28u + (n >> 1u));
        let shift = (n & 1u) * 12u;
        let map = bits(order, shift, 3u);
        let coord_raw = bits(order, shift + 3u, 3u);
        let enabled = bits(order, shift + 6u, 1u) != 0u;
        let chan = bits(order, shift + 7u, 3u);
        var coord = coord_raw;
        if coord >= ntex {
            coord = 0u;
        }
        let cenv = bp(0xC0u + 2u * n);
        let aenv = bp(0xC1u + 2u * n);

        // Indirect texturing of this stage's coordinate.
        let tind = bp(0x10u + n);
        let s = fixed_uv[coord].x;
        let t = fixed_uv[coord].y;
        let indmap = ind[bits(tind, 0u, 2u)];
        var bump = 0;
        switch bits(tind, 7u, 2u) {
            case 1u: { bump = indmap.a; }
            case 2u: { bump = indmap.b; }
            case 3u: { bump = indmap.g; }
            default: {}
        }
        let fmt = bits(tind, 2u, 2u);
        let bias_value = select(1, -128, fmt == 0u);
        let fmt_shift = select(fmt + 2u, 0u, fmt == 0u);
        var ic = vec3<i32>(indmap.a >> fmt_shift, indmap.b >> fmt_shift, indmap.g >> fmt_shift);
        ic += vec3<i32>(select(0, bias_value, bits(tind, 4u, 1u) != 0u), select(0, bias_value, bits(tind, 5u, 1u) != 0u), select(0, bias_value, bits(tind, 6u, 1u) != 0u));
        switch fmt {
            case 0u: { bump = bump & 0xf8; }
            case 1u: { bump = (bump << 5u) & 0xff; }
            case 2u: { bump = (bump << 4u) & 0xff; }
            default: { bump = (bump << 3u) & 0xff; }
        }
        var trans = vec2<i32>(0);
        let mtx = bits(tind, 9u, 2u);
        if mtx != 0u {
            let base = 0x06u + (mtx - 1u) * 3u;
            let ma = bp(base);
            let mb = bp(base + 1u);
            let mc = bp(base + 2u);
            let scale = i32(bits(ma, 22u, 2u) | (bits(mb, 22u, 2u) << 2u) | (bits(mc, 22u, 1u) << 4u));
            let sh = 17 - scale;
            switch bits(tind, 11u, 2u) {
                case 1u: { trans = vec2<i32>(s * ic.x / 256, t * ic.x / 256); }
                case 2u: { trans = vec2<i32>(s * ic.y / 256, t * ic.y / 256); }
                default: {
                    trans.x = (sext(bits(ma, 0u, 11u), 11u) * ic.x + sext(bits(mb, 0u, 11u), 11u) * ic.y + sext(bits(mc, 0u, 11u), 11u) * ic.z) >> 3u;
                    trans.y = (sext(bits(ma, 11u, 11u), 11u) * ic.x + sext(bits(mb, 11u, 11u), 11u) * ic.y + sext(bits(mc, 11u, 11u), 11u) * ic.z) >> 3u;
                }
            }
            if sh >= 0 {
                trans = trans >> vec2<u32>(u32(sh));
            } else {
                trans = trans << vec2<u32>(u32(-sh));
            }
        }
        let wrapped = vec2<i32>(wrap_ind(s, bits(tind, 13u, 3u)), wrap_ind(t, bits(tind, 16u, 3u)));
        if bits(tind, 20u, 1u) != 0u {
            tex_coord += wrapped + trans;
        } else {
            tex_coord = wrapped + trans;
        }

        var tex = vec4<i32>(0);
        if enabled {
            var texel = vec4<i32>(0);
            if ntex > 0u {
                let lod = lod_of(map, dx[coord_raw], dy[coord_raw]);
                texel = sample(map, tex_coord.x, tex_coord.y, lod.lod, lod.linear);
            }
            raw_tex = texel;
            tex = swap(texel, bits(aenv, 2u, 2u));
        }

        let ksel = bp(0xF6u + (n >> 1u));
        let kc = bits(ksel, 4u + (n & 1u) * 10u, 5u);
        let ka = bits(ksel, 9u + (n & 1u) * 10u, 5u);
        let k = vec4<i32>(konst(kc).rgb, konst(ka).a);

        var ras = vec4<i32>(0);
        switch chan {
            case 0u: { ras = swap(ras0, bits(aenv, 0u, 2u)); }
            case 1u: { ras = swap(ras1, bits(aenv, 0u, 2u)); }
            case 5u: { ras = vec4<i32>(bump); }
            case 6u: { ras = vec4<i32>(bump | (bump >> 5u)); }
            default: {}
        }

        // Inputs a, b and c take 8 bits; d is signed.
        let ca = color_arg(bits(cenv, 12u, 4u), regs, tex, ras, k) & vec3<i32>(255);
        let cb = color_arg(bits(cenv, 8u, 4u), regs, tex, ras, k) & vec3<i32>(255);
        let cc = color_arg(bits(cenv, 4u, 4u), regs, tex, ras, k) & vec3<i32>(255);
        let cd = color_arg(bits(cenv, 0u, 4u), regs, tex, ras, k);
        let aa = alpha_arg(bits(aenv, 13u, 3u), regs, tex, ras, k) & 255;
        let ab = alpha_arg(bits(aenv, 10u, 3u), regs, tex, ras, k) & 255;
        let ac = alpha_arg(bits(aenv, 7u, 3u), regs, tex, ras, k) & 255;
        let ad = alpha_arg(bits(aenv, 4u, 3u), regs, tex, ras, k);

        var color: vec3<i32>;
        if bits(cenv, 16u, 2u) != 3u {
            color = vec3<i32>(combine(ca.r, cb.r, cc.r, cd.r, cenv, false), combine(ca.g, cb.g, cc.g, cd.g, cenv, false), combine(ca.b, cb.b, cc.b, cd.b, cenv, false));
        } else {
            let mode = bits(cenv, 20u, 2u);
            let gt = bits(cenv, 18u, 1u) != 0u;
            let a4 = vec4<i32>(ca, aa);
            let b4 = vec4<i32>(cb, ab);
            for (var i = 0u; i < 3u; i++) {
                let x = compare_value(a4, mode, i);
                let y = compare_value(b4, mode, i);
                let ok = select(x == y, x > y, gt);
                color[i] = cd[i] + select(0, cc[i], ok);
            }
        }
        var alpha: i32;
        if bits(aenv, 16u, 2u) != 3u {
            alpha = combine(aa, ab, ac, ad, aenv, true);
        } else {
            let mode = bits(aenv, 20u, 2u);
            let gt = bits(aenv, 18u, 1u) != 0u;
            let x = compare_value(vec4<i32>(ca, aa), mode, 3u);
            let y = compare_value(vec4<i32>(cb, ab), mode, 3u);
            let ok = select(x == y, x > y, gt);
            alpha = ad + select(0, ac, ok);
        }
        if bits(cenv, 19u, 1u) != 0u {
            color = clamp(color, vec3<i32>(0), vec3<i32>(255));
        } else {
            color = clamp(color, vec3<i32>(-1024), vec3<i32>(1023));
        }
        if bits(aenv, 19u, 1u) != 0u {
            alpha = clamp(alpha, 0, 255);
        } else {
            alpha = clamp(alpha, -1024, 1023);
        }
        let cdest = bits(cenv, 22u, 2u);
        let adest = bits(aenv, 22u, 2u);
        regs[cdest] = vec4<i32>(color, regs[cdest].a);
        regs[adest].a = alpha;
    }

    let last = nstages - 1u;
    let cdest = bits(bp(0xC0u + 2u * last), 22u, 2u);
    let adest = bits(bp(0xC1u + 2u * last), 22u, 2u);
    var output = vec4<i32>(regs[cdest].rgb, regs[adest].a) & vec4<i32>(255);

    if st.mode.x != 2u && !alpha_test(output.a) {
        discard;
    }

    // Z textures.
    let ztex2 = bp(0xF5u);
    let zop = bits(ztex2, 2u, 2u);
    if zop != 0u {
        var zt = bits(bp(0xF4u), 0u, 24u);
        let raw = vec4<u32>(raw_tex & vec4<i32>(255));
        switch bits(ztex2, 0u, 2u) {
            case 0u: { zt += raw.a; }
            case 1u: { zt += (raw.a << 8u) | raw.r; }
            default: { zt += (raw.r << 16u) | (raw.g << 8u) | raw.b; }
        }
        if zop == 1u {
            zt += u32(z);
        }
        z = i32(zt & 0x00ffffffu);
    }

    // Fog.
    let fog3 = bp(0xF1u);
    let fsel = bits(fog3, 21u, 3u);
    if fsel != 0u {
        var ze: f32;
        if bits(fog3, 20u, 1u) == 0u {
            let denom = i32(bits(bp(0xEFu), 0u, 24u)) - (z >> bits(bp(0xF0u), 0u, 5u));
            ze = (st.fog.x * 16777215.0) / f32(denom);
        } else {
            ze = st.fog.x * (f32(z) / 16777215.0);
        }
        let range = bp(0xE8u);
        if bits(range, 10u, 1u) != 0u {
            let offset = (f32(ex) - f32(i32(bits(range, 0u, 10u)) - 342)) / st.fog.z;
            let index = clamp(9.0 - abs(offset) * 9.0, 0.0, 9.0);
            let lower = i32(index);
            let upper = lower + 1;
            let kl = fog_k(lower) * 4.0;
            let ku = fog_k(upper) * 4.0;
            let factor = f32(upper) - index;
            let kk = kl * factor + ku * (1.0 - factor);
            ze *= sqrt(offset * offset + kk * kk) / kk;
        }
        ze -= st.fog.y;
        var fog = clamp(ze, 0.0, 1.0);
        switch fsel {
            case 4u: { fog = 1.0 - exp2(-8.0 * fog); }
            case 5u: { fog = 1.0 - exp2(-8.0 * fog * fog); }
            case 6u: { fog = exp2(-8.0 * (1.0 - fog)); }
            case 7u: { let f = 1.0 - fog; fog = exp2(-8.0 * f * f); }
            default: {}
        }
        let fi = i32(fog * 256.0);
        let inv = 256 - fi;
        let fc = bp(0xF2u);
        let fcol = vec3<i32>(i32(bits(fc, 16u, 8u)), i32(bits(fc, 8u, 8u)), i32(bits(fc, 0u, 8u)));
        output = vec4<i32>((output.rgb * inv + fi * fcol) >> vec3<u32>(8u), output.a);
    }

    // The EFB's format: six bits a channel, dithered, for RGBA6.
    let pe = bp(0x43u);
    let blend = bp(0x41u);
    var final_c = output;
    if bits(pe, 0u, 3u) == 1u {
        if bits(blend, 2u, 1u) != 0u && bits(blend, 0u, 1u) == 0u {
            var dither = array<i32, 4>(0, 2, 3, 1);
            let d = dither[(ey & 1) * 2 + (ex & 1)];
            final_c = vec4<i32>(((output.rgb - (output.rgb >> vec3<u32>(6u))) + d) & vec3<i32>(0xfc), output.a);
        }
        final_c = (final_c & vec4<i32>(0xfc)) | ((final_c & vec4<i32>(0xfc)) >> vec4<u32>(6u));
    }

    let dst_alpha = bp(0x42u);
    var written_a = final_c.a;
    if bits(dst_alpha, 8u, 1u) != 0u {
        written_a = i32(bits(dst_alpha, 0u, 8u));
        if bits(pe, 0u, 3u) == 1u {
            written_a = (written_a & 0xfc) | (written_a >> 6u);
        }
    }
    out.color = vec4<f32>(vec3<f32>(final_c.rgb), f32(written_a)) / 255.0;
    out.alpha = vec4<f32>(f32(output.a) / 255.0);
    out.depth = f32(z) / 16777216.0;
    return out;
}

fn fog_k(i: i32) -> f32 {
    let reg = bp(0xE9u + u32(i / 2));
    var v: u32;
    if (i % 2) != 0 {
        v = bits(reg, 0u, 12u);
    } else {
        v = bits(reg, 12u, 12u);
    }
    return f32(v) / 256.0;
}
