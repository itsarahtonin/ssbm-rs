// SPDX-License-Identifier: GPL-3.0-or-later

//! The player's build on Windows carries its icon (icon/ssbm-rs.ico, which icon/make.py draws)
//! and an application manifest: Common Controls 6, which its dialogs' own button labels need,
//! and per-monitor DPI awareness, which keeps them sharp. Both go in one resource section.

fn main() {
    if std::env::var_os("CARGO_FEATURE_PLAYER").is_some()
        && std::env::var_os("CARGO_CFG_WINDOWS").is_some()
    {
        let manifest = embed_manifest::new_manifest("ssbm-rs").to_string();
        let mut resources = winresource::WindowsResource::new();
        resources
            .set_icon("icon/ssbm-rs.ico")
            .set_manifest(&manifest)
            .set("ProductName", "ssbm-rs")
            .set("FileDescription", "ssbm-rs");
        resources.compile().expect("the player's resources");
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=icon/ssbm-rs.ico");
}
