// SPDX-License-Identifier: GPL-3.0-or-later

//! Hand ports, one module per decomp unit, for functions the translator cannot get right from
//! the C alone, such as ones reading variables the C leaves uninitialized. The translator
//! skips every function defined here and registers these instead.

pub mod MSL__string;
pub mod Runtime__Gecko_setjmp;
pub mod Runtime__runtime;
pub mod melee__gm__gm_1601;
