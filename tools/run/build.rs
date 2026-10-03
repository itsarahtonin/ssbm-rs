// SPDX-License-Identifier: GPL-3.0-or-later

//! The player's build on Windows carries an application manifest: Common Controls 6, which its
//! dialogs' own button labels need, and per-monitor DPI awareness, which keeps them sharp.

fn main() {
    if std::env::var_os("CARGO_FEATURE_PLAYER").is_some()
        && std::env::var_os("CARGO_CFG_WINDOWS").is_some()
    {
        embed_manifest::embed_manifest(embed_manifest::new_manifest("ssbm-rs"))
            .expect("the application manifest");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
