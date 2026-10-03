// SPDX-License-Identifier: GPL-3.0-or-later
// Command formats follow Dolphin's OpcodeDecoding and BPStructs (GPL-2.0-or-later).

//! The GameCube GPU's command stream and register state: the command processor's registers
//! (vertex formats and arrays), the transform unit's memory (XF: matrices, lights and its
//! registers), the BP registers (rasterization, TEV, textures, pixel engine), and the palettes
//! loaded into texture memory. Running commands yields events: draws, EFB copies, and what
//! signals the CPU.
//!
//! Each draw and copy can be digested: a hash of what decides its output, from the state it
//! actually uses (vertex bytes, indexed attributes from memory, the textures it samples and the
//! TEV register components its stages read), so two runs' streams compare on what they draw.

pub mod dff;
pub mod tev;
pub mod texture;
pub mod vertex;
pub mod xform;

use texture::Image;

/// The GPU's host: memory the GPU reads (display lists, vertex arrays, textures and palettes,
/// by physical address), and what sees the stream run. `command` sees each command the FIFO
/// brings (not those of display lists), whole, once it has run; a host that `draws` gets each
/// draw with its vertices fetched, and each EFB copy, with the state as it stands for it.
pub trait Memory {
    fn read(&self, phys: u32, out: &mut [u8]);
    fn command(&self, _bytes: &[u8]) {}
    fn draws(&self) -> bool {
        false
    }
    fn draw(&self, _state: &State, _draw: &Draw<'_>) {}
    fn copy(&self, _state: &State, _value: u32) {}
}

/// A draw: its primitive (the opcode's top five bits), vertex format, and vertices, each its
/// attributes' bytes in `layout` order, an indexed attribute's fetched from its array.
pub struct Draw<'a> {
    pub primitive: u8,
    pub vat: u8,
    pub count: u16,
    pub layout: &'a [vertex::Attr],
    pub data: &'a [u8],
}

/// What draws the stream: a renderer, given each draw and EFB copy with the state as it stands
/// for it and the memory the GPU reads.
pub trait Sink {
    fn draw(&mut self, state: &State, draw: &Draw<'_>, mem: &dyn Memory);
    fn copy(&mut self, state: &State, value: u32, mem: &dyn Memory);
    /// Whether it follows the state by its changes (`State::take_delta`), which the state then
    /// tracks.
    fn tracks_changes(&self) -> bool {
        false
    }
    /// Finishes what was sent, at the end of a run.
    fn finish(&mut self) {}
}

/// Registers and texture memory changed since the last `State::take_delta`: what a copy of the
/// state needs (`State::apply`) to stay the same.
#[derive(Default)]
pub struct Delta {
    pub bp: Vec<(u8, u32)>,
    pub cp: Vec<(u8, u32)>,
    pub xf: Vec<(u16, u32)>,
    /// The TEV color and konst registers (color RA, color BG, konst RA, konst BG), if any changed.
    pub tev: Option<[[u32; 4]; 4]>,
    pub tmem: Vec<(u32, Vec<u8>)>,
}

impl Delta {
    pub fn is_empty(&self) -> bool {
        self.bp.is_empty()
            && self.cp.is_empty()
            && self.xf.is_empty()
            && self.tev.is_none()
            && self.tmem.is_empty()
    }
}

/// What changed since the last delta, each register listed once.
#[derive(Default)]
struct Tracking {
    on: bool,
    bp: [u64; 4],
    bp_list: Vec<u8>,
    cp: [u64; 4],
    cp_list: Vec<u8>,
    xf: Vec<u64>,
    xf_list: Vec<u16>,
    tev: bool,
    tmem: Vec<(u32, u32)>,
}

impl Tracking {
    fn bp(&mut self, reg: usize) {
        if self.on && self.bp[reg / 64] & 1 << (reg % 64) == 0 {
            self.bp[reg / 64] |= 1 << (reg % 64);
            self.bp_list.push(reg as u8);
        }
    }

    fn cp(&mut self, reg: usize) {
        if self.on && self.cp[reg / 64] & 1 << (reg % 64) == 0 {
            self.cp[reg / 64] |= 1 << (reg % 64);
            self.cp_list.push(reg as u8);
        }
    }

