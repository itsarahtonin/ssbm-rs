// SPDX-License-Identifier: GPL-3.0-or-later
// Follows Dolphin's DSP HLE of the AX microcode (AX.cpp, AXVoice.h) and its DSP accelerator
// (DSPAccelerator.cpp), GPL-2.0-or-later.

//! The AX microcode, the DSP program that mixes the game's sound: each 5 ms frame the CPU sends it
//! a command list; it reads each voice's parameter block (PB) from main memory, decodes its
//! samples from ARAM (ADPCM or PCM, through the DSP's accelerator), resamples them, applies the
//! volume envelope and mixes them into its main and auxiliary buses, which it then hands back to
//! main memory for the CPU's effects and for the audio interface.

mod accelerator;
pub mod coefs;
mod pb;

use accelerator::Accelerator;
pub use pb::Pb;

/// What the DSP reads and writes: main memory, by physical address, and ARAM.
pub trait Bus {
    fn read(&self, addr: u32, out: &mut [u8]);
    fn write(&mut self, addr: u32, data: &[u8]);
    fn aram(&self, addr: u32) -> u8;
}

/// The early AX microcode (Melee's, and many games' of its time): its PBs have no low-pass filter
/// and it reads mixer control bits its own way.
pub const CRC_EARLY: u32 = 0x4E8A_8B21;

/// Dolphin's hash of a microcode, by which it tells them apart.
pub fn ucode_crc(ucode: &[u8]) -> u32 {
    ucode
        .iter()
        .fold(0u32, |crc, &b| (crc ^ u32::from(b)).rotate_left(3))
}

const SAMPLES: usize = 5 * 32;

/// Which buses a voice mixes into, and which of those ramp their volume.
mod mix {
    pub const MAIN_L: u32 = 0x000001;
    pub const MAIN_R: u32 = 0x000004;
    pub const MAIN_S: u32 = 0x000010;
    pub const AUXA_L: u32 = 0x000040;
    pub const AUXA_R: u32 = 0x000100;
    pub const AUXA_S: u32 = 0x000400;
    pub const AUXB_L: u32 = 0x001000;
    pub const AUXB_R: u32 = 0x004000;
    pub const AUXB_S: u32 = 0x010000;
    pub const ALL_RAMPS: u32 = 0xAAAAAA;
}

/// The buses, in Dolphin's order: main, aux A and aux B, each left, right and surround.
const MAIN_L: usize = 0;
const MAIN_R: usize = 1;
const MAIN_S: usize = 2;
const AUXA_L: usize = 3;
const AUXB_L: usize = 6;
const AUXB_R: usize = 7;
const AUXB_S: usize = 8;

/// The microcode's state between command lists.
pub struct Ax {
    crc: u32,
    /// The polyphase resampler's coefficients, when there are any (else it's linear).
    coeffs: Option<Vec<i16>>,
    buffers: [[i32; SAMPLES]; 9],
    compressor_pos: u32,
}

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

