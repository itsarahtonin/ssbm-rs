// SPDX-License-Identifier: GPL-3.0-or-later

//! Hand ports, one module per decomp unit, for functions the translator cannot get right from
//! the C alone: assembly, and ones reading variables the C leaves uninitialized. The translator
//! skips every function defined here and registers these instead.

pub mod dolphin__ai__ai;
pub mod dolphin__base__PPCArch;
pub mod dolphin__db__db;
pub mod dolphin__os__OS;
pub mod dolphin__os__OSAlarm;
pub mod dolphin__os__OSCache;
pub mod dolphin__os__OSInterrupt;
pub mod dolphin__os__OSSync;
pub mod dolphin__os__OSTime;
pub mod melee__gm__gm_1601;
pub mod MSL__string;
pub mod Runtime__Gecko_setjmp;
pub mod Runtime__runtime;
pub mod sysdolphin__baselib__hsd_397E;

mod exception;
