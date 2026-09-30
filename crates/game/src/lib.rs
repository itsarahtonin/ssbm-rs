// SPDX-License-Identifier: GPL-3.0-or-later

//! The game's code in Rust. `tools/c2rs` translates each decomp C file into a module of `tu`;
//! `register` puts the ports into a machine, where they replace the original functions.

use ssbm_rt::Ctx;

#[allow(non_snake_case)]
pub mod manual;
pub mod support;
pub mod tu;

/// Registers the ports of every unit `wanted` accepts, by unit name such as
/// `melee/ft/ft_0C31`. Returns how many units were registered.
pub fn register(ctx: &Ctx, wanted: impl Fn(&str) -> bool) -> usize {
    let mut n = 0;
    for (unit, register) in tu::UNITS {
        if wanted(unit) {
            register(ctx);
            n += 1;
        }
    }
    n
}
