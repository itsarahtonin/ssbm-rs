// SPDX-License-Identifier: GPL-3.0-or-later

//! Prints one Gecko code from a replay's code list, by injection address, or lists every code
//! with whether playback leaves it out, or with `--words` the lines playback applies.
//!
//! `cargo run -p ssbm-slippi --example gecko_code -- replay.slp [8006DA34 | --words]`

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&a[1]).unwrap();
    let replay = ssbm_slippi::Replay::parse(&bytes).unwrap();
    let src = &replay.gecko_codes;
    if a.get(2).map(String::as_str) == Some("--words") {
        // Every code line playback keeps, for scanning.
        for c in ssbm_slippi::gecko::lines(&ssbm_slippi::gecko::playback_list(src)) {
            println!("{:08X} {:08X}", c.0, c.1);
        }
        return;
    }
    let want = a.get(2).map(|s| u32::from_str_radix(s, 16).unwrap());
    let word = |at: usize| u32::from_be_bytes(src[at..at + 4].try_into().unwrap());
    let mut at = 0;
    while at + 8 <= src.len() {
        let kind = src[at] & 0xFE;
        let addr = (word(at) & 0x01FF_FFFF) | 0x8000_0000;
        let len = match kind {
            0xC0 | 0xC2 => 8 + 8 * word(at + 4) as usize,
            0x08 => 16,
            0x06 => 8 + ((word(at + 4) as usize + 7) & !7),
            _ => 8,
        };
        match want {
            Some(want) if addr == want => {
                for i in (at..at + len).step_by(8) {
                    println!("{:08X} {:08X}", word(i), word(i + 4));
                }
                return;
            }
            Some(_) => {}
            None => println!(
                "{addr:08X} {kind:02X} {len:5} bytes{}",
                if ssbm_slippi::denylist::DENYLIST.contains(&addr) {
                    "  left out"
                } else {
                    ""
                }
            ),
        }
        at += len;
    }
    if let Some(want) = want {
        println!("no code at {want:08X}");
    }
}
