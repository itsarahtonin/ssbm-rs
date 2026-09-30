// SPDX-License-Identifier: GPL-3.0-or-later
// Hand ports of functions in src/sysdolphin/baselib/hsd_397E.c that are assembly.

use ssbm_rt::{Ctx, VarArg};
use ssbm_types::fns;

use crate::support::cstr;

/// The special registers `baselib_mfspr` reads, by number: the XER, LR and CTR, the exception
/// and memory registers, the GQRs, HID0 to HID2, the DMA and performance counters, the cache
/// and thermal controls.
const READABLE: &[u32] = &[
    1, 8, 9, 18, 19, 22, 25, 26, 27, 272, 273, 274, 275, 280, 282, 287, 528, 529, 530, 531, 532,
    533, 534, 535, 536, 537, 538, 539, 540, 541, 542, 543, 912, 913, 914, 915, 916, 917, 918,
    919, 920, 921, 922, 923, 936, 937, 938, 939, 940, 941, 942, 943, 952, 953, 954, 955, 956,
    957, 958, 959, 1008, 1009, 1010, 1013, 1017, 1019, 1020, 1021, 1022,
];

/// "unsupported no. of special purpose register (%d)."
const UNSUPPORTED: u32 = 0x8040_BF10;

/// The special register numbered `spr`, or 0 after a report for one it has no `mfspr` for.
pub fn baselib_mfspr(ctx: &Ctx, spr: i32) -> i32 {
    if READABLE.contains(&(spr as u32)) {
        return ctx.regs.get_spr(spr as u32) as i32;
    }
    fns::OSReport(ctx, cstr(ctx, UNSUPPORTED), &[VarArg::Int(spr as u32)]);
    0
}
