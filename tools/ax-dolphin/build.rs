// SPDX-License-Identifier: GPL-3.0-or-later

//! Compiles Dolphin's AX microcode HLE from its source (dolphin/), with cpp/stubs/ standing in
//! for the rest of Dolphin.

use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    // Dolphin's code needs C++23 (std::to_underlying), which MSVC calls c++latest.
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    cc::Build::new()
        .cpp(true)
        .std(if msvc { "c++latest" } else { "c++23" })
        .file(root.join("cpp/shim.cpp"))
        .include(root.join("cpp/stubs"))
        .include(root.join("dolphin/Source/Core"))
        .flag_if_supported("/permissive-")
        .flag_if_supported("/EHsc")
        .flag_if_supported("/utf-8")
        .warnings(false)
        .compile("ax_dolphin");
    println!("cargo::rerun-if-changed=cpp");
    println!("cargo::rerun-if-changed=dolphin");
}
