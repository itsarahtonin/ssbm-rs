// SPDX-License-Identifier: GPL-3.0-or-later

//! The ports of Slippi's code stubs are found for the codes playback bundles, as the generator
//! matched them.

use ssbm_game::playback::{Applied, find};
use ssbm_slippi::gecko;

#[test]
fn bundled_code_stubs_have_ports() {
    let lines: Vec<(u32, u32)> = gecko::parse_list(ssbm_slippi::BOOTLOADER)
        .into_iter()
        .chain(gecko::parse_ini(ssbm_slippi::PLAYBACK_INI))
        .flat_map(|c| c.lines)
        .collect();
    let codes = gecko::parse_codes(&lines);
    // Playback's helper functions, which its codes put behind stubs in low memory.
    let stubs: Vec<&gecko::Parsed> = codes
        .iter()
        .filter(|c| c.kind == 0xC2 && (0x8000_5500..0x8000_5700).contains(&c.addr))
        .collect();
    assert!(stubs.len() >= 7, "{} stubs", stubs.len());
    for c in stubs {
        let applied = [Applied {
            kind: c.kind,
            addr: c.addr,
            first: c.first,
            words: &c.words,
        }];
        assert!(find(c.addr, &applied).is_some(), "no port of the stub at {:#010x}", c.addr);
        // Another code there is not the one the port was made for.
        let mut words = c.words.clone();
        words[0] ^= 1;
        let other = [Applied {
            words: &words,
            ..applied[0]
        }];
        assert!(find(c.addr, &other).is_none());
    }
}
