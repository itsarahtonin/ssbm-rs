// SPDX-License-Identifier: GPL-3.0-or-later

//! Compiles the two references: current Dolphin's interpreter float code, from its source, and a
//! transcription of what Slippi's Ishiiruka Dolphin JIT emits.

use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    // Dolphin's code needs C++23, which MSVC calls c++latest.
    let standard = if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        "c++latest"
    } else {
        "c++23"
    };
    cc::Build::new()
        .cpp(true)
        .std(standard)
        .file(root.join("cpp/unity.cpp"))
        .include(root.join("cpp/stubs"))
        .include(root.join("cpp"))
        .include(root.join("dolphin/common/Source/Core"))
        .include(root.join("dolphin/master/Source/Core"))
        .define("SHIM_NAMESPACE", "dolphin_master")
        .define("SHIM_PREFIX", "dolphin_master_")
        .flag_if_supported("/permissive-")
        .flag_if_supported("/EHsc")
        .flag_if_supported("/utf-8")
        .warnings(false)
        .compile("dolphin_fp_master");
    cc::Build::new()
        .cpp(true)
        .std(standard)
        .file(root.join("cpp/ishiiruka.cpp"))
        .include(root.join("dolphin/ishiiruka"))
        .flag_if_supported("/arch:AVX2")
        .flag_if_supported("-mavx2")
        .flag_if_supported("-mfma")
        .warnings(false)
        .compile("dolphin_fp_ishiiruka");
    println!("cargo::rerun-if-changed=cpp");
    println!("cargo::rerun-if-changed=dolphin");
}
