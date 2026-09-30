// SPDX-License-Identifier: GPL-3.0-or-later

//! Dolphin's float instructions, compiled from its source, as the reference that `gekko-fp` and
//! the interpreter are fuzzed against. `Master` is current Dolphin, which matches hardware;
//! `Slippi` is the fork that recorded the replays.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Master,
    Slippi,
}

unsafe extern "C" {
    fn dolphin_master_exec(hex: u32, fpr: *mut u64, fpscr: *mut u32, cr: *mut u32) -> i32;
    fn dolphin_master_convert_to_double(value: u32) -> u64;
    fn dolphin_master_convert_to_single(value: u64) -> u32;
    fn dolphin_master_convert_to_single_ftz(value: u64) -> u32;
    fn dolphin_slippi_exec(hex: u32, fpr: *mut u64, fpscr: *mut u32, cr: *mut u32) -> i32;
    fn dolphin_slippi_convert_to_double(value: u32) -> u64;
    fn dolphin_slippi_convert_to_single(value: u64) -> u32;
    fn dolphin_slippi_convert_to_single_ftz(value: u64) -> u32;
}

/// Float registers as ps0/ps1 bit pairs.
pub type Fprs = [(u64, u64); 32];

impl Variant {
    /// Runs one float instruction on `fpr` with FPSCR and CR zero and returns CR. Panics on an
    /// instruction the shim does not know.
    pub fn exec(self, hex: u32, fpr: &mut Fprs) -> u32 {
        let mut flat = [0u64; 64];
        for (i, (ps0, ps1)) in fpr.iter().enumerate() {
            flat[2 * i] = *ps0;
            flat[2 * i + 1] = *ps1;
        }
        let (mut fpscr, mut cr) = (0, 0);
        let f = match self {
            Variant::Master => dolphin_master_exec,
            Variant::Slippi => dolphin_slippi_exec,
        };
        // SAFETY: `flat` holds the 64 words the shim reads and writes.
        let known = unsafe { f(hex, flat.as_mut_ptr(), &mut fpscr, &mut cr) };
        assert!(known != 0, "shim has no float instruction {hex:08X}");
        for (i, reg) in fpr.iter_mut().enumerate() {
            *reg = (flat[2 * i], flat[2 * i + 1]);
        }
        cr
    }

    /// `lfs`: single bits to the double held in a register.
    pub fn convert_to_double(self, value: u32) -> u64 {
        // SAFETY: pure functions.
        unsafe {
            match self {
                Variant::Master => dolphin_master_convert_to_double(value),
                Variant::Slippi => dolphin_slippi_convert_to_double(value),
            }
        }
    }

    /// `stfs`: register double to single bits.
    pub fn convert_to_single(self, value: u64) -> u32 {
        // SAFETY: pure functions.
        unsafe {
            match self {
                Variant::Master => dolphin_master_convert_to_single(value),
                Variant::Slippi => dolphin_slippi_convert_to_single(value),
            }
        }
    }

    /// `psq_st` with a float GQR: like `stfs`, but denormals flush to zero.
    pub fn convert_to_single_ftz(self, value: u64) -> u32 {
        // SAFETY: pure functions.
        unsafe {
            match self {
                Variant::Master => dolphin_master_convert_to_single_ftz(value),
                Variant::Slippi => dolphin_slippi_convert_to_single_ftz(value),
            }
        }
    }
}