fn be32(b: &[u8], at: usize) -> i32 {
    i32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn clamp_s16(v: i64) -> i16 {
    v.clamp(-0x8000, 0x7FFF) as i16
}

fn hilo(hi: u16, lo: u16) -> u32 {
    u32::from(hi) << 16 | u32::from(lo)
}

impl Ax {
    /// The microcode with hash `crc`, resampling with `coeffs` (0x800 of them) or linearly.
    pub fn new(crc: u32, coeffs: Option<Vec<i16>>) -> Self {
        Ax {
            crc,
            coeffs,
            buffers: [[0; SAMPLES]; 9],
            compressor_pos: 0,
        }
    }

    pub fn crc(&self) -> u32 {
        self.crc
    }

    fn read_words(bus: &dyn Bus, addr: u32, n: usize) -> Vec<u16> {
        let mut b = vec![0; 2 * n];
        bus.read(addr, &mut b);
        (0..n).map(|i| be16(&b, 2 * i)).collect()
    }

    fn read_samples(bus: &dyn Bus, addr: u32, n: usize) -> Vec<i32> {
        let mut b = vec![0; 4 * n];
        bus.read(addr, &mut b);
        (0..n).map(|i| be32(&b, 4 * i)).collect()
    }

    fn write_samples(bus: &mut dyn Bus, addr: u32, samples: &[i32]) {
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_be_bytes()).collect();
        bus.write(addr, &bytes);
    }

    /// Runs the command list of `size` words at `addr`.
    pub fn run(&mut self, bus: &mut dyn Bus, addr: u32, size: u16) {
        let mut list = Self::read_words(bus, addr, usize::from(size));
        let mut at = 0usize;
        let mut pb_addr = 0;
        loop {
            let mut next = || {
                let v = list.get(at).copied().unwrap_or(0);
                at += 1;
                v
            };
            let cmd = next();
            match cmd {
                0x00 => {
                    let a = hilo(next(), next());
                    self.setup(bus, a);
                }
                0x01 => {
                    let a = hilo(next(), next());
                    let vols = [next(), next(), next()];
                    self.download_and_mix(bus, a, vols);
                }
                0x02 => pb_addr = hilo(next(), next()),
                0x03 => self.process_pbs(bus, pb_addr),
                0x04 | 0x05 => {
                    let write = hilo(next(), next());
                    let read = hilo(next(), next());
                    self.mix_aux(bus, usize::from(cmd - 0x04), write, read);
                }
                0x06 => {
                    let a = hilo(next(), next());
                    self.upload_lrs(bus, a);
                }
                0x07 => {
                    let a = hilo(next(), next());
                    self.set_main_lr(bus, a);
                }
                0x08 => at += 10,
                0x09 => {
                    let read = hilo(next(), next());
                    self.mix_aux(bus, 1, 0, read);
                }
                0x0A..=0x0C => {}
                0x0D => {
                    let a = hilo(next(), next());
                    let n = next();
                    list = Self::read_words(bus, a, usize::from(n));
                    at = 0;
                }
                0x0E => {
                    let surround = hilo(next(), next());
                    let lr = hilo(next(), next());
                    self.output(bus, lr, surround);
                }
                0x10 => {
                    let up = hilo(next(), next());
                    let down = hilo(next(), next());
                    self.mix_auxb_lr(bus, up, down);
                }
                0x11 => {
                    let a = hilo(next(), next());
                    self.set_opposite_lr(bus, a);
                }
                0x12 => {
                    let threshold = next();
                    let frames = next();
                    let table = hilo(next(), next());
                    self.compressor(bus, threshold, u32::from(frames), table);
                }
                0x13 => {
                    let a: Vec<u32> = (0..6).map(|_| hilo(next(), next())).collect();
                    self.send_aux_and_mix(bus, &a);
                }
                // 0x0F ends the list, as does anything unknown.
                _ => break,
            }
        }
    }

    fn setup(&mut self, bus: &dyn Bus, addr: u32) {
        let init = Self::read_words(bus, addr, 27);
        for (i, buf) in self.buffers.iter_mut().enumerate() {
            let value = (u32::from(init[3 * i]) << 16 | u32::from(init[3 * i + 1])) as i32;
            let delta = init[3 * i + 2] as i16;
            for (j, s) in buf.iter_mut().enumerate() {
                *s = if value == 0 {
                    0
                } else {
                    value.wrapping_add(j as i32 * i32::from(delta))
                };
            }
        }
    }

    fn download_and_mix(&mut self, bus: &dyn Bus, addr: u32, vols: [u16; 3]) {
        // Three groups (main, aux A, aux B) of three buses, all from the same samples.
        for (group, &vol) in vols.iter().enumerate() {
            let samples = Self::read_samples(bus, addr, 3 * SAMPLES);
            for ch in 0..3 {
                for k in 0..SAMPLES {
                    let s = (i64::from(samples[ch * SAMPLES + k]) * i64::from(vol)) >> 15;
                    let b = &mut self.buffers[group * 3 + ch][k];
                    *b = b.wrapping_add(s as i32);
                }
            }
        }
    }

    fn mix_control(&self, mixer_control: u16) -> u32 {
        let mc = u32::from(mixer_control);
        let mut r = 0;
        if self.crc == CRC_EARLY {
            if mc & 0x10 != 0 {
                // Dolby Pro Logic II.
                r |= mix::MAIN_L | mix::MAIN_R;
                if mc & 0x6 == 0 {
                    r |= mix::AUXB_L | mix::AUXB_R;
                }
                if mc & 0x7 == 1 {
                    r |= mix::AUXA_L | mix::AUXA_R | mix::AUXA_S;
                }
            } else {
                r |= mix::MAIN_L | mix::MAIN_R;
                if mc & 1 != 0 {
                    r |= mix::AUXA_L | mix::AUXA_R;
                }
                if mc & 2 != 0 {
                    r |= mix::AUXB_L | mix::AUXB_R;
                }
                if mc & 4 != 0 {
                    r |= mix::MAIN_S;
                    if r & mix::AUXA_L != 0 {
                        r |= mix::AUXA_S;
                    }
                    if r & mix::AUXB_L != 0 {
                        r |= mix::AUXB_S;
                    }
                }
            }
            if mc & 8 != 0 {
                r |= mix::ALL_RAMPS;
            }
        } else {
            // Later microcodes: a bit per bus and ramp.
            let bits = [
                (0x0001, mix::MAIN_L),
                (0x0002, mix::MAIN_R),
                (0x0004, mix::MAIN_S),
                (0x0008, 0x2 | 0x8 | 0x20),
                (0x0010, mix::AUXA_L),
                (0x0020, mix::AUXA_R),
                (0x0040, 0x80 | 0x200),
                (0x0080, mix::AUXA_S),
                (0x0100, 0x800),
                (0x0200, mix::AUXB_L),
                (0x0400, mix::AUXB_R),
                (0x0800, 0x2000 | 0x8000),
                (0x1000, mix::AUXB_S),
                (0x2000, 0x20000),
            ];
            for (bit, flags) in bits {
                if mc & bit != 0 {
                    r |= flags;
                }
            }
        }
        r
    }

    fn process_pbs(&mut self, bus: &mut dyn Bus, mut addr: u32) {
        let has_lpf = self.crc != CRC_EARLY;
        while addr != 0 {
            let mut pb = Pb::read(bus, addr, has_lpf);
            let updates = Self::read_words(
                bus,
                hilo(pb.w[pb::UPDATES_DATA], pb.w[pb::UPDATES_DATA + 1]),
                64,
            );
            for ms in 0..5 {
                pb.apply_updates(ms, &updates);
                let mctrl = self.mix_control(pb.w[pb::MIXER_CONTROL]);
                self.voice(bus, &mut pb, ms * 32, mctrl);
            }
            pb.write(bus, addr, has_lpf);
            addr = hilo(pb.w[pb::NEXT_PB], pb.w[pb::NEXT_PB + 1]);
        }
    }

    /// One millisecond (32 samples) of a voice, into the buses from sample `at`.
    fn voice(&mut self, bus: &dyn Bus, pb: &mut Pb, at: usize, mctrl: u32) {
        const N: usize = 32;
        if pb.w[pb::RUNNING] != 1 {
            return;
        }
        let mut samples = [0i16; N];
        self.input_samples(bus, pb, &mut samples);
        // The volume envelope, signed on the GameCube.
        for s in &mut samples {
            let volume = i32::from(pb.w[pb::VOL_ENV] as i16);
            *s = clamp_s16(i64::from((i32::from(*s) * volume) >> 15));
            pb.w[pb::VOL_ENV] = pb.w[pb::VOL_ENV].wrapping_add(pb.w[pb::VOL_ENV + 1]);
        }
        if pb.w[pb::LPF] != 0 {
            let (a0, b0) = (
                i32::from(pb.w[pb::LPF + 2]),
                i32::from(pb.w[pb::LPF + 3] as i16),
            );
            for s in &mut samples {
                let yn1 = i32::from(pb.w[pb::LPF + 1] as i16);
                let v = clamp_s16(i64::from((a0 * i32::from(*s) + b0 * yn1) >> 15));
                *s = v;
                pb.w[pb::LPF + 1] = v as u16;
            }
        }
        // (bus flag, buffer, mixer volume word, dpop word)
        let targets = [
            (mix::MAIN_L, MAIN_L, pb::MIX_MAIN_L, pb::DPOP_MAIN_L),
            (mix::MAIN_R, MAIN_R, pb::MIX_MAIN_R, pb::DPOP_MAIN_R),
            (mix::MAIN_S, MAIN_S, pb::MIX_MAIN_S, pb::DPOP_MAIN_S),
            (mix::AUXA_L, AUXA_L, pb::MIX_AUXA_L, pb::DPOP_AUXA_L),
            (mix::AUXA_R, AUXA_L + 1, pb::MIX_AUXA_R, pb::DPOP_AUXA_R),
            (mix::AUXA_S, AUXA_L + 2, pb::MIX_AUXA_S, pb::DPOP_AUXA_S),
            (mix::AUXB_L, AUXB_L, pb::MIX_AUXB_L, pb::DPOP_AUXB_L),
            (mix::AUXB_R, AUXB_R, pb::MIX_AUXB_R, pb::DPOP_AUXB_R),
            (mix::AUXB_S, AUXB_S, pb::MIX_AUXB_S, pb::DPOP_AUXB_S),
        ];
        for (flag, buf, vol, dpop) in targets {
            if mctrl & flag == 0 {
                continue;
            }
            let ramp = mctrl & (flag << 1) != 0;
            let delta = if ramp { pb.w[vol + 1] } else { 0 };
            let out = &mut self.buffers[buf][at..at + N];
            for (o, &s) in out.iter_mut().zip(&samples) {
                let v = clamp_s16((i64::from(s) * i64::from(pb.w[vol])) >> 15);
                *o = o.wrapping_add(i32::from(v));
                pb.w[vol] = pb.w[vol].wrapping_add(delta);
                pb.w[dpop] = v as u16;
            }
        }
    }

    /// A voice's samples for one millisecond, read through the accelerator and resampled.
    fn input_samples(&self, bus: &dyn Bus, pb: &mut Pb, out: &mut [i16; 32]) {
        let mut acc = Accelerator::setup(pb);
        let coeffs = self
            .coeffs
            .as_deref()
            .map(|c| &c[usize::from(pb.w[pb::COEF_SELECT]) * 0x200..]);
        let ratio = hilo(pb.w[pb::SRC_RATIO], pb.w[pb::SRC_RATIO + 1]);
        let mut pos = u32::from(pb.w[pb::SRC_FRAC]);
        let src_type = pb.w[pb::SRC_TYPE];
        let mut last = [0i16; 4];
        for (k, l) in last.iter_mut().enumerate() {
            *l = pb.w[pb::SRC_LAST + k] as i16;
        }
        let mut read = |pb: &mut Pb| acc.read_sample(bus, pb) as i16;
        if let (Some(c), 0) = (coeffs, src_type) {
            let mut temp = last;
            let mut idx = 4u32;
            for o in out.iter_mut() {
                pos = pos.wrapping_add(ratio);
                while pos >= 0x10000 {
                    temp[(idx & 3) as usize] = read(pb);
                    idx = idx.wrapping_add(1);
                    pos -= 0x10000;
                }
                let frac = usize::from((((pos & 0xFFFF) >> 9) << 2) as u16);
                let mut acc_sum = 0i64;
                for k in 0..4 {
                    acc_sum += i64::from(temp[(idx & 3) as usize]) * i64::from(c[frac + k]);
                    idx = idx.wrapping_add(1);
                }
                *o = (acc_sum >> 15).clamp(-0x8000, 0x7FFF) as i16;
            }
            for k in (0..4).rev() {
                idx = idx.wrapping_sub(1);
                last[k] = temp[(idx & 3) as usize];
            }
        } else if src_type == 0 || src_type == 1 {
            let mut temp = last;
            let mut idx = 4u32;
            for o in out.iter_mut() {
                pos = pos.wrapping_add(ratio);
                while pos >= 0x10000 {
                    temp[(idx & 3) as usize] = read(pb);
                    idx = idx.wrapping_add(1);
                    pos -= 0x10000;
                }
                let frac = (pos & 0xFFFF) as u16;
                let inv = frac.wrapping_neg();
                if frac != 0 {
                    let s0 = i32::from(temp[(idx & 3) as usize]);
                    let s1 = i32::from(temp[((idx + 1) & 3) as usize]);
                    *o = ((i64::from(s0) * i64::from(inv) + i64::from(s1) * i64::from(frac)) >> 16)
                        as i16;
                    idx = idx.wrapping_add(4);
                } else {
                    *o = temp[(idx & 3) as usize];
                    idx = idx.wrapping_add(4);
                }
            }
            for k in (0..4).rev() {
                idx = idx.wrapping_sub(1);
                last[k] = temp[(idx & 3) as usize];
            }
        } else {
            for o in out.iter_mut() {
                *o = read(pb);
            }
            last.copy_from_slice(&out[out.len() - 4..]);
        }
        for (k, l) in last.iter().enumerate() {
            pb.w[pb::SRC_LAST + k] = *l as u16;
        }
        pb.w[pb::SRC_FRAC] = (pos & 0xFFFF) as u16;
        acc.store(pb);
    }

    fn mix_aux(&mut self, bus: &mut dyn Bus, aux: usize, write: u32, read: u32) {
        let first = if aux == 0 { AUXA_L } else { AUXB_L };
        if write != 0 {
            for i in 0..3 {
                Self::write_samples(
                    bus,
                    write + (i * SAMPLES * 4) as u32,
                    &self.buffers[first + i],
                );
            }
        }
        let back = Self::read_samples(bus, read, 3 * SAMPLES);
        for ch in 0..3 {
            for k in 0..SAMPLES {
                let b = &mut self.buffers[MAIN_L + ch][k];
                *b = b.wrapping_add(back[ch * SAMPLES + k]);
            }
        }
    }

    fn upload_lrs(&self, bus: &mut dyn Bus, addr: u32) {
        for i in 0..3 {
            Self::write_samples(
                bus,
                addr + (i * SAMPLES * 4) as u32,
                &self.buffers[MAIN_L + i],
            );
        }
    }

    fn set_main_lr(&mut self, bus: &dyn Bus, addr: u32) {
        let s = Self::read_samples(bus, addr, SAMPLES);
        self.buffers[MAIN_L].copy_from_slice(&s);
        self.buffers[MAIN_R].copy_from_slice(&s);
        self.buffers[MAIN_S] = [0; SAMPLES];
    }

    fn set_opposite_lr(&mut self, bus: &dyn Bus, addr: u32) {
        let s = Self::read_samples(bus, addr, SAMPLES);
        for k in 0..SAMPLES {
            self.buffers[MAIN_L][k] = s[k].wrapping_neg();
            self.buffers[MAIN_R][k] = s[k];
        }
        self.buffers[MAIN_S] = [0; SAMPLES];
    }

    fn compressor(&mut self, bus: &dyn Bus, threshold: u16, release_frames: u32, table: u32) {
        let t = i32::from(threshold);
        let triggered = (0..SAMPLES)
            .any(|i| self.buffers[MAIN_L][i].abs() > t || self.buffers[MAIN_R][i].abs() > t);
        let frame_bytes = (SAMPLES * 2) as u32;
        let offset = if triggered {
            let o = self.compressor_pos * frame_bytes;
            self.compressor_pos = release_frames;
            o
        } else if self.compressor_pos != 0 {
            self.compressor_pos -= 1;
            // The release ramps follow the 11 attack ramps.
            (11 + self.compressor_pos) * frame_bytes
        } else {
            return;
        };
        let ramp = Self::read_words(bus, table + offset, SAMPLES);
        for i in 0..SAMPLES {
            let c = i64::from(ramp[i]);
            self.buffers[MAIN_L][i] = ((i64::from(self.buffers[MAIN_L][i]) * c) >> 15) as i32;
            self.buffers[MAIN_R][i] = ((i64::from(self.buffers[MAIN_R][i]) * c) >> 15) as i32;
        }
    }

    /// The frame's output: surround as is, then left and right as 16-bit pairs, right first.
    fn output(&self, bus: &mut dyn Bus, lr: u32, surround: u32) {
        Self::write_samples(bus, surround, &self.buffers[MAIN_S]);
        let mut out = Vec::with_capacity(SAMPLES * 4);
        for i in 0..SAMPLES {
            let l = clamp_s16(i64::from(self.buffers[MAIN_L][i]));
            let r = clamp_s16(i64::from(self.buffers[MAIN_R][i]));
            out.extend_from_slice(&r.to_be_bytes());
            out.extend_from_slice(&l.to_be_bytes());
        }
        bus.write(lr, &out);
    }

    fn mix_auxb_lr(&mut self, bus: &mut dyn Bus, up: u32, down: u32) {
        Self::write_samples(bus, up, &self.buffers[AUXB_L]);
        Self::write_samples(bus, up + (SAMPLES * 4) as u32, &self.buffers[AUXB_R]);
        let s = Self::read_samples(bus, down, 2 * SAMPLES);
        for k in 0..SAMPLES {
            self.buffers[AUXB_L][k] = s[k];
            self.buffers[MAIN_L][k] = self.buffers[MAIN_L][k].wrapping_add(s[k]);
        }
        for k in 0..SAMPLES {
            self.buffers[AUXB_R][k] = s[SAMPLES + k];
            self.buffers[MAIN_R][k] = self.buffers[MAIN_R][k].wrapping_add(s[SAMPLES + k]);
        }
    }

    fn send_aux_and_mix(&mut self, bus: &mut dyn Bus, a: &[u32]) {
        for i in 0..3 {
            Self::write_samples(
                bus,
                a[0] + (i * SAMPLES * 4) as u32,
                &self.buffers[AUXA_L + i],
            );
        }
        Self::write_samples(bus, a[1], &self.buffers[AUXB_S]);
        for (buf, &addr) in [MAIN_L, MAIN_R, AUXB_L, AUXB_R].iter().zip(&a[2..6]) {
            let s = Self::read_samples(bus, addr, SAMPLES);
            for k in 0..SAMPLES {
                self.buffers[*buf][k] = self.buffers[*buf][k].wrapping_add(s[k]);
            }
        }
    }
}
