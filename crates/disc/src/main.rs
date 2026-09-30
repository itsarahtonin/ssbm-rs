// SPDX-License-Identifier: GPL-3.0-or-later

//! `ssbm-disc <image>`: verifies a Melee disc image and loads `main.dol` into memory.

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use ssbm_disc::{DISC_SHA1, Disc};
use ssbm_mem::Mem;

fn main() -> ExitCode {
    let Some(path) = std::env::args_os().nth(1) else {
        eprintln!("usage: ssbm-disc <disc image>");
        return ExitCode::from(2);
    };
    match run(Path::new(&path)) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(path: &Path) -> ssbm_disc::Result<bool> {
    let mut disc = Disc::open(path)?;
    println!("GALE01 revision 2, {} image", disc.format());

    let dol = disc.main_dol()?;
    let mem = Mem::new();
    dol.load_into(&mem)?;
    println!(
        "main.dol matches the decomp's SHA-1; loaded {} sections, entry {:#010X}",
        dol.sections.len(),
        dol.entry
    );

    let sha1 = disc.disc_sha1(|done, total| {
        eprint!("\rhashing disc: {:3}%", done * 100 / total.max(1));
        let _ = std::io::stderr().flush();
    })?;
    eprintln!();
    if sha1 == DISC_SHA1 {
        println!("disc SHA-1 matches Redump");
        Ok(true)
    } else {
        println!("disc SHA-1 is {sha1}; the Redump dump is {DISC_SHA1}, so this image is modified");
        Ok(false)
    }
}
