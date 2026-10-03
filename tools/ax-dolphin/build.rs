// SPDX-License-Identifier: GPL-3.0-or-later

//! Compiles Dolphin's AX microcode HLE from its source (dolphin/), with cpp/stubs/ standing in
//! for the rest of Dolphin.

use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    cc::Build::new()
        .cpp(true)
        .std("c++latest")
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
