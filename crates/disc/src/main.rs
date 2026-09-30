// SPDX-License-Identifier: GPL-3.0-or-later

//! `ssbm-disc <image>`: verifies a Melee disc image and loads `main.dol` into memory.
//! `ssbm-disc extract <image> <file|main.dol> <out>`: copies one file off the disc.

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use ssbm_disc::{DISC_SHA1, Disc};
use ssbm_mem::Mem;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let result = match args.as_slice() {
        [cmd, image, file, out] if cmd == "extract" => {
            extract(Path::new(image), &file.to_string_lossy(), Path::new(out)).map(|()| true)
        }
        [image] => verify(Path::new(image)),
        _ => {
            eprintln!("usage: ssbm-disc <image> | ssbm-disc extract <image> <file|main.dol> <out>");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn extract(image: &Path, file: &str, out: &Path) -> ssbm_disc::Result<()> {
    let disc = Disc::open(image)?;
    let data = if file == "main.dol" {
        disc.main_dol()?.raw().to_vec()
    } else {
        disc.read_file(file)?
    };
    std::fs::write(out, &data)?;
    println!("wrote {} bytes to {}", data.len(), out.display());
    Ok(())
}

fn verify(path: &Path) -> ssbm_disc::Result<bool> {
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
