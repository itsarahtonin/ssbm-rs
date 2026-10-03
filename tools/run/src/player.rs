// SPDX-License-Identifier: GPL-3.0-or-later

//! The player's build (the `player` feature): started with no arguments, as a double-click
//! starts it, it plays in a window. It asks for the disc image once, remembering it in its
//! settings folder (%APPDATA%\ssbm-rs on Windows), and keeps the memory card in slot A there.
//! It runs without a console, so errors show in a message box.

use std::path::PathBuf;

use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};

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

/// The first launch's word on what ssbm-rs needs, after `note` if any: true to browse for it,
/// false to quit.
fn welcome(note: &str) -> bool {
    const BROWSE: &str = "Browse\u{2026}";
    let result = MessageDialog::new()
        .set_level(MessageLevel::Info)
        .set_title("ssbm-rs")
        .set_description(format!(
            "{note}ssbm-rs plays Super Smash Bros. Melee from your own copy of the game.\n\n\
             Choose a disc image of Melee, NTSC version 1.02 (game ID GALE01, revision 2), \
             such as an .iso or .rvz file. ssbm-rs remembers it, so you only do this once."
        ))
        .set_buttons(MessageButtons::OkCancelCustom(BROWSE.to_owned(), "Quit".to_owned()))
        .show();
    matches!(result, MessageDialogResult::Ok)
        || matches!(&result, MessageDialogResult::Custom(s) if s == BROWSE)
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
    let before = std::fs::read_to_string(&remembered)
        .ok()
        .map(|s| PathBuf::from(s.trim()));
    let mut disc = before.clone().filter(|p| ssbm_disc::Disc::open(p).is_ok());
    // Until it has one: a word on what it needs, then the file browser.
    let mut note = match &before {
        Some(p) if disc.is_none() => format!(
            "The disc image ssbm-rs played before can't be opened anymore:\n{}\n\n",
            p.display()
        ),
        _ => String::new(),
    };
    while disc.is_none() {
        if !welcome(&note) {
            std::process::exit(0);
        }
        let Some(path) = FileDialog::new()
            .set_title("Choose your Super Smash Bros. Melee disc image (NTSC 1.02)")
            .add_filter("Disc images", &["iso", "gcm", "rvz", "ciso", "gcz", "wbfs", "nfs"])
            .add_filter("All files", &["*"])
            .pick_file()
        else {
            continue;
        };
        match ssbm_disc::Disc::open(&path) {
            Ok(_) => disc = Some(path),
            Err(e) => {
                note = format!("{} isn't a disc ssbm-rs can play ({e}).\n\n", path.display())
            }
        }
    }
    let disc = disc.unwrap();
    // A new card comes formatted.
    let card = folder.join("card.raw");
    if !card.exists() {
        let _ = std::fs::create_dir_all(&folder);
        if let Err(e) = std::fs::write(&card, ssbm_sdk::formatted_card()) {
            error(&format!("Couldn't make a memory card at {}: {e}", card.display()));
        }
    }
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
