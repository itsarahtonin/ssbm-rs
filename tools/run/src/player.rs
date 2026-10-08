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
    // On Linux the dialog is zenity's, whose Quit answers Custom: plain Cancel means zenity
    // couldn't run, so go on to the disc picker (the desktop's portal) rather than quit unasked.
    matches!(result, MessageDialogResult::Ok)
        || matches!(&result, MessageDialogResult::Custom(s) if s == BROWSE)
        || cfg!(target_os = "linux") && matches!(result, MessageDialogResult::Cancel)
}

/// The arguments a double-click means, or None when given some: the disc (the one remembered,
/// or one asked for), a window, and the card in the settings folder. Exits if asked for a disc
/// and given none.
pub fn args() -> Option<Vec<String>> {
    if std::env::args().len() > 1 {
        return None;
    }
    // The card and the recording first: the window may be closed rather than the message
    // answered.
    std::panic::set_hook(Box::new(|info| {
        ssbm_sdk::flush_cards_after_panic();
        crate::inputs::flush();
        error(&format!("ssbm-rs stopped: {info}"));
    }));
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
    let mut args = vec![
        std::env::args().next().unwrap_or_default(),
        disc.to_string_lossy().into_owned(),
        "--window".to_owned(),
        "--card".to_owned(),
        card.to_string_lossy().into_owned(),
    ];
    // A recordings folder in the settings folder asks for each session's controllers to be
    // kept there (inputs.rs), named for the time it started (UTC), to play back with --inputs.
    let recordings = folder.join("recordings");
    if recordings.is_dir() {
        let name = format!("{}.inputs", session_name());
        args.extend(["--record".to_owned(), recordings.join(name).to_string_lossy().into_owned()]);
    }
    Some(args)
}

/// The time now as YYYY-MM-DD_HH-MM-SS, in UTC.
fn session_name() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rest) = ((secs / 86_400) as i64, secs % 86_400);
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}_{:02}-{:02}-{:02}",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}