    fn xf(&mut self, addr: usize) {
        if self.on && self.xf[addr / 64] & 1 << (addr % 64) == 0 {
            self.xf[addr / 64] |= 1 << (addr % 64);
            self.xf_list.push(addr as u16);
        }
    }
}

/// Whether the BP write `value` (register and value) triggers an EFB copy to the XFB, the
/// frame's picture.
pub fn is_xfb_copy(value: u32) -> bool {
    (value >> 24) as usize == TRIGGER_EFB_COPY && value & (1 << 14) != 0
}

pub(crate) fn bits(v: u32, at: u32, width: u32) -> u32 {
    (v >> at) & ((1 << width) - 1)
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// BP registers.
pub const GENMODE: usize = 0x00;
pub const TREF: usize = 0x28;
pub const SETDRAWDONE: usize = 0x45;
pub const PE_TOKEN: usize = 0x47;
pub const PE_TOKEN_INT: usize = 0x48;
pub const TRIGGER_EFB_COPY: usize = 0x52;
pub const LOADTLUT0: usize = 0x64;
pub const LOADTLUT1: usize = 0x65;
pub const TEV_COLOR: usize = 0xE0;
pub const BP_MASK: usize = 0xFE;

/// XF memory: matrices and lights below 0x1000, registers from 0x1000.
pub const XF_SIZE: usize = 0x1058;
/// Texture memory.
pub const TMEM_SIZE: usize = 1 << 20;

/// BP registers that trigger something rather than hold state a draw uses: they stay out of
/// the state hash (the copy's registers go into the copy's own digest).
fn trigger(reg: usize) -> bool {
    matches!(reg, 0x23 | 0x24 | 0x45..=0x48 | 0x52 | 0x55..=0x57 | 0x63..=0x67 | 0xFE)
        || (TEV_COLOR..TEV_COLOR + 8).contains(&reg)
}

/// What a command does outside the GPU's own state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A primitive: its kind (the opcode's top five bits), vertex format and vertex count.
    Draw { primitive: u8, vat: u8, count: u16 },
    /// An EFB copy, with the trigger's value.
    Copy(u32),
    /// `PE_DONE`: the frame is drawn.
    Finish,
    /// `PE_TOKEN` or `PE_TOKEN_INT`.
    Token { token: u16, interrupt: bool },
}

/// Hashes of 64 bits for the state hashes: splitmix64.
fn mix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

const FNV_OFFSET: u64 = 0xCBF2_9CE4_8422_2325;

/// FNV-1a over bytes, from `h`.
pub fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

/// A set of registers' hash that one write updates in constant time: the XOR of each
/// register's hash with its value.
#[derive(Clone, Copy, Default)]
struct SetHash(u64);

impl SetHash {
    fn entry(space: u64, reg: usize, value: u32) -> u64 {
        mix((space << 48) ^ ((reg as u64) << 32) ^ u64::from(value))
    }

