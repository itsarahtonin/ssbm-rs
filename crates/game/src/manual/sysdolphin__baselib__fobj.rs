// SPDX-License-Identifier: GPL-3.0-or-later
// Hand ports of functions in src/sysdolphin/baselib/fobj.c that read variables the C leaves
// uninitialized.

#![allow(non_snake_case)]

use gekko_fp as fp;
use ssbm_rt::*;
use ssbm_types::fns;
use ssbm_types::records::*;

use crate::support::*;

/// `@193`, which HSD_FObjInterpretAnim keeps in r30.
const FOBJ_STRINGS: u32 = 0x8040_6350;
/// Where the update callback returns to, which its prologue saves in this frame.
const UPDATE_RETURN: u32 = 0x8036_B014;

/// For an interpolation it has no case for, the C hands the callback `fobjdata` unset, and the
/// callback reads whatever the stack held there: with HSD_A_J_BRANCH, a float that hides or shows
/// a whole branch of joints (Final Destination's background change meets it). The stack is
/// written as the original writes it, so that value is the same: the prologue's saves of
/// HSD_FObjInterpretAnim's r29 to r31 (0x43300000, `@193` and `fobj`), the temporaries MWCC
/// converts `fterm` to float through, and the callback's saved return address. The return
/// address this prologue saves in its caller's frame depends on the call site and is not written.
pub fn FObjUpdateAnim<'a>(ctx: &'a Ctx, fobj: HSD_FObj<'a>, obj: Addr<'a>, obj_update: FnPtr<'a>) {
    let __frame = ctx.stack_frame(0x38);
    let fobjdata: HSD_ObjData<'a> = frame_at(ctx, &__frame, 0xc);
    let sp = ctx.regs.r(1);
    ctx.write_u32(sp + 0x34, Handle::addr(fobj));
    ctx.write_u32(sp + 0x30, FOBJ_STRINGS);
    ctx.write_u32(sp + 0x2c, 0x4330_0000);
    // MWCC converts `fobj->fterm` to a double through r1+0x20 (0x43300000:fterm).
    let fterm = || {
        ctx.write_u32(sp + 0x24, fobj.fterm() as u32);
        ctx.write_u32(sp + 0x20, 0x4330_0000);
        (fobj.fterm() as i32) as f64
    };
    if Handle::is_null(obj_update) {
        return;
    }
    match fobj.op_intrp() as i32 {
        // HSD_A_OP_KEY
        6 => {
            if (fobj.flags() as i32) & 0x80 == 0 {
                return;
            }
            fobjdata.set_fv(fobj.p0());
            fobj.set_flags(((fobj.flags() as u32) & 0xffff_ff7f) as u8);
        }
        // HSD_A_OP_CON
        1 => {
            let v = if fobj.time() >= fp::frsp(fterm()) {
                fobj.p1()
            } else {
                fobj.p0()
            };
            fobjdata.set_fv(v);
        }
        // HSD_A_OP_LIN
        2 => {
            if (fobj.flags() as i32) & 0x20 != 0 {
                fobj.set_flags(((fobj.flags() as u32) & 0xffff_ffdf) as u8);
                if (fobj.fterm() as i32) != 0 {
                    fobj.set_d0(fp::fdivs(
                        fp::fsubs(fobj.p1(), fobj.p0()),
                        fp::frsp(fterm()),
                    ));
                } else {
                    fobj.set_d0(fp::frsp(0.0));
                    fobj.set_p0(fobj.p1());
                }
            }
            fobjdata.set_fv(fp::fmadds(fobj.d0(), fobj.time(), fobj.p0()));
        }
        // HSD_A_OP_SPL0, HSD_A_OP_SPL, HSD_A_OP_SLP
        3..=5 => {
            if (fobj.fterm() as i32) != 0 {
                let rate = fp::frsp(fp::fdiv(1.0, fterm()));
                fobjdata.set_fv(fns::splGetHelmite(
                    ctx,
                    rate,
                    fobj.time(),
                    fobj.p0(),
                    fobj.p1(),
                    fobj.d0(),
                    fobj.d1(),
                ));
            } else {
                fobjdata.set_fv(fobj.p1());
            }
        }
        _ => {}
    }
    ctx.write_u32(sp + 4, UPDATE_RETURN);
    obj_update.call::<_, ()>((obj, fobj.obj_type() as i32, fobjdata));
}
