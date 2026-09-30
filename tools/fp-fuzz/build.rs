// SPDX-License-Identifier: GPL-3.0-or-later

//! Compiles Dolphin's float instructions twice: current master and Slippi's fork.

use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    for variant in ["master", "slippi"] {
        cc::Build::new()
            .cpp(true)
            .std("c++latest")
            .file(root.join("cpp/unity.cpp"))
            .include(root.join("cpp/stubs"))
            .include(root.join("cpp"))
            .include(root.join("dolphin/common/Source/Core"))
            .include(root.join(format!("dolphin/{variant}/Source/Core")))
            .define("SHIM_NAMESPACE", format!("dolphin_{variant}").as_str())
            .define("SHIM_PREFIX", format!("dolphin_{variant}_").as_str())
            .flag_if_supported("/permissive-")
            .flag_if_supported("/EHsc")
            .flag_if_supported("/utf-8")
            .warnings(false)
            .compile(&format!("dolphin_fp_{variant}"));
    }
    println!("cargo::rerun-if-changed=cpp");
    println!("cargo::rerun-if-changed=dolphin");
}