    fn update(&mut self, space: u64, reg: usize, old: u32, new: u32) {
        self.0 ^= Self::entry(space, reg, old) ^ Self::entry(space, reg, new);
    }
}

/// The GPU's state.
pub struct State {
    pub bp: [u32; 256],
    bp_mask: u32,
    /// TEV color registers (PREV, C0, C1, C2) and konst registers, as RA and BG halves: BP
    /// E0-E7 writes one or the other by the value's type bit.
    pub color_ra: [u32; 4],
    pub color_bg: [u32; 4],
    pub konst_ra: [u32; 4],
    pub konst_bg: [u32; 4],
    pub cp: [u32; 256],
    pub xf: Vec<u32>,
    pub tmem: Vec<u8>,
    bp_hash: SetHash,
    cp_hash: SetHash,
    xf_hash: SetHash,
    /// Bytes of a command the stream hasn't finished.
    buf: Vec<u8>,
    /// Display lists being run.
    depth: u32,
    tracking: std::cell::RefCell<Tracking>,
    fetched: Vec<u8>,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

/// What decides a draw's or copy's output, hashed by part, to tell two runs' apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Digest {
    pub bp: u64,
    pub cp: u64,
    pub xf: u64,
    pub tev: u64,
    pub vertices: u64,
    pub textures: u64,
}

impl Digest {
    pub fn total(&self) -> u64 {
        let parts = [
            self.bp,
            self.cp,
            self.xf,
            self.tev,
            self.vertices,
            self.textures,
        ];
        parts
            .iter()
            .fold(FNV_OFFSET, |h, p| fnv(h, &p.to_le_bytes()))
    }
}

/// Something the stream produced, with its digest when digests are asked for.
pub struct Produced {
    pub event: Event,
    pub digest: Option<Digest>,
}

impl State {
    pub fn new() -> Self {
        let mut s = State {
            bp: [0; 256],
            bp_mask: 0x00FF_FFFF,
            color_ra: [0; 4],
            color_bg: [0; 4],
            konst_ra: [0; 4],
            konst_bg: [0; 4],
            cp: [0; 256],
            xf: vec![0; XF_SIZE],
            tmem: vec![0; TMEM_SIZE],
            bp_hash: SetHash::default(),
            cp_hash: SetHash::default(),
            xf_hash: SetHash::default(),
            buf: Vec::new(),
            depth: 0,
            tracking: Default::default(),
            fetched: Vec::new(),
        };
        // The hashes start as those of all-zero registers.
        for reg in 0..256 {
            if !trigger(reg) {
                s.bp_hash.0 ^= SetHash::entry(1, reg, 0);
            }
            s.cp_hash.0 ^= SetHash::entry(2, reg, 0);
        }
        for addr in 0..XF_SIZE {
            s.xf_hash.0 ^= SetHash::entry(3, addr, 0);
        }
        s
    }

    /// The state a FIFO log starts from: its registers and memories, as if each register had
    /// been written last (so a TEV register holds its color or its konst, by the type bit).
    pub fn load(bp: &[u32], cp: &[u32], xf: &[u32], tmem: &[u8]) -> Self {
        let mut s = State::new();
        for (reg, &v) in bp.iter().enumerate().take(256) {
            s.bp_hash
                .update(1, reg, 0, if trigger(reg) { 0 } else { v });
            s.bp[reg] = v;
            if (TEV_COLOR..TEV_COLOR + 8).contains(&reg) {
                s.set_tev_color(reg, v);
            }
        }
        for (reg, &v) in cp.iter().enumerate().take(256) {
            s.cp_hash.update(2, reg, 0, v);
            s.cp[reg] = v;
        }
        for (addr, &v) in xf.iter().enumerate().take(XF_SIZE) {
            s.xf_hash.update(3, addr, 0, v);
            s.xf[addr] = v;
        }
        let n = tmem.len().min(TMEM_SIZE);
        s.tmem[..n].copy_from_slice(&tmem[..n]);
        s
    }

    /// Starts tracking changes for `take_delta`, the first delta holding all of the state.
    pub fn track_changes(&mut self) {
        let t = self.tracking.get_mut();
        t.on = true;
        t.xf = vec![0; XF_SIZE.div_ceil(64)];
        for reg in 0..256 {
            t.bp(reg);
            t.cp(reg);
        }
        for addr in 0..XF_SIZE {
            t.xf(addr);
        }
        t.tev = true;
        t.tmem.push((0, TMEM_SIZE as u32));
    }

    /// What changed since the last call, with `track_changes`.
    pub fn take_delta(&self) -> Delta {
        let mut t = self.tracking.borrow_mut();
        let t = &mut *t;
        for &r in &t.bp_list {
            t.bp[r as usize / 64] = 0;
        }
        for &r in &t.cp_list {
            t.cp[r as usize / 64] = 0;
        }
        for &a in &t.xf_list {
            t.xf[a as usize / 64] = 0;
        }
        Delta {
            bp: t.bp_list.drain(..).map(|r| (r, self.bp[r as usize])).collect(),
            cp: t.cp_list.drain(..).map(|r| (r, self.cp[r as usize])).collect(),
            xf: t.xf_list.drain(..).map(|a| (a, self.xf[a as usize])).collect(),
            tev: std::mem::take(&mut t.tev)
                .then_some([self.color_ra, self.color_bg, self.konst_ra, self.konst_bg]),
            tmem: t
                .tmem
                .drain(..)
                .map(|(at, len)| (at, self.tmem[at as usize..(at + len) as usize].to_vec()))
                .collect(),
        }
    }

