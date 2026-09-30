// SPDX-License-Identifier: GPL-3.0-or-later

//! `setjmp` and `longjmp` across ports and original code. Either may save a jump buffer and
//! either may jump to it: the jump unwinds the Rust stack to where the buffer was saved, so the
//! ports and interpreter runs in between end, as their frames do on the console.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use crate::Ctx;

/// Where a jump buffer was saved.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Target {
    /// A port's `setjmp` block, `Ctx::setjmp`.
    Port,
    /// Original code, in the interpreter run with this id.
    Run { run: u64, natives: usize },
}

/// Panic payload of a `longjmp`, caught where its buffer was saved.
struct LongJmp {
    env: u32,
    value: i32,
}

/// Describes a panic payload that is a `longjmp`.
pub(crate) fn describe(payload: &(dyn Any + Send)) -> Option<String> {
    payload
        .downcast_ref::<LongJmp>()
        .map(|j| format!("longjmp to {:#010X} with {}", j.env, j.value))
}

// The jump buffer's layout, from Runtime/Gecko_setjmp.h.
pub(crate) const JMP_BUF_SIZE: u32 = 0x118;
const PC: u32 = 0x0;
const CR: u32 = 0x4;
const SP: u32 = 0x8;
const RTOC: u32 = 0xC;
const GPRS: u32 = 0x14;
const FPRS: u32 = 0x60;
const FPSCR: u32 = 0xF0;

impl Ctx {
    /// `if (setjmp(env) == 0) body` in a port: runs `body`, which a `longjmp` to `env` from
    /// anywhere inside it, ported or original, ends early. Returns 0 when the body finishes,
    /// else the value the `longjmp` passed. As after the original's `longjmp`, the registers a
    /// call preserves hold what they held at the `setjmp`.
    pub fn setjmp(&self, env: u32, body: impl FnOnce()) -> i32 {
        let saved = self.regs.snapshot();
        let natives = self.natives_depth();
        let at = self.jump_targets.borrow().len();
        self.jump_targets.borrow_mut().push((env, Target::Port));
        self.jump_buffers.borrow_mut().insert(env);
        let result = catch_unwind(AssertUnwindSafe(body));
        let ours = self.latest_target(env).is_some_and(|(i, _)| i == at);
        self.jump_targets.borrow_mut().truncate(at);
        let payload = match result {
            Ok(()) => return 0,
            Err(p) => p,
        };
        let jump = match payload.downcast::<LongJmp>() {
            Ok(jump) if jump.env == env && ours => jump,
            Ok(jump) => resume_unwind(jump),
            Err(p) => resume_unwind(p),
        };
        self.truncate_natives(natives);
        let r = &self.regs;
        for i in [1, 2].into_iter().chain(13..32) {
            r.set_r(i, saved.gpr[i]);
        }
        for i in 14..32 {
            r.fpr[i].set(saved.fpr[i]);
        }
        r.cr.set(saved.cr);
        r.fpscr.set(saved.fpscr);
        if jump.value == 0 { 1 } else { jump.value }
    }

    /// `__setjmp(env)` called from original code: saves where it returns and the registers a
    /// call preserves, as the original does, and which interpreter run it is in.
    pub fn setjmp_original(&self, env: u32) {
        let r = &self.regs;
        self.write_u32(env + PC, r.lr.get());
        self.write_u32(env + CR, r.cr.get());
        self.write_u32(env + SP, r.r(1));
        self.write_u32(env + RTOC, r.r(2));
        for i in 13..32 {
            self.write_u32(env + GPRS + 4 * (i as u32 - 13), r.r(i));
        }
        for i in 14..32 {
            self.write_u64(env + FPRS + 8 * (i as u32 - 14), r.fpr[i].get().ps0.to_bits());
        }
        // `mffs` leaves FPSCR in the low word.
        self.write_u64(env + FPSCR, 0xFFF8_0000_0000_0000 | u64::from(r.fpscr.get()));
        let target = Target::Run {
            run: self.current_run.get(),
            natives: self.natives_depth(),
        };
        self.jump_buffers.borrow_mut().insert(env);
        let mut targets = self.jump_targets.borrow_mut();
        targets.retain(|t| t.0 != env || matches!(t.1, Target::Port));
        targets.push((env, target));
    }

    /// `longjmp(env, value)`: unwinds to where `env` was saved, and does not return. A buffer
    /// nothing here saved is jumped to in place: the original code that called this continues
    /// where the buffer says.
    pub fn longjmp(&self, env: u32, value: i32) {
        if self.latest_target(env).is_some() {
            resume_unwind(Box::new(LongJmp { env, value }));
        }
        let pc = self.load_jmp_buf(env, value);
        self.resume_at(pc);
    }

    /// Where interpreter run `run` continues if `payload` is a `longjmp` to a buffer saved in it.
    pub(crate) fn landing(
        &self,
        run: u64,
        payload: Box<dyn Any + Send>,
    ) -> Result<u32, Box<dyn Any + Send>> {
        let Some(jump) = payload.downcast_ref::<LongJmp>() else {
            return Err(payload);
        };
        match self.latest_target(jump.env) {
            Some((i, Target::Run { run: r, natives })) if r == run => {
                self.jump_targets.borrow_mut().truncate(i + 1);
                self.truncate_natives(natives);
                Ok(self.load_jmp_buf(jump.env, jump.value))
            }
            _ => Err(payload),
        }
    }

    fn latest_target(&self, env: u32) -> Option<(usize, Target)> {
        let targets = self.jump_targets.borrow();
        targets
            .iter()
            .enumerate()
            .rev()
            .find(|(_, t)| t.0 == env)
            .map(|(i, t)| (i, t.1))
    }

    /// Restores what `env` saved, as the original `__longjmp` does, and returns where to go on.
    fn load_jmp_buf(&self, env: u32, value: i32) -> u32 {
        let r = &self.regs;
        let pc = self.read_u32(env + PC);
        r.lr.set(pc);
        r.set_r(1, self.read_u32(env + SP));
        r.set_r(2, self.read_u32(env + RTOC));
        for i in 13..32 {
            r.set_r(i, self.read_u32(env + GPRS + 4 * (i as u32 - 13)));
        }
        for i in 14..32 {
            let mut ps = r.fpr[i].get();
            ps.ps0 = f64::from_bits(self.read_u64(env + FPRS + 8 * (i as u32 - 14)));
            r.fpr[i].set(ps);
        }
        r.fpscr.set(self.read_u64(env + FPSCR) as u32);
        // `cmpwi val, 0` sets CR0 after the saved CR is restored.
        let cr0 = match value {
            v if v < 0 => 8,
            v if v > 0 => 4,
            _ => 2,
        } | (r.xer.get() >> 31);
        r.cr.set((self.read_u32(env + CR) & 0x0FFF_FFFF) | (cr0 << 28));
        r.set_r(3, if value == 0 { 1 } else { value as u32 });
        pc
    }
}
