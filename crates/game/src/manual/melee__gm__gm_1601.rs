// SPDX-License-Identifier: GPL-3.0-or-later
// Hand ports of functions in src/melee/gm/gm_1601.c that the translator cannot get right.

use gekko_fp as fp;
use ssbm_rt::Ctx;
use ssbm_types::enums;

/// The decomp leaves `base` unset for most characters. MWCC keeps it in r3, which still holds
/// `ckind` there, so the original returns `ckind + arg2 * 30` for them.
pub fn gm_80168B34(_ctx: &Ctx, ckind: i32, arg1: i32, arg2: i32) -> f64 {
    let ck = ckind as u32;
    if ck == enums::CKind_GKoops as u32 {
        return 58.0;
    }
    if ck == enums::CKind_Boy as u32 || ck == enums::CKind_Girl as u32 {
        return 26.0;
    }
    if ck == enums::CKind_MasterH as u32 {
        return 28.0;
    }
    if ck == enums::CKind_CrezyH as u32 {
        return 27.0;
    }
    let mut base = ckind;
    if ck == enums::CKind_Zelda as u32 || ck == enums::CKind_Seak as u32 {
        base = if arg1 == 7 { 0x19 } else { 0x12 };
    } else if ck == enums::ChKind_Sandbag as u32 {
        return 59.0;
    } else if ck == enums::ChKind_Popo as u32 {
        base = 0xE;
    } else if ckind > enums::CKind_Seak {
        // Signed, as `-enum int` makes CharacterKind.
        base = ckind.wrapping_sub(1);
    }
    fp::frsp(base.wrapping_add(arg2.wrapping_mul(30)) as f64)
}
