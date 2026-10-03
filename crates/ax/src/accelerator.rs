// SPDX-License-Identifier: GPL-3.0-or-later
// Follows Dolphin's DSP accelerator (DSPAccelerator.cpp) and AX HLE's use of it (AXVoice.h),
// GPL-2.0-or-later.

//! The DSP's accelerator, as AX drives it for one voice: it walks the voice's samples in ARAM,
//! decoding ADPCM (with its frame headers) or PCM, and loops or stops at the end.

use crate::Bus;
use crate::pb::{self, Pb};

pub(crate) struct Accelerator {
    start: u32,
    end: u32,
    current: u32,
    format: u16,
    gain: i16,
    yn1: i16,
    yn2: i16,
    pred_scale: u16,
    reads_stopped: bool,
    coefs: [i16; 16],
}

const START_END_MASK: u32 = 0x3FFF_FFFF;
const CURRENT_MASK: u32 = 0xBFFF_FFFF;
const ARAM_MASK: u32 = 0x00FF_FFFF;

impl Accelerator {
    /// Set up from a voice's PB.
    pub fn setup(pb: &Pb) -> Self {
        let w = &pb.w;
        let addr = |at: usize| u32::from(w[at]) << 16 | u32::from(w[at + 1]);
        let mut coefs = [0i16; 16];
        for (k, c) in coefs.iter_mut().enumerate() {
            *c = w[pb::ADPCM_COEFS + k] as i16;
        }
        Accelerator {
            start: addr(pb::LOOP_ADDR) & START_END_MASK,
            end: addr(pb::END_ADDR) & START_END_MASK,
            current: addr(pb::CUR_ADDR) & CURRENT_MASK,
            format: w[pb::SAMPLE_FORMAT],
            gain: w[pb::GAIN] as i16,
            yn1: w[pb::YN1] as i16,
            yn2: w[pb::YN2] as i16,
            pred_scale: w[pb::PRED_SCALE] & 0x7F,
            reads_stopped: false,
            coefs,
        }
    }

    /// Back into the PB: where it got to, and the decoder's state.
    pub fn store(&self, pb: &mut Pb) {
        pb.w[pb::CUR_ADDR] = (self.current >> 16) as u16;
        pb.w[pb::CUR_ADDR + 1] = self.current as u16;
        pb.w[pb::YN1] = self.yn1 as u16;
        pb.w[pb::YN2] = self.yn2 as u16;
        pb.w[pb::PRED_SCALE] = self.pred_scale;
    }

    fn mem(bus: &dyn Bus, addr: u32) -> u16 {
        u16::from(bus.aram(addr & ARAM_MASK))
    }

    fn current_sample(&self, bus: &dyn Bus) -> u16 {
        match self.format & 3 {
            0 => {
                let v = Self::mem(bus, self.current >> 1);
                if self.current & 1 != 0 {
                    v & 0xF
                } else {
                    v >> 4
                }
            }
            1 => Self::mem(bus, self.current),
            2 => {
                Self::mem(bus, self.current.wrapping_mul(2)) << 8
                    | Self::mem(bus, self.current.wrapping_mul(2).wrapping_add(1))
            }
            _ => 0,
        }
    }

    /// The next sample. At the end of the samples it loops, as the voice's PB says, or stops the
    /// voice.
    pub fn read_sample(&mut self, bus: &dyn Bus, pb: &mut Pb) -> u16 {
        if self.reads_stopped {
            return 0;
        }
        let decode = (self.format >> 2) & 3;
        let gain_scale = (self.format >> 4) & 3;
        // Reads from the accelerator's input register (decode 1 and 3) find nothing in AX.
        let mut raw = if decode == 1 || decode == 3 {
            0i16
        } else {
            self.current_sample(bus) as i16
        };
        let idx = usize::from((self.pred_scale >> 4) & 7);
        let coef1 = i32::from(self.coefs[idx * 2]);
        let coef2 = i32::from(self.coefs[idx * 2 + 1]);
        let val: u16;
        let mut step: u32;
        match decode {
            0 => {
                raw &= 0xF;
                let scale = 1i32 << (self.pred_scale & 0xF);
                if raw >= 8 {
                    raw -= 16;
                }
                let v = scale.wrapping_mul(i32::from(raw)).wrapping_add(
                    0x400i32
                        .wrapping_add(coef1.wrapping_mul(i32::from(self.yn1)))
                        .wrapping_add(coef2.wrapping_mul(i32::from(self.yn2)))
                        >> 11,
                );
                val = v.clamp(-0x8000, 0x7FFF) as i16 as u16;
                step = 2;
                self.yn2 = self.yn1;
                self.yn1 = val as i16;
                self.current = self.current.wrapping_add(1);
                if self.end & 0xF == 0 && self.current == self.end {
                    self.current = self.start.wrapping_add(1);
                } else if self.end & 0xF == 1 && self.current == self.end.wrapping_sub(1) {
                    self.current = self.start;
                } else if self.current & 15 == 0 {
                    self.pred_scale = Self::mem(bus, (self.current & !15) >> 1);
                    self.current = self.current.wrapping_add(2);
                    step += 2;
                }
            }
            _ => {
                let shift = match gain_scale {
                    0 => 11,
                    2 => 16,
                    _ => 0,
                };
                let v = ((i32::from(self.gain) * i32::from(raw)) >> shift)
                    + (((coef1 * i32::from(self.yn1)) >> shift)
                        + ((coef2 * i32::from(self.yn2)) >> shift));
                val = v as i16 as u16;
                self.yn2 = self.yn1;
                self.yn1 = val as i16;
                step = 2;
                if decode != 1 {
                    self.current = self.current.wrapping_add(1);
                }
            }
        }
        if self.current == self.end.wrapping_add(step).wrapping_sub(1) {
            self.current = self.start;
            self.reads_stopped = true;
            self.end_of_samples(pb);
        }
        self.current &= CURRENT_MASK;
        val
    }

    /// AX's handling of the end of a voice's samples: back to the loop's decoder state, or stop.
    fn end_of_samples(&mut self, pb: &mut Pb) {
        if pb.w[pb::LOOPING] != 0 {
            self.pred_scale = pb.w[pb::LOOP_PRED_SCALE] & 0x7F;
            if pb.w[pb::IS_STREAM] != 1 {
                self.yn1 = pb.w[pb::LOOP_YN1] as i16;
                self.yn2 = pb.w[pb::LOOP_YN2] as i16;
            } else {
                pb.w[pb::LOOP_COUNTER] = pb.w[pb::LOOP_COUNTER].wrapping_add(1);
            }
            // Setting yn2 restarts reads.
            self.reads_stopped = false;
        } else {
            pb.w[pb::RUNNING] = 0;
        }
    }
}
