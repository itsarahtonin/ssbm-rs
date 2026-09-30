// SPDX-License-Identifier: GPL-3.0-or-later

//! Lists the item events over a frame range, in file order.
//!
//! `cargo run -p ssbm-slippi --example items -- replay.slp first last`

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&a[1]).unwrap();
    let (first, last): (i32, i32) = (a[2].parse().unwrap(), a[3].parse().unwrap());
    let raw = ssbm_slippi::replay::raw_events(&bytes).unwrap();
    for e in ssbm_slippi::replay::parse_events(raw).unwrap() {
        let p = &e.payload;
        if e.command != ssbm_slippi::replay::event::ITEM {
            continue;
        }
        let w = |at: usize| u32::from_be_bytes(p[at..at + 4].try_into().unwrap());
        let frame = w(0) as i32;
        if (first..=last).contains(&frame) {
            println!(
                "frame {frame}: type {:04X} state {:02X} spawn {} owner {} pos ({:08X}, {:08X}) timer {:08X}",
                u16::from_be_bytes([p[4], p[5]]),
                p[6],
                w(0x21),
                p.get(0x29).copied().unwrap_or(0xFF) as i8,
                w(0x13),
                w(0x17),
                w(0x1D)
            );
        }
    }
}
