// SPDX-License-Identifier: GPL-3.0-or-later

//! Generated from the decomp by `tools/typegen`: a handle type per C struct, enum constants,
//! accessors for globals, and a call stub plus ABI adapter per function. C names are kept.

#![allow(
    non_snake_case,
    non_camel_case_types,
    non_upper_case_globals,
    dead_code,
    unused_imports,
    unused_parens,
    clippy::all
)]

#[rustfmt::skip]
#[path = "gen/enums.rs"]
pub mod enums;
#[rustfmt::skip]
#[path = "gen/fns.rs"]
pub mod fns;
#[rustfmt::skip]
#[path = "gen/records.rs"]
pub mod records;
#[rustfmt::skip]
#[path = "gen/symbols.rs"]
pub mod symbols;
#[rustfmt::skip]
#[path = "gen/tu.rs"]
pub mod tu;

/// The symbol containing `addr`, with its start address.
pub fn symbol_at(addr: u32) -> Option<(u32, &'static str)> {
    let i = symbols::SYMBOLS.partition_point(|s| s.0 <= addr);
    let (start, size, name, _) = *symbols::SYMBOLS.get(i.checked_sub(1)?)?;
    (addr < start + size.max(1)).then_some((start, name))
}

/// The function starting at `addr`, as `[start, end)`.
pub fn function_bounds(addr: u32) -> Option<(u32, u32)> {
    let i = symbols::SYMBOLS.partition_point(|s| s.0 < addr);
    let &(start, size, _, function) = symbols::SYMBOLS.get(i)?;
    (start == addr && function && size > 0).then_some((start, start + size))
}

/// Start addresses of the functions that overlap `[addr, addr + len)`.
pub fn functions_overlapping(addr: u32, len: u32) -> Vec<u32> {
    let end = addr.saturating_add(len.max(1));
    let first = symbols::SYMBOLS.partition_point(|s| s.0 + s.1.max(1) <= addr);
    symbols::SYMBOLS[first..]
        .iter()
        .take_while(|s| s.0 < end)
        .filter(|s| s.3 && s.0 + s.1.max(1) > addr)
        .map(|s| s.0)
        .collect()
}

/// Names addresses for diagnostics, as `name+0xoff`.
pub fn describe(addr: u32) -> Option<String> {
    let (start, name) = symbol_at(addr)?;
    Some(if addr == start {
        name.to_owned()
    } else {
        format!("{name}+{:#x}", addr - start)
    })
}

/// Everything a port needs in scope.
pub mod prelude {
    pub use crate::enums::*;
    pub use crate::fns::*;
    pub use crate::records::*;
    pub use gekko_fp::*;
    pub use ssbm_rt::*;
}
