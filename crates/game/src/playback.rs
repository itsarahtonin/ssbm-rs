// SPDX-License-Identifier: GPL-3.0-or-later

//! Ports of the functions Slippi's playback codes patch, which `tools/c2rs/patched.py` makes
//! for the code sets it has seen, chosen for the codes a replay applies.

use crate::patched::{Code, ENTRIES, Entry, PATCHED, Patched};

/// A code as playback applies it: its type (0x04, 0xC2, ...), the address it writes, its first
/// line's second word and the words of its other lines.
#[derive(Clone, Copy, Debug)]
pub struct Applied<'a> {
    pub kind: u8,
    pub addr: u32,
    pub first: u32,
    pub words: &'a [u32],
}

/// The port of the function at `function` made for exactly `codes`, the codes that write in it
/// (or, for a stub outside any function, the code at its address), if one was made.
pub fn find(function: u32, codes: &[Applied]) -> Option<&'static Patched> {
    PATCHED.iter().find(|p| {
        p.function == function
            && p.sites.len() == codes.len()
            && p.sites.iter().all(|s| {
                codes
                    .iter()
                    .any(|c| c.addr == s.addr && matches(&s.code, c))
            })
    })
}

/// The ports of places in `codes`' injected instructions that other code calls, made for these
/// codes.
pub fn entries(codes: &[Applied]) -> Vec<&'static Entry> {
    ENTRIES
        .iter()
        .filter(|e| {
            codes
                .iter()
                .any(|c| c.addr == e.site && matches(&e.code, c))
        })
        .collect()
}

/// Where injected code a code's site branches to sits, once playback has placed it.
pub fn placed(ctx: &ssbm_rt::Ctx, site: u32) -> Option<u32> {
    let w = ctx.mem.read_u32(site).ok()?;
    (w & 0xFC00_0003 == 0x4800_0000)
        .then(|| site.wrapping_add((((w & 0x03FF_FFFC) << 6) as i32 >> 6) as u32))
}

/// Whether `c` is the code a port was made for. Injected code matches by the instructions it
/// runs, so the data among them, such as per-match settings, may differ.
fn matches(code: &Code, c: &Applied) -> bool {
    match *code {
        Code::Word(w) => c.kind == 0x04 && c.first == w,
        Code::Inject { len, words } => {
            c.kind == 0xC2
                && c.words.len() == len as usize
                && words
                    .iter()
                    .all(|&(off, w)| c.words.get(off as usize / 4) == Some(&w))
        }
    }
}