    /// Makes the changes in `delta`, as another state took them.
    pub fn apply(&mut self, delta: &Delta) {
        for &(reg, v) in &delta.bp {
            self.bp[reg as usize] = v;
        }
        for &(reg, v) in &delta.cp {
            self.cp[reg as usize] = v;
        }
        for &(addr, v) in &delta.xf {
            self.xf[addr as usize] = v;
        }
        if let Some([cra, cbg, kra, kbg]) = delta.tev {
            (self.color_ra, self.color_bg, self.konst_ra, self.konst_bg) = (cra, cbg, kra, kbg);
        }
        for (at, bytes) in &delta.tmem {
            self.tmem[*at as usize..*at as usize + bytes.len()].copy_from_slice(bytes);
        }
    }

    fn set_tev_color(&mut self, reg: usize, new: u32) {
        self.tracking.get_mut().tev = true;
        let i = (reg - TEV_COLOR) / 2;
        let konst = new & (1 << 23) != 0;
        let field = new & 0x7F_F7FF;
        match (reg % 2 == 0, konst) {
            (true, false) => self.color_ra[i] = field,
            (false, false) => self.color_bg[i] = field,
            (true, true) => self.konst_ra[i] = field,
            (false, true) => self.konst_bg[i] = field,
        }
    }

    fn write_bp(&mut self, value: u32, mem: &dyn Memory, out: &mut Vec<Produced>, digests: bool) {
        let reg = (value >> 24) as usize;
        let value = value & 0x00FF_FFFF;
        let old = self.bp[reg];
        let new = if reg == BP_MASK {
            value
        } else {
            (old & !self.bp_mask) | (value & self.bp_mask)
        };
        if reg != BP_MASK {
            self.bp_mask = 0x00FF_FFFF;
        } else {
            self.bp_mask = value;
        }
        self.bp[reg] = new;
        self.tracking.get_mut().bp(reg);
        if !trigger(reg) {
            self.bp_hash.update(1, reg, old, new);
        }
        match reg {
            SETDRAWDONE if new & 0xFF == 2 => out.push(Produced {
                event: Event::Finish,
                digest: None,
            }),
            PE_TOKEN | PE_TOKEN_INT => out.push(Produced {
                event: Event::Token {
                    token: new as u16,
                    interrupt: reg == PE_TOKEN_INT,
                },
                digest: None,
            }),
            TRIGGER_EFB_COPY => {
                mem.copy(self, new);
                let digest = digests.then(|| self.copy_digest(new));
                out.push(Produced {
                    event: Event::Copy(new),
                    digest,
                });
            }
            LOADTLUT1 => self.load_tlut(mem),
            0xE0..=0xE7 => self.set_tev_color(reg, new),
            _ => {}
        }
    }

    /// A palette load: lines of 32 bytes from memory into texture memory.
    fn load_tlut(&mut self, mem: &dyn Memory) {
        let src = (self.bp[LOADTLUT0] & 0x001F_FFFF) << 5;
        let dest = ((self.bp[LOADTLUT1] & 0x3FF) << 9) as usize;
        let len = (bits(self.bp[LOADTLUT1], 10, 11) * 32) as usize;
        let end = (dest + len).min(TMEM_SIZE);
        if dest < end {
            mem.read(src, &mut self.tmem[dest..end]);
            let t = self.tracking.get_mut();
            if t.on {
                t.tmem.push((dest as u32, (end - dest) as u32));
            }
        }
    }

    fn write_cp(&mut self, reg: u8, value: u32) {
        let old = std::mem::replace(&mut self.cp[reg as usize], value);
        self.tracking.get_mut().cp(reg as usize);
        self.cp_hash.update(2, reg as usize, old, value);
    }

