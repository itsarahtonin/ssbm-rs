// SPDX-License-Identifier: GPL-3.0-or-later

//! The Slippi ruleset's sessions (`--ruleset slippi`), which melee-hd's oracle compares with its
//! own from power-on. From power-on the run applies the general codes Slippi's playback applies
//! (Unlock All Characters and Stages, Disable Special Messages, Disable Trophy Messages), Skip
//! Memcard Prompt from Slippi Dolphin's netplay set, which passes the boot's "No memory card"
//! question, Slippi's 27 gameplay codes (`ssbm_slippi::GAMEPLAY`), so every mode runs Slippi's
//! rules, the single-player ones included, and melee-hd's C-stick fix, so the C-stick smashes in
//! the single-player modes as in VS and doesn't zoom the camera. The run has Slippi Dolphin's floating point and no
//! card in slot A, and a recording keeps the ruleset (inputs.rs), so its playback runs the same
//! way. Slippi Dolphin's menus, online scene and other codes aren't part of it.

use ssbm_rt::Ctx;
use ssbm_slippi::gecko::{self, Code};

/// The codes before the gameplay codes, with Slippi 3.19.1's bytes: the first 16 lines of the
/// playback set's General Codes (`ssbm_slippi::PLAYBACK_INI`), then Skip Memcard Prompt from the
/// netplay set (`Sys/GameSettings/GALE01r2.ini`).
const GENERAL: &str = "\
$Unlock All Characters and Stages [Datel]
0415EE98 38600001
0415EDDC 38600001
04164B14 38600001
041648F4 38600001
0415EE4C 38600001
0415EE14 38600001
0415D968 38600001
0415D9D8 38600001
0417229C 38600000
041737B0 38600000
04164658 38600001
041644E8 38600001
0430490C 38600063
043044F0 38600001
$Disable Special Messages [Most]
0415D94C 4E800020
$Disable Trophy Messages [Achilles]
0415D984 4E800020
$Skip Memcard Prompt
C21AF6F4 00000006
2C1D000F 41820010
2C1D000D 41820008
48000014 3D80801B
618C01AC 7D8903A6
4E800420 2C1D0000
60000000 00000000
";

/// melee-hd's C-stick fix: each line turns a `bl gm_IsCurrently1PMode_inline` into `li r3, 0`,
/// whose caller then takes VS's path: Fighter_procInput's computer and human C-stick branches,
/// and Camera_8002B0E0's C-stick zoom. The function's other callers keep its answer. Slippi
/// Dolphin's netplay set has C-Stick in Single Player (`0416B480 60000000`) instead, which
/// changes every caller.
const CSTICK: &str = "$C-stick in single player (melee-hd)
0406AE04 38600000
0406AE9C 38600000
0402B0F8 38600000
";

/// Where the codes' list goes: the low memory Dolphin's code handler uses, 0x80001800 to
/// 0x80003000. A `C2` code's injected instructions are its lines after the first, in the list:
/// Skip Memcard Prompt is its lines 17 to 23 (from 0x80001880), the gameplay codes follow from
/// 0x800018B8, and the C-stick fix ends the list.
const LIST: u32 = 0x8000_1800;
const LIST_END: u32 = 0x8000_3000;

/// The ruleset's codes, in the order they're applied.
pub fn codes() -> Vec<Code> {
    let mut codes = gecko::parse_list(GENERAL);
    codes.extend(gecko::parse_list(ssbm_slippi::GAMEPLAY));
    codes.extend(gecko::parse_list(CSTICK));
    codes
}

/// The ruleset's code lines, in the order they're applied.
pub fn lines() -> Vec<(u32, u32)> {
    codes().into_iter().flat_map(|c| c.lines).collect()
}

/// Applies the codes, once the game is loaded and before it starts.
pub fn apply(ctx: &Ctx) {
    let codes = codes();
    let lines: usize = codes.iter().map(|c| c.lines.len()).sum();
    assert!(
        LIST + 8 * lines as u32 <= LIST_END,
        "the ruleset's codes overrun low memory"
    );
    gecko::apply(ctx, &codes, LIST);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_playback_sets_general_codes_lead() {
        let playback = gecko::parse_ini(ssbm_slippi::PLAYBACK_INI);
        let general = playback
            .iter()
            .find(|c| c.name == "Required: General Codes")
            .expect("the playback set's General Codes");
        assert_eq!(lines()[..16], general.lines[..16]);
    }

    #[test]
    fn the_codes_are_4_general_27_gameplay_and_the_c_stick() {
        let codes = codes();
        let names: Vec<&str> = codes.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names[..4],
            [
                "Unlock All Characters and Stages",
                "Disable Special Messages",
                "Disable Trophy Messages",
                "Skip Memcard Prompt",
            ]
        );
        assert_eq!(codes.len(), 4 + 27 + 1);
        let lines = lines();
        assert_eq!(lines[16], (0xC21A_F6F4, 6));
        assert_eq!(lines[23], (0xC216_E510, 0x98));
        assert_eq!(lines.len(), 16 + 7 + 658 + 3);
        assert_eq!(
            lines[681..],
            [
                (0x0406_AE04, 0x3860_0000),
                (0x0406_AE9C, 0x3860_0000),
                (0x0402_B0F8, 0x3860_0000)
            ]
        );
        assert!(LIST + 8 * lines.len() as u32 <= LIST_END);
    }

    #[test]
    fn the_gameplay_codes_are_what_playback_keeps() {
        // Every gameplay code's injection is one playback keeps from a replay's code list.
        for c in gecko::parse_codes(
            &gecko::parse_list(ssbm_slippi::GAMEPLAY)
                .into_iter()
                .flat_map(|c| c.lines)
                .collect::<Vec<_>>(),
        ) {
            assert!(
                ssbm_slippi::denylist::DENYLIST
                    .binary_search(&c.addr)
                    .is_err(),
                "{:#010x}",
                c.addr
            );
        }
    }
}
