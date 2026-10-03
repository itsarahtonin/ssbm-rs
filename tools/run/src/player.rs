// SPDX-License-Identifier: GPL-3.0-or-later

//! The player's build (the `player` feature): started with no arguments, as a double-click
//! starts it, it plays in a window. It asks for the disc image once, remembering it in its
//! settings folder (%APPDATA%\ssbm-rs on Windows), and keeps the memory card in slot A there.
//! It runs without a console, so errors show in a message box.

use std::path::PathBuf;

use rfd::{FileDialog, MessageButtons, MessageDialog, MessageLevel};

/// The settings folder.
fn folder() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("ssbm-rs")
}

fn error(text: &str) {
    MessageDialog::new()
        .set_level(MessageLevel::Error)
        .set_title("ssbm-rs")
        .set_description(text)
        .set_buttons(MessageButtons::Ok)
        .show();
}

/// The arguments a double-click means, or None when given some: the disc (the one remembered,
/// or one asked for), a window, and the card in the settings folder. Exits if asked for a disc
/// and given none.
pub fn args() -> Option<Vec<String>> {
    if std::env::args().len() > 1 {
        return None;
    }
    std::panic::set_hook(Box::new(|info| error(&format!("ssbm-rs stopped: {info}"))));
    let folder = folder();
    let remembered = folder.join("disc.txt");
    let mut disc = std::fs::read_to_string(&remembered)
        .ok()
        .map(|s| PathBuf::from(s.trim()))
        .filter(|p| ssbm_disc::Disc::open(p).is_ok());
    while disc.is_none() {
        let Some(path) = FileDialog::new()
            .set_title("Choose your Super Smash Bros. Melee disc image (NTSC 1.02)")
            .add_filter("Disc images", &["iso", "gcm", "rvz", "ciso", "gcz", "wbfs", "nfs"])
            .add_filter("All files", &["*"])
            .pick_file()
        else {
            std::process::exit(0);
        };
        match ssbm_disc::Disc::open(&path) {
            Ok(_) => disc = Some(path),
            Err(e) => error(&format!(
                "{} isn't a disc ssbm-rs plays: {e}\n\nIt needs Super Smash Bros. Melee, NTSC 1.02 (GALE01, revision 2).",
                path.display()
            )),
        }
    }
    let disc = disc.unwrap();
    let card = folder.join("card.raw");
    if let Err(e) = std::fs::create_dir_all(&folder)
        .and_then(|()| std::fs::write(&remembered, disc.to_string_lossy().as_bytes()))
    {
        error(&format!("Couldn't keep settings in {}: {e}", folder.display()));
    }
    Some(vec![
        std::env::args().next().unwrap_or_default(),
        disc.to_string_lossy().into_owned(),
        "--window".to_owned(),
        "--card".to_owned(),
        card.to_string_lossy().into_owned(),
    ])
}
