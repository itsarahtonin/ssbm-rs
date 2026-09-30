// SPDX-License-Identifier: GPL-3.0-or-later

//! `replay-sort <replay dir> [--out DIR] [--smoke N]`
//!
//! Indexes Slippi replays by what the replay oracle must reproduce (platform, controller fixes,
//! frozen Stadium, Gecko code set) and picks a diverse smoke set from the most common profile.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use peppi::game::immutable::Game;
use peppi::game::{DashBack, PlayerType, ShieldDrop};
use peppi::io::slippi;
use ssbm_data::{character::External, stage::Stage};

/// Everything a replay needs from the harness besides the game itself.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Profile {
    platform: String,
    controller_fix: &'static str,
    frozen_stadium: bool,
    gecko_codes: String,
}

struct Entry {
    path: PathBuf,
    version: String,
    profile: Profile,
    stage: String,
    characters: Vec<String>,
    humans: usize,
    teams: bool,
    frames: i64,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let (mut dir, mut out, mut smoke) = (None, PathBuf::from("local/replays"), 40);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--out") => {
                out = args
                    .next()
                    .map(PathBuf::from)
                    .ok_or("--out needs a directory")?
            }
            Some("--smoke") => {
                smoke = args
                    .next()
                    .and_then(|s| s.to_str()?.parse().ok())
                    .ok_or("--smoke needs a number")?
            }
            _ => dir = Some(PathBuf::from(arg)),
        }
    }
    let dir = dir.ok_or("usage: replay-sort <replay dir> [--out DIR] [--smoke N]")?;

    let mut paths = Vec::new();
    collect_replays(&dir, &mut paths).map_err(|e| format!("reading {}: {e}", dir.display()))?;
    paths.sort();

    let mut entries = Vec::new();
    let mut unreadable = 0;
    for path in paths {
        match read_entry(&path) {
            Ok(entry) => entries.push(entry),
            Err(err) => {
                eprintln!("skipping {}: {err}", path.display());
                unreadable += 1;
            }
        }
    }

    fs::create_dir_all(&out).map_err(|e| format!("creating {}: {e}", out.display()))?;
    write_index(&out.join("index.csv"), &entries).map_err(|e| format!("writing index: {e}"))?;
    let (profile, chosen) = pick_smoke_set(&entries, smoke);
    write_smoke_set(&out.join("smoke-set.txt"), profile.as_ref(), &chosen)
        .map_err(|e| format!("writing smoke set: {e}"))?;

    print_summary(&entries, unreadable, profile.as_ref(), chosen.len(), &out);
    Ok(())
}

fn collect_replays(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_replays(&path, out)?;
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("slp"))
        {
            out.push(path);
        }
    }
    Ok(())
}

fn read_entry(path: &Path) -> Result<Entry, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    // A full parse: peppi's `skip_frames` also skips the Gecko code list.
    let game = slippi::read(BufReader::new(file), None).map_err(|e| e.to_string())?;
    Ok(entry_from_game(path, &game))
}

