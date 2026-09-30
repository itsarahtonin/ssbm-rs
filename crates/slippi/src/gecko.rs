// SPDX-License-Identifier: GPL-3.0-or-later

//! Gecko codes: the code lists in Dolphin's game `.ini` format, the GCT image Slippi's
//! bootloader asks for, and native application of the few code types the bootloader uses.

use ssbm_rt::Ctx;

/// One named code: its lines as (address word, data word) pairs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Code {
    pub name: String,
    pub lines: Vec<(u32, u32)>,
}

/// The codes in `[Gecko]` that `[Gecko_Enabled]` enables, in file order.
pub fn parse_ini(ini: &str) -> Vec<Code> {
    let (mut codes, enabled) = parse(ini, "");
    codes.retain(|c| enabled.contains(&c.name));
    codes
}

/// Every code in a bare code list, such as Slippi's bootloader.
pub fn parse_list(text: &str) -> Vec<Code> {
    parse(text, "[Gecko]").0
}

fn parse(text: &str, start_section: &str) -> (Vec<Code>, Vec<String>) {
    let mut section = start_section;
    let mut codes: Vec<Code> = Vec::new();
    let mut enabled = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            section = line;
            continue;
        }
        // Dolphin's INI reader takes a line with '=' anywhere, even in a trailing comment, as
        // a key/value pair and drops it from the code. Slippi's playback set relies on this:
        // one of its codes, annotated "... = ...", would break the game if applied.
        if line.contains('=') && !line.starts_with(['$', '+', '*']) {
            continue;
        }
        match section {
            "[Gecko]" => {
                if let Some(name) = line.strip_prefix('$') {
                    // "$Name [authors]": Dolphin keys codes by the name alone.
                    let name = name.split(" [").next().unwrap_or(name).trim();
                    codes.push(Code {
                        name: name.to_owned(),
                        lines: Vec::new(),
                    });
                } else if let Some(code) = codes.last_mut() {
                    let body = line.split('#').next().unwrap_or("").trim();
                    let mut words = body.split_whitespace();
                    if let (Some(a), Some(d), None) = (words.next(), words.next(), words.next())
                        && let (Ok(a), Ok(d)) =
                            (u32::from_str_radix(a, 16), u32::from_str_radix(d, 16))
                    {
                        code.lines.push((a, d));
                    }
                }
            }
            "[Gecko_Enabled]" => {
                if let Some(name) = line.strip_prefix('$') {
                    enabled.push(name.trim().to_owned());
                }
            }
            _ => {}
        }
    }
    (codes, enabled)
}

/// A GCT image, as Dolphin's `Gecko::GenerateGct` builds it: header, code lines, terminator.
pub fn gct(codes: &[Code]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut push = |a: u32, d: u32| {
        out.extend_from_slice(&a.to_be_bytes());
        out.extend_from_slice(&d.to_be_bytes());
    };
    push(0x00D0_C0DE, 0x00D0_C0DE);
    for code in codes {
        for &(a, d) in &code.lines {
            push(a, d);
        }
    }
    push(0xFF00_0000, 0);
    out
}

/// Writes `codes` to memory at `list` as a code list and applies them the way the code handler
/// would: `04` writes a word, `C2` branches from its address into its code and back.
pub fn apply(ctx: &Ctx, codes: &[Code], list: u32) {
    let lines: Vec<(u32, u32)> = codes.iter().flat_map(|c| c.lines.clone()).collect();
    for (i, &(a, d)) in lines.iter().enumerate() {
        ctx.write_u32(list + 8 * i as u32, a);
        ctx.write_u32(list + 8 * i as u32 + 4, d);
    }
    let mut i = 0;
    while i < lines.len() {
        let (a, d) = lines[i];
        let target = 0x8000_0000 | (a & 0x01FF_FFFF);
        match a >> 24 & 0xFE {
            0x04 => {
                ctx.write_u32(target, d);
                i += 1;
            }
            0xC2 => {
                let at = list + 8 * i as u32;
                let n = d;
                let body = at + 8;
                ctx.write_u32(target, branch(target, body));
                // The body's last word branches back after the injection point.
                let last = at + 8 * n + 4;
                ctx.write_u32(last, branch(last, target + 4));
                i += 1 + n as usize;
            }
            t => panic!("unsupported Gecko code type {t:02X} at {a:08X}"),
        }
    }
}

