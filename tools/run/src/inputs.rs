//! Recording the controllers a session's game reads, and playing them back. `--record FILE`
//! keeps what every `PADRead` reports, with the settings the game's course depends on: the disc
//! rate, whether the DSP mixes sound and, beside it as FILE.card, the memory card as the
//! session found it. `--inputs FILE` hands the game those controllers read by read and takes
//! those settings, so a headless run (with `--lockstep`, say) plays the session again; it stops
//! a second after the recording runs out. The runtime runs in virtual time, so the game reads
//! the same things in the same order whatever pace the session was played at.
//!
//! The file is "SSBMINP1", a version (u32), the disc rate in bytes a second (u64, 0 for none)
//! and flags (u8: 1 sound mixed, 2 a card beside it), all little-endian, then a byte per read:
//! 0 for the same controllers as the read before, or 1 followed by the four controllers, 11
//! bytes each (connected, buttons big-endian, the sticks, triggers and analog buttons).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ssbm_rt::Stop;
use ssbm_sdk::{PadStatus, Sdk, TB_HZ};

const MAGIC: &[u8; 8] = b"SSBMINP1";
const VERSION: u32 = 1;
const SOUND: u8 = 1;
const CARD: u8 = 2;
const PAD_BYTES: usize = 11;

/// The settings a recorded session ran with.
pub struct Settings {
    pub disc_rate: Option<u64>,
    pub sound: bool,
    /// The card the session started with, if it had one.
    pub card: Option<PathBuf>,
}

/// The open recording, flushed as a run ends.
static WRITER: Mutex<Option<BufWriter<File>>> = Mutex::new(None);

/// Where a recording keeps the card its session started with.
pub fn card_path(path: &Path) -> PathBuf {
    path.with_extension("card")
}

/// Copies the card a session starts with beside its recording, before the game opens it.
pub fn keep_card(path: &Path, card: &Path) {
    if card.exists() {
        std::fs::copy(card, card_path(path))
            .unwrap_or_else(|e| panic!("keeping {}: {e}", card.display()));
    }
}

/// Records every `PADRead` into `path`, with the settings the run has now.
pub fn record(sdk: &Sdk, path: &Path) {
    let mut out =
        BufWriter::new(File::create(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())));
    let flags = (if sdk.dev.mix_audio.get() { SOUND } else { 0 })
        | (if card_path(path).exists() { CARD } else { 0 });
    out.write_all(MAGIC).unwrap();
    out.write_all(&VERSION.to_le_bytes()).unwrap();
    out.write_all(&sdk.hw.disc_rate().unwrap_or(0).to_le_bytes())
        .unwrap();
    out.write_all(&[flags]).unwrap();
    *WRITER.lock().unwrap() = Some(out);
    let mut last: Option<[PadStatus; 4]> = None;
    let mut reads = 0u64;
    sdk.dev.tap_pads(Box::new(move |_, pads| {
        let mut guard = WRITER.lock().unwrap();
        let Some(out) = guard.as_mut() else { return };
        let written = if last.as_ref() == Some(pads) {
            out.write_all(&[0])
        } else {
            last = Some(*pads);
            let mut bytes = vec![1u8];
            for p in pads.iter() {
                bytes.extend_from_slice(&encode(p));
            }
            out.write_all(&bytes)
        };
        written.expect("writing the recording");
        reads += 1;
        // About two seconds' worth at a time, so a session cut short keeps nearly all of it.
        if reads % 120 == 0 {
            let _ = out.flush();
        }
    }));
}

/// Writes what the recording holds yet, as a run ends or stops on a panic. A panic that left the
/// recording mid-read (holding it) skips this rather than wait on itself.
pub fn flush() {
    let mut guard = match WRITER.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return,
    };
    if let Some(out) = guard.as_mut() {
        let _ = out.flush();
    }
}

/// Reads a recording: its settings, and the controllers of each read.
pub fn load(path: &Path) -> (Settings, Vec<[PadStatus; 4]>) {
    let data = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert!(
        data.len() >= 21 && &data[..8] == MAGIC,
        "{}: not a controller recording",
        path.display()
    );
    let version = u32::from_le_bytes(data[8..12].try_into().unwrap());
    assert!(
        version == VERSION,
        "{}: recording version {version}",
        path.display()
    );
    let rate = u64::from_le_bytes(data[12..20].try_into().unwrap());
    let flags = data[20];
    let mut reads = Vec::new();
    let mut last = [PadStatus::default(); 4];
    let mut at = 21;
    while at < data.len() {
        match data[at] {
            0 => at += 1,
            1 if at + 1 + 4 * PAD_BYTES <= data.len() => {
                for (i, pad) in last.iter_mut().enumerate() {
                    *pad = decode(&data[at + 1 + i * PAD_BYTES..][..PAD_BYTES]);
                }
                at += 1 + 4 * PAD_BYTES;
            }
            // A session cut short mid-write leaves a partial last read.
            1 => break,
            b => panic!("{}: bad read marker {b} at byte {at}", path.display()),
        }
        reads.push(last);
    }
    let settings = Settings {
        disc_rate: (rate > 0).then_some(rate),
        sound: flags & SOUND != 0,
        card: (flags & CARD != 0).then(|| card_path(path)),
    };
    (settings, reads)
}

/// Hands the game the recorded controllers, read by read; a second after they run out, the
/// run stops.
pub fn play(sdk: &Sdk, reads: Vec<[PadStatus; 4]>) {
    let total = reads.len();
    let mut next = 0usize;
    sdk.dev.tap_pads(Box::new(move |ctx, pads| {
        if let Some(r) = reads.get(next) {
            *pads = *r;
        } else if let Some(r) = reads.last() {
            *pads = *r;
        }
        next += 1;
        if next == total + 1 {
            eprintln!("inputs: the recording's {total} reads are played; stopping in a second");
            ctx.ext::<Sdk>()
                .after(ctx, TB_HZ, |_| std::panic::panic_any(Stop));
        }
    }));
}

fn encode(p: &PadStatus) -> [u8; PAD_BYTES] {
    let [b0, b1] = p.button.to_be_bytes();
    [
        p.connected as u8,
        b0,
        b1,
        p.stick_x as u8,
        p.stick_y as u8,
        p.substick_x as u8,
        p.substick_y as u8,
        p.trigger_l,
        p.trigger_r,
        p.analog_a,
        p.analog_b,
    ]
}

fn decode(b: &[u8]) -> PadStatus {
    PadStatus {
        connected: b[0] != 0,
        button: u16::from_be_bytes([b[1], b[2]]),
        stick_x: b[3] as i8,
        stick_y: b[4] as i8,
        substick_x: b[5] as i8,
        substick_y: b[6] as i8,
        trigger_l: b[7],
        trigger_r: b[8],
        analog_a: b[9],
        analog_b: b[10],
    }
}