fn entry_from_game(path: &Path, game: &Game) -> Entry {
    let start = &game.start;
    let metadata = game.metadata.as_ref();
    let platform = metadata
        .and_then(|m| m.get("playedOn"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_owned();
    let frames = metadata
        .and_then(|m| m.get("lastFrame"))
        .and_then(|v| v.as_i64())
        .unwrap_or(-1);

    let fixes: BTreeSet<&'static str> = start
        .players
        .iter()
        .filter_map(|p| p.ucf)
        .flat_map(|ucf| {
            [
                ucf.dash_back.map(dash_back_name),
                ucf.shield_drop.map(shield_drop_name),
            ]
        })
        .flatten()
        .collect();
    let controller_fix = match fixes.len() {
        0 => "off",
        1 => fixes.into_iter().next().unwrap(),
        _ => "mixed",
    };

    let gecko_hooks = game.gecko_codes.as_ref().map(|codes| {
        // Bytes past `actual_size` are padding from the recorder.
        let len = (codes.actual_size as usize).min(codes.bytes.len());
        gecko_hooks(&codes.bytes[..len])
    });
    let gecko_codes = match &gecko_hooks {
        Some(hooks) => {
            let signature: Vec<u8> = hooks
                .iter()
                .flat_map(|(kind, addr)| std::iter::once(*kind).chain(addr.to_be_bytes()))
                .collect();
            format!("{}-hooks-{:08x}", hooks.len(), fnv1a(&signature) as u32)
        }
        None => "none".to_owned(),
    };

    Entry {
        path: path.to_owned(),
        version: start.slippi.version.to_string(),
        profile: Profile {
            platform,
            controller_fix,
            frozen_stadium: start.is_frozen_ps == Some(true),
            gecko_codes,
        },
        stage: Stage::try_from(start.stage)
            .map_or_else(|_| format!("stage {}", start.stage), |s| format!("{s:?}")),
        characters: start
            .players
            .iter()
            .map(|p| {
                External::try_from(p.character)
                    .map_or_else(|_| format!("char {}", p.character), |c| format!("{c:?}"))
            })
            .collect(),
        humans: start
            .players
            .iter()
            .filter(|p| p.r#type == PlayerType::Human)
            .count(),
        teams: start.is_teams,
        frames,
    }
}

fn dash_back_name(fix: DashBack) -> &'static str {
    match fix {
        DashBack::Ucf => "ucf",
        DashBack::Arduino => "arduino",
    }
}

fn shield_drop_name(fix: ShieldDrop) -> &'static str {
    match fix {
        ShieldDrop::Ucf => "ucf",
        ShieldDrop::Arduino => "arduino",
    }
}

/// The distinct (code type, address) pairs in a Gecko code list. Values are left out because
/// Slippi writes per-match data into some codes.
fn gecko_hooks(codes: &[u8]) -> Vec<(u8, u32)> {
    let word = |at: usize| {
        codes
            .get(at..at + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };
    let mut at = if word(0) == Some(0x00D0_C0DE) && word(4) == Some(0x00D0_C0DE) {
        8
    } else {
        0
    };
    let mut hooks = Vec::new();
    while let (Some(header), Some(arg)) = (word(at), word(at + 4)) {
        let kind = (header >> 24) as u8;
        let len = match kind & 0xFE {
            0xF0 => break,
            0x06 => 8 + (arg as usize).div_ceil(8) * 8,
            0x08 => 16,
            0xC0 | 0xC2 => 8 + arg as usize * 8,
            _ => 8,
        };
        hooks.push((kind, 0x8000_0000 | (header & 0x01FF_FFFF)));
        at += len;
    }
    hooks.sort_unstable();
    hooks.dedup();
    hooks
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xCBF2_9CE4_8422_2325, |hash, b| {
        (hash ^ u64::from(*b)).wrapping_mul(0x100_0000_01B3)
    })
}

/// Singles, both human, at least 30 seconds: the simplest games to reproduce first.
fn is_smoke_candidate(entry: &Entry) -> bool {
    entry.characters.len() == 2 && entry.humans == 2 && !entry.teams && entry.frames >= 1800
}

/// Picks from the most common profile, greedily covering new characters, matchups and stages.
fn pick_smoke_set(entries: &[Entry], count: usize) -> (Option<Profile>, Vec<&Entry>) {
    let mut by_profile: BTreeMap<&Profile, Vec<&Entry>> = BTreeMap::new();
    for entry in entries.iter().filter(|e| is_smoke_candidate(e)) {
        by_profile.entry(&entry.profile).or_default().push(entry);
    }
    let Some((profile, mut pool)) = by_profile.into_iter().max_by_key(|(_, group)| group.len())
    else {
        return (None, Vec::new());
    };

    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut chosen = Vec::new();
    while chosen.len() < count && !pool.is_empty() {
        let features = |e: &Entry| {
            let mut pair = e.characters.clone();
            pair.sort();
            let mut f: Vec<String> = e.characters.iter().map(|c| format!("char:{c}")).collect();
            f.push(format!("stage:{}", e.stage));
            f.push(format!("matchup:{}", pair.join("/")));
            f
        };
        let (best, _) = pool
            .iter()
            .enumerate()
            .max_by_key(|(i, e)| {
                (
                    features(e).iter().filter(|f| !seen.contains(*f)).count(),
                    std::cmp::Reverse(*i),
                )
            })
            .unwrap();
        let entry = pool.remove(best);
        seen.extend(features(entry));
        chosen.push(entry);
    }
    (Some(profile.clone()), chosen)
}

fn csv(field: &str) -> String {
    if field.contains([',', '"', '\n']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_owned()
    }
}

fn write_index(path: &Path, entries: &[Entry]) -> std::io::Result<()> {
    let mut w = BufWriter::new(File::create(path)?);
    writeln!(
        w,
        "path,slippi_version,platform,controller_fix,frozen_stadium,gecko_codes,stage,characters,humans,teams,frames"
    )?;
    for e in entries {
        let p = &e.profile;
        writeln!(
            w,
            "{},{},{},{},{},{},{},{},{},{},{}",
            csv(&e.path.display().to_string()),
            e.version,
            csv(&p.platform),
            p.controller_fix,
            p.frozen_stadium,
            p.gecko_codes,
            csv(&e.stage),
            csv(&e.characters.join(" vs ")),
            e.humans,
            e.teams,
            e.frames
        )?;
    }
    w.flush()
}

fn write_smoke_set(
    path: &Path,
    profile: Option<&Profile>,
    chosen: &[&Entry],
) -> std::io::Result<()> {
    let mut w = BufWriter::new(File::create(path)?);
    if let Some(p) = profile {
        writeln!(
            w,
            "# platform={} controller_fix={} frozen_stadium={} gecko_codes={}",
            p.platform, p.controller_fix, p.frozen_stadium, p.gecko_codes
        )?;
    }
    for e in chosen {
        writeln!(w, "{}", e.path.display())?;
    }
    w.flush()
}

fn print_summary(
    entries: &[Entry],
    unreadable: usize,
    profile: Option<&Profile>,
    chosen: usize,
    out: &Path,
) {
    println!("{} replays indexed, {unreadable} unreadable", entries.len());
    let tally = |label: &str, key: &dyn Fn(&Entry) -> String| {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for e in entries {
            *counts.entry(key(e)).or_default() += 1;
        }
        let parts: Vec<String> = counts.iter().map(|(k, n)| format!("{k} {n}")).collect();
        println!("  {label}: {}", parts.join(", "));
    };
    tally("platform", &|e| e.profile.platform.clone());
    tally("controller fix", &|e| e.profile.controller_fix.to_owned());
    tally("frozen stadium", &|e| e.profile.frozen_stadium.to_string());
    tally("gecko code sets", &|e| e.profile.gecko_codes.clone());
    tally("players", &|e| {
        format!("{}p/{} human", e.characters.len(), e.humans)
    });
    match profile {
        Some(p) => println!(
            "smoke set: {chosen} replays from the most common profile ({}, fix {}, frozen {}, codes {})",
            p.platform, p.controller_fix, p.frozen_stadium, p.gecko_codes
        ),
        None => println!("smoke set: no 2-player human games of 30 s or more"),
    }
    println!(
        "wrote {} and {}",
        out.join("index.csv").display(),
        out.join("smoke-set.txt").display()
    );
}
