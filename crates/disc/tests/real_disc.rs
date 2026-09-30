// SPDX-License-Identifier: GPL-3.0-or-later

//! Runs only when `SSBM_DISC` points at a disc image, so no game data is needed in CI.

use ssbm_disc::{Disc, SectionKind};
use ssbm_mem::Mem;

fn disc() -> Option<Disc> {
    let path = std::env::var_os("SSBM_DISC")?;
    Some(Disc::open(path).expect("SSBM_DISC should be Melee NTSC 1.02"))
}

#[test]
fn loads_main_dol_at_the_decomp_addresses() {
    let Some(disc) = disc() else {
        eprintln!("SSBM_DISC is not set; skipping");
        return;
    };
    let dol = disc.main_dol().unwrap();
    // The decomp's symbols place .init (starting with memset) at 0x80003100.
    let init = dol.sections[0];
    assert_eq!((init.kind, init.addr), (SectionKind::Text, 0x8000_3100));

    let mem = Mem::new();
    dol.load_into(&mem).unwrap();
    for section in &dol.sections {
        let mut loaded = vec![0; section.size as usize];
        mem.read_bytes(section.addr, &mut loaded).unwrap();
        assert!(
            loaded == dol.section_data(section),
            "section at {:#010X}",
            section.addr
        );
    }
}

#[test]
fn reads_files_from_the_file_system() {
    let Some(disc) = disc() else {
        eprintln!("SSBM_DISC is not set; skipping");
        return;
    };
    let data = disc.read_file("PlFxNr.dat").unwrap();
    // HSD archives start with their own file size.
    assert_eq!(
        u32::from_be_bytes(data[..4].try_into().unwrap()) as usize,
        data.len()
    );
    assert!(disc.read_file("NotARealFile.dat").is_err());
}