    fn write_xf(&mut self, addr: usize, value: u32) {
        if addr < XF_SIZE {
            let old = std::mem::replace(&mut self.xf[addr], value);
            self.tracking.get_mut().xf(addr);
            self.xf_hash.update(3, addr, old, value);
        }
    }

    /// Feeds bytes from the FIFO and runs every command they complete, appending what they
    /// produce to `out`, digested if `digests`.
    pub fn feed(&mut self, mem: &dyn Memory, data: &[u8], out: &mut Vec<Produced>, digests: bool) {
        self.buf.extend_from_slice(data);
        let mut buf = std::mem::take(&mut self.buf);
        let used = self.run(mem, &buf, out, digests);
        buf.drain(..used);
        self.buf = buf;
    }

    /// Runs the complete commands at the start of `data`; returns the bytes they used.
    pub fn run(
        &mut self,
        mem: &dyn Memory,
        data: &[u8],
        out: &mut Vec<Produced>,
        digests: bool,
    ) -> usize {
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
                    self.write_cp(rest[1], be32(&rest[2..]));
                    6
                }
                0x10 => {
                    if rest.len() < 5 {
                        break;
                    }
                    let head = be32(&rest[1..]);
                    let count = bits(head, 16, 4) as usize + 1;
                    if rest.len() < 5 + 4 * count {
                        break;
                    }
                    let addr = (head & 0xFFFF) as usize;
                    for i in 0..count {
                        self.write_xf(addr + i, be32(&rest[5 + 4 * i..]));
                    }
                    5 + 4 * count
                }
                0x20 | 0x28 | 0x30 | 0x38 => {
                    if rest.len() < 5 {
                        break;
                    }
                    self.indexed_xf_load(op, be32(&rest[1..]), mem);
                    5
                }
                0x40 => {
                    if rest.len() < 9 {
                        break;
                    }
                    let (addr, size) = (be32(&rest[1..]), be32(&rest[5..]));
                    let mut list = vec![0; size as usize];
                    mem.read(addr & 0x03FF_FFFF, &mut list);
                    // A display list holds whole commands; the FIFO's parse state is untouched.
                    let saved = std::mem::take(&mut self.buf);
                    self.depth += 1;
                    let used = self.run(mem, &list, out, digests);
                    self.depth -= 1;
                    self.buf = saved;
                    assert!(
                        list[used..].iter().all(|&b| b == 0),
                        "display list at {addr:#010X} ends in the middle of a command"
                    );
                    9
                }
                0x61 => {
                    if rest.len() < 5 {
                        break;
                    }
                    self.write_bp(be32(&rest[1..]), mem, out, digests);
                    5
                }
                0x80..=0xBF => {
                    if rest.len() < 3 {
                        break;
                    }
                    let vat = usize::from(op & 7);
                    let count = u16::from_be_bytes([rest[1], rest[2]]);
                    let layout = vertex::layout(&self.cp, vat);
                    let len = 3 + (u32::from(count) * vertex::stream_size(&layout)) as usize;
                    if rest.len() < len {
                        break;
                    }
                    let draws = mem.draws();
                    // The vertices fetched, in a buffer kept from draw to draw.
                    let mut bytes = std::mem::take(&mut self.fetched);
                    bytes.clear();
                    if digests || draws {
                        vertex::fetch(
                            &self.cp,
                            &layout,
                            u32::from(count),
                            &rest[3..],
                            mem,
                            &mut bytes,
                        );
                    }
                    if draws {
                        let draw = Draw {
                            primitive: op & 0xF8,
                            vat: op & 7,
                            count,
                            layout: &layout,
                            data: &bytes,
                        };
                        mem.draw(self, &draw);
                    }
                    let digest = digests.then(|| self.draw_digest(op, &bytes, mem));
                    self.fetched = bytes;
                    out.push(Produced {
                        event: Event::Draw {
                            primitive: op & 0xF8,
                            vat: op & 7,
                            count,
                        },
                        digest,
                    });
                    len
                }
                _ => panic!(
                    "unknown GX command {op:#04X}; next bytes {:02X?}",
                    &rest[..rest.len().min(16)]
                ),
            };
            if self.depth == 0 {
                mem.command(&rest[..len]);
            }
            at += len;
        }
        at
    }

    /// `LOAD_INDX_A`-`D`: XF words from arrays 12-15, as a matrix load by index.
    fn indexed_xf_load(&mut self, op: u8, value: u32, mem: &dyn Memory) {
        let array = vertex::ARRAY_XF_A + usize::from((op - 0x20) / 8);
        let addr = (value & 0xFFF) as usize;
        let count = bits(value, 12, 4) as usize + 1;
        let index = value >> 16;
        let base = self.cp[vertex::ARRAY_BASE as usize + array] & 0x03FF_FFFF;
        let stride = self.cp[vertex::ARRAY_STRIDE as usize + array] & 0xFF;
        let mut words = vec![0u8; 4 * count];
        mem.read(base + index * stride, &mut words);
        for i in 0..count {
            self.write_xf(addr + i, be32(&words[4 * i..]));
        }
    }

    /// The texture maps the active stages sample.
    pub fn textures_used(&self) -> Vec<usize> {
        let stages = bits(self.bp[GENMODE], 10, 4) as usize + 1;
        let mut maps = Vec::new();
        for s in 0..stages {
            let order = self.bp[TREF + s / 2] >> (12 * (s % 2) as u32);
            if bits(order, 6, 1) != 0 {
                let map = bits(order, 0, 3) as usize;
                if !maps.contains(&map) {
                    maps.push(map);
                }
            }
        }
        maps
    }

    fn tev_digest(&self) -> u64 {
        let usage = tev::usage(&self.bp);
        let mut h = FNV_OFFSET;
        for i in 0..4 {
            let (ra, bg) = tev::masked(self.color_ra[i], self.color_bg[i], usage.color[i]);
            let (kra, kbg) = tev::masked(self.konst_ra[i], self.konst_bg[i], usage.konst[i]);
            for v in [ra, bg, kra, kbg] {
                h = fnv(h, &v.to_le_bytes());
            }
        }
        h
    }

    fn textures_digest(&self, mem: &dyn Memory) -> u64 {
        let mut h = FNV_OFFSET;
        for map in self.textures_used() {
            let image = Image::of(&self.bp, map);
            let mut data = vec![0; image.size() as usize];
            mem.read(image.address, &mut data);
            h = fnv(h, &data);
            if let Some(entries) = texture::palette_entries(image.format) {
                let tlut = self.bp[texture::reg(texture::SETTLUT, map)];
                let at = ((tlut & 0x3FF) << 9) as usize;
                let end = (at + 2 * entries as usize).min(TMEM_SIZE);
                h = fnv(h, &self.tmem[at..end]);
                // The palette's format: IA8, RGB565 or RGB5A3.
                h = fnv(h, &[bits(tlut, 10, 2) as u8]);
            }
        }
        h
    }

    fn draw_digest(&self, op: u8, vertices: &[u8], mem: &dyn Memory) -> Digest {
        Digest {
            bp: self.bp_hash.0,
            cp: self.cp_hash.0,
            xf: self.xf_hash.0,
            tev: self.tev_digest(),
            vertices: fnv(fnv(FNV_OFFSET, &[op]), vertices),
            textures: self.textures_digest(mem),
        }
    }

    fn copy_digest(&self, value: u32) -> Digest {
        // The copy's own registers: source rectangle, destination, stride, scale, clear
        // values, filters, and the trigger.
        let regs = [
            0x01, 0x02, 0x03, 0x04, 0x49, 0x4A, 0x4B, 0x4D, 0x4E, 0x4F, 0x50, 0x51, 0x53, 0x54,
        ];
        let mut h = FNV_OFFSET;
        for r in regs {
            h = fnv(h, &self.bp[r].to_le_bytes());
        }
        Digest {
            bp: self.bp_hash.0,
            cp: 0,
            xf: 0,
            tev: 0,
            vertices: fnv(h, &value.to_le_bytes()),
            textures: 0,
        }
    }
}
