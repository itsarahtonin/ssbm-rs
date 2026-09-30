// SPDX-License-Identifier: GPL-3.0-or-later

//! Prints one Gecko code from a replay's code list, by injection address.
//!
//! `cargo run -p ssbm-slippi --example gecko_code -- replay.slp 8006DA34`

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&a[1]).unwrap();
    let want = u32::from_str_radix(&a[2], 16).unwrap();
    let replay = ssbm_slippi::Replay::parse(&bytes).unwrap();
    let src = &replay.gecko_codes;
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
        if addr == want {
            for i in (at..at + len).step_by(8) {
                println!("{:08X} {:08X}", word(i), word(i + 4));
            }
            return;
        }
        at += len;
    }
    println!("no code at {want:08X}");
}
