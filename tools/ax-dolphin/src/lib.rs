// SPDX-License-Identifier: GPL-3.0-or-later

//! Dolphin's AX microcode HLE, built from its source (cpp/shim.cpp), and a checker that runs each
//! command list through it and through ssbm-ax over the same memory, comparing what they write.

use std::collections::BTreeMap;
use std::ffi::c_void;

use ssbm_ax::Bus;

#[repr(C)]
struct CBus {
    user: *mut c_void,
    read: extern "C" fn(*mut c_void, u32, *mut u8, usize),
    write: extern "C" fn(*mut c_void, u32, *const u8, usize),
    aram: extern "C" fn(*mut c_void, u32) -> u8,
}

unsafe extern "C" {
    fn ax_dolphin_new(crc: u32, coefs: *const u8, len: usize) -> *mut c_void;
    fn ax_dolphin_run(handle: *mut c_void, bus: *mut CBus, addr: u32, size: u16);
    fn ax_dolphin_free(handle: *mut c_void);
}

extern "C" fn read(user: *mut c_void, addr: u32, out: *mut u8, len: usize) {
    // SAFETY: `user` is the `&mut dyn Bus` run() passes, alive for the call; `out` is `len`
    // bytes the C++ side owns.
    let (bus, out) = unsafe {
        (
            &mut *(user as *mut &mut dyn Bus),
            std::slice::from_raw_parts_mut(out, len),
        )
    };
    bus.read(addr, out);
}

extern "C" fn write(user: *mut c_void, addr: u32, data: *const u8, len: usize) {
    // SAFETY: as in read().
    let (bus, data) = unsafe {
        (
            &mut *(user as *mut &mut dyn Bus),
            std::slice::from_raw_parts(data, len),
        )
    };
    bus.write(addr, data);
}

extern "C" fn aram(user: *mut c_void, addr: u32) -> u8 {
    // SAFETY: as in read().
    let bus = unsafe { &mut *(user as *mut &mut dyn Bus) };
    bus.aram(addr)
}

/// Dolphin's AX microcode, with its own state between command lists.
pub struct DolphinAx(*mut c_void);

impl DolphinAx {
    /// The microcode with hash `crc`, resampling with `coefs` (dsp_coef.bin's 0x800).
    pub fn new(crc: u32, coefs: &[i16]) -> Self {
        let bytes: Vec<u8> = coefs.iter().flat_map(|c| c.to_be_bytes()).collect();
        // SAFETY: the C++ side copies the coefficients.
        DolphinAx(unsafe { ax_dolphin_new(crc, bytes.as_ptr(), bytes.len()) })
    }

    pub fn run(&mut self, bus: &mut dyn Bus, addr: u32, size: u16) {
        let mut bus: &mut dyn Bus = bus;
        let mut c = CBus {
            user: &mut bus as *mut &mut dyn Bus as *mut c_void,
            read,
            write,
            aram,
        };
        // SAFETY: the handle is ours, and the bus outlives the call.
        unsafe { ax_dolphin_run(self.0, &mut c, addr, size) };
    }
}

impl Drop for DolphinAx {
    fn drop(&mut self) {
        // SAFETY: the handle is ours and freed once.
        unsafe { ax_dolphin_free(self.0) };
    }
}

/// Memory as a command list leaves it: reads see the list's own writes, which are kept apart.
struct Overlay<'a> {
    base: &'a dyn Bus,
    writes: BTreeMap<u32, u8>,
}

impl Bus for Overlay<'_> {
    fn read(&self, addr: u32, out: &mut [u8]) {
        self.base.read(addr, out);
        for (i, b) in out.iter_mut().enumerate() {
            if let Some(&v) = self
                .writes
                .get(&(addr.wrapping_add(i as u32) & 0x01FF_FFFF))
            {
                *b = v;
            }
        }
    }

    fn write(&mut self, addr: u32, data: &[u8]) {
        for (i, &b) in data.iter().enumerate() {
            self.writes
                .insert(addr.wrapping_add(i as u32) & 0x01FF_FFFF, b);
        }
    }

    fn aram(&self, addr: u32) -> u8 {
        self.base.aram(addr)
    }
}

/// Runs command lists through both microcodes and compares what they write to memory.
pub struct Checker {
    ours: ssbm_ax::Ax,
    theirs: DolphinAx,
    pub lists: u64,
    pub mismatches: u64,
}

impl Checker {
    pub fn new(crc: u32) -> Self {
        let coefs = ssbm_ax::coefs::table();
        Checker {
            ours: ssbm_ax::Ax::new(crc, Some(coefs.clone())),
            theirs: DolphinAx::new(crc, &coefs),
            lists: 0,
            mismatches: 0,
        }
    }

    /// Runs the command list at `addr` on both over `mem`, which neither changes; a description
    /// of the first difference in what they write, if any.
    pub fn check(&mut self, mem: &dyn Bus, addr: u32, size: u16) -> Option<String> {
        let mut a = Overlay {
            base: mem,
            writes: BTreeMap::new(),
        };
        let mut b = Overlay {
            base: mem,
            writes: BTreeMap::new(),
        };
        self.ours.run(&mut a, addr, size);
        self.theirs.run(&mut b, addr, size);
        self.lists += 1;
        if a.writes == b.writes {
            return None;
        }
        self.mismatches += 1;
        let keys: std::collections::BTreeSet<u32> =
            a.writes.keys().chain(b.writes.keys()).copied().collect();
        let first = keys
            .into_iter()
            .find(|k| a.writes.get(k) != b.writes.get(k))?;
        let around = |w: &BTreeMap<u32, u8>| {
            let start = first & !0xF;
            (start..start + 16)
                .map(|k| w.get(&k).map_or("--".into(), |v| format!("{v:02x}")))
                .collect::<Vec<_>>()
                .join(" ")
        };
        Some(format!(
            "list {} at {addr:08x}: first difference at {first:08x} ({} bytes written by ours, {} by Dolphin's)\n  ours    {}\n  Dolphin {}",
            self.lists,
            a.writes.len(),
            b.writes.len(),
            around(&a.writes),
            around(&b.writes)
        ))
    }
}