/// Game addresses a code list writes, as (address, length): the words `04`, `02` and `00`
/// codes write, the strings `06` codes write, and the branch a `C2` code puts at its address.
/// Other code types (conditionals, pointer codes) write nothing directly.
pub fn targets(lines: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let (a, d) = lines[i];
        let target = 0x8000_0000 | (a & 0x01FF_FFFF);
        match a >> 24 & 0xFE {
            0x00 => out.push((target, 1)),
            0x02 => out.push((target, 2 * ((d >> 16) + 1))),
            0x04 => out.push((target, 4)),
            0x06 => {
                out.push((target, d));
                i += (d as usize).div_ceil(8);
            }
            0x08 => i += 1,
            0xC0 => i += d as usize,
            0xC2 => {
                out.push((target, 4));
                i += d as usize;
            }
            0xF0 | 0xFE => break,
            _ => {}
        }
        i += 1;
    }
    out
}

/// The replay's code list as playback applies it: without the codes it leaves out.
pub fn playback_list(replay_codes: &[u8]) -> Vec<u8> {
    crate::device::filter_codes(replay_codes)
}

/// A code list's lines from its raw bytes. The `F0`/`FF` terminator only ends a list where a
/// code starts, so it is left for [`targets`] to find: inside a `C2` code's instructions, a
/// word such as `FF800890` (`fmr`) is an instruction.
pub fn lines(raw: &[u8]) -> Vec<(u32, u32)> {
    raw.as_chunks::<8>()
        .0
        .iter()
        .map(|c| {
            (
                u32::from_be_bytes([c[0], c[1], c[2], c[3]]),
                u32::from_be_bytes([c[4], c[5], c[6], c[7]]),
            )
        })
        .collect()
}

/// `b to`, placed at `from`.
fn branch(from: u32, to: u32) -> u32 {
    0x4800_0000 | (to.wrapping_sub(from) & 0x03FF_FFFC)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INI: &str = "\
[Gecko]
$Alpha [someone]
04001000 00000001 #note
*a note
$Beta
C2002000 00000001
60000000 00000000
$Gamma
04003000 00000002
[Gecko_Enabled]
$Alpha
$Beta
";

    #[test]
    fn parses_enabled_codes_in_order() {
        let codes = parse_ini(INI);
        let names: Vec<_> = codes.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "Beta"]);
        assert_eq!(codes[1].lines, [(0xC200_2000, 1), (0x6000_0000, 0)]);
    }

    #[test]
    fn gct_has_header_and_terminator() {
        let image = gct(&parse_ini(INI));
        assert_eq!(image.len(), 8 * (1 + 3 + 1));
        assert_eq!(&image[..8], &[0, 0xD0, 0xC0, 0xDE, 0, 0xD0, 0xC0, 0xDE]);
        assert_eq!(&image[image.len() - 8..image.len() - 4], &[0xFF, 0, 0, 0]);
    }

    #[test]
    fn applies_writes_and_injections() {
        let ctx = Ctx::new();
        apply(&ctx, &parse_ini(INI), 0x8000_1800);
        assert_eq!(ctx.read_u32(0x8000_1000), 1);
        // Branch from 0x80002000 to the body at 0x80001810, and back to 0x80002004.
        assert_eq!(ctx.read_u32(0x8000_2000), branch(0x8000_2000, 0x8000_1810));
        assert_eq!(ctx.read_u32(0x8000_1814), branch(0x8000_1814, 0x8000_2004));
    }

    #[test]
    fn targets_look_past_instructions_that_look_like_terminators() {
        let mut raw = Vec::new();
        for (a, d) in [
            (0xC206_A880, 1),
            (0xFF80_0890, 0x6000_0000),
            (0x0400_1000, 1),
            (0xFF00_0000, 0),
            (0x0400_2000, 2),
        ] {
            raw.extend_from_slice(&u32::to_be_bytes(a));
            raw.extend_from_slice(&u32::to_be_bytes(d));
        }
        assert_eq!(
            targets(&lines(&raw)),
            [(0x8006_A880, 4), (0x8000_1000, 4)]
        );
    }
}
