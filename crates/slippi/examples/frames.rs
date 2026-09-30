// SPDX-License-Identifier: GPL-3.0-or-later

//! Lists every post-frame event for one port over a frame range, in file order, to show
//! frames that online rollback simulated more than once.
//!
//! `cargo run -p ssbm-slippi --example frames -- replay.slp port first last`

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&a[1]).unwrap();
    let (port, first, last): (u8, i32, i32) = (
        a[2].parse().unwrap(),
        a[3].parse().unwrap(),
        a[4].parse().unwrap(),
    );
    let raw = ssbm_slippi::replay::raw_events(&bytes).unwrap();
    for e in ssbm_slippi::replay::parse_events(raw).unwrap() {
        let p = &e.payload;
        if e.command != ssbm_slippi::replay::event::POST_FRAME || p[4] != port {
            continue;
        }
        let frame = i32::from_be_bytes(p[0..4].try_into().unwrap());
        if (first..=last).contains(&frame) {
            let w = |at: usize| u32::from_be_bytes(p[at..at + 4].try_into().unwrap());
            println!(
                "frame {frame}: state {:04X} pos ({:08X}, {:08X}) self ({:08X}, {:08X}, ground {:08X}) kb ({:08X}, {:08X}) air {} flags {:02X?}",
                u16::from_be_bytes([p[7], p[8]]),
                w(0x9),
                w(0xD),
                w(0x34),
                w(0x38),
                w(0x44),
                w(0x3C),
                w(0x40),
                p[0x2E],
                &p[0x25..0x2A]
            );
        }
    }
}
