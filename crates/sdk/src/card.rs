// SPDX-License-Identifier: GPL-3.0-or-later
// Behavior follows Dolphin's EXI memory card device (GPL-2.0-or-later).

//! A memory card in slot A or B, answering at the EXI level so the SDK's CARD library runs on
//! it as on a console: a 59-block flash card that takes the library's commands, raises its
//! interrupt when an erase or a program is done, and keeps its contents in a file.
//!
//! Like Dolphin's, the card reports itself unlocked from the start, so the library never runs
//! the DSP's unlock exchange, and takes the card's flash ID from the SRAM. Slot A's is
//! Dolphin's, which the card's format then carries; slot B's is the one a formatted card there
//! carries (`flash_id`, as Dolphin sets it), or Dolphin's for a blank one.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// The card's EXI ID: 4 Mbit, 8 KiB sectors, 4 bytes of read latency.
pub const ID: u32 = 0x0000_0004;
/// Bytes on a card of that size: 64 sectors, 5 for the system areas and 59 blocks.
pub const SIZE: usize = (4 << 20) / 8;
const SECTOR: usize = 0x2000;
const PAGE: usize = 0x80;

/// An empty card, formatted as the SDK's CARDFormat formats one in this console's slot A (its
/// SRAM flash ID is os::default_sram's), at time 0: what a new card holds. Blank flash instead
/// reads as a broken card, which the game offers to format.
pub fn formatted() -> Vec<u8> {
    const BLOCK: usize = SECTOR;
    let mut card = vec![0xFF; SIZE];
    // The ID: a serial from the flash ID, keyed by the format's time (0); counter bias,
    // language and the VI's DTV status (all 0 here); device 0; size in Mbit; ANSI encoding.
    let sram = crate::os::default_sram();
    let id = &mut card[..BLOCK];
    let mut rand: i64 = 0;
    for i in 0..12 {
        rand = rand.wrapping_mul(1_103_515_245).wrapping_add(12345) >> 16;
        id[i] = sram[20 + i].wrapping_add(rand as u8);
        rand = (rand.wrapping_mul(1_103_515_245).wrapping_add(12345) >> 16) & 0x7FFF;
    }
    id[12..32].fill(0);
    id[0x20..0x22].copy_from_slice(&0u16.to_be_bytes());
    id[0x22..0x24].copy_from_slice(&((SIZE * 8 >> 20) as u16).to_be_bytes());
    id[0x24..0x26].copy_from_slice(&0u16.to_be_bytes());
    let (sum, inv) = checksum(&id[..0x1FC]);
    id[0x1FC..0x1FE].copy_from_slice(&sum.to_be_bytes());
    id[0x1FE..0x200].copy_from_slice(&inv.to_be_bytes());
    // Two directories, empty, and two allocation tables, all blocks free.
    for i in 0..2 {
        let dir = &mut card[(1 + i) * BLOCK..(2 + i) * BLOCK];
        dir[0x1FFA..0x1FFC].copy_from_slice(&(i as u16).to_be_bytes());
        let (sum, inv) = checksum(&dir[..BLOCK - 4]);
        dir[0x1FFC..0x1FFE].copy_from_slice(&sum.to_be_bytes());
        dir[0x1FFE..].copy_from_slice(&inv.to_be_bytes());
    }
    for i in 0..2 {
        let fat = &mut card[(3 + i) * BLOCK..(4 + i) * BLOCK];
        fat.fill(0);
        fat[4..6].copy_from_slice(&(i as u16).to_be_bytes());
        fat[6..8].copy_from_slice(&((SIZE / BLOCK - 5) as u16).to_be_bytes());
        fat[8..10].copy_from_slice(&4u16.to_be_bytes());
        let (sum, inv) = checksum(&fat[4..]);
        fat[0..2].copy_from_slice(&sum.to_be_bytes());
        fat[2..4].copy_from_slice(&inv.to_be_bytes());
    }
    card
}

/// __CARDCheckSum: sums of the big-endian words and of their complements, 0xFFFF as 0.
fn checksum(bytes: &[u8]) -> (u16, u16) {
    let (mut sum, mut inv) = (0u16, 0u16);
    for w in bytes.chunks_exact(2) {
        let v = u16::from_be_bytes([w[0], w[1]]);
        sum = sum.wrapping_add(v);
        inv = inv.wrapping_add(!v);
    }
    let fix = |v: u16| if v == 0xFFFF { 0 } else { v };
    (fix(sum), fix(inv))
}

// Commands.
const CMD_ID: u8 = 0x00;
const CMD_READ_ARRAY: u8 = 0x52;
const CMD_SET_INTERRUPT: u8 = 0x81;
const CMD_READ_STATUS: u8 = 0x83;
const CMD_CLEAR_STATUS: u8 = 0x89;
const CMD_SECTOR_ERASE: u8 = 0xF1;
const CMD_PAGE_PROGRAM: u8 = 0xF2;
const CMD_CHIP_ERASE: u8 = 0xF4;

// Status bits.
const STATUS_READY: u8 = 0x01;
const STATUS_UNLOCKED: u8 = 0x40;
const STATUS_BUSY: u8 = 0x80;

/// What a command leaves to do once the host deselects the card.
pub(crate) enum Done {
    Nothing,
    /// An erase or a program, which takes time and then raises the card's interrupt.
    Busy,
}

pub struct Card {
    data: Arc<Mutex<Vec<u8>>>,
    /// What keeps the contents in a file between runs, if anything does.
    saver: Option<Arc<Saver>>,
    status: Cell<u8>,
    interrupts: Cell<bool>,
    /// Bytes the host has sent since it selected the card.
    sent: RefCell<Vec<u8>>,
    /// Bytes the card has sent back since then.
    answered: Cell<usize>,
}

impl Card {
    /// A card with the contents of `path`, or blank and saved there, or blank in memory only.
    pub fn new(path: Option<PathBuf>) -> std::io::Result<Self> {
        let data = match &path {
            Some(p) if p.exists() => {
                let mut d = std::fs::read(p)?;
                d.resize(SIZE, 0xFF);
                d
            }
            _ => vec![0xFF; SIZE],
        };
        let data = Arc::new(Mutex::new(data));
        Ok(Self {
            saver: path.map(|path| Saver::start(data.clone(), path)),
            data,
            status: Cell::new(STATUS_READY | STATUS_UNLOCKED),
            interrupts: Cell::new(false),
            sent: RefCell::default(),
            answered: Cell::new(0),
        })
    }

    /// The flash ID of the console slot that formatted the card, from the serial its ID block
    /// carries (CARDFormat's, undone as Dolphin's SetCardFlashID does): what the CARD library
    /// checks the serial against on a mount. None for a card whose ID block fails its checksum,
    /// such as a blank one.
    pub(crate) fn flash_id(&self) -> Option<[u8; 12]> {
        let data = self.data.lock().unwrap();
        let id = &data[..0x200];
        let word = |at: usize| u16::from_be_bytes([id[at], id[at + 1]]);
        if checksum(&id[..0x1FC]) != (word(0x1FC), word(0x1FE)) {
            return None;
        }
        let mut flash = [0u8; 12];
        let mut rand = i64::from_be_bytes(id[12..20].try_into().unwrap());
        for (i, f) in flash.iter_mut().enumerate() {
            rand = rand.wrapping_mul(1_103_515_245).wrapping_add(12345) >> 16;
            *f = id[i].wrapping_sub(rand as u8);
            rand = (rand.wrapping_mul(1_103_515_245).wrapping_add(12345) >> 16) & 0x7FFF;
        }
        Some(flash)
    }

    /// Whether the card raises its interrupt when an erase or a program finishes.
    pub(crate) fn interrupts(&self) -> bool {
        self.interrupts.get()
    }

    /// The host selects the card: a command starts.
    pub(crate) fn select(&self) {
        self.sent.borrow_mut().clear();
        self.answered.set(0);
    }

    /// Bytes from the host.
    pub(crate) fn write(&self, bytes: &[u8]) {
        self.sent.borrow_mut().extend_from_slice(bytes);
    }

    /// Bytes to the host.
    pub(crate) fn read(&self, n: usize) -> Vec<u8> {
        let sent = self.sent.borrow();
        let at = self.answered.get();
        self.answered.set(at + n);
        let Some(&cmd) = sent.first() else {
            return vec![0; n];
        };
        match cmd {
            CMD_ID => (at..at + n)
                .map(|i| ID.to_be_bytes().get(i).copied().unwrap_or(0))
                .collect(),
            CMD_READ_STATUS => vec![self.status.get(); n],
            CMD_READ_ARRAY if sent.len() >= 5 => {
                let start = address(&sent[1..5]) + at;
                let data = self.data.lock().unwrap();
                (start..start + n).map(|i| data[i % SIZE]).collect()
            }
            _ => vec![0; n],
        }
    }

    /// The host deselects the card: the command takes effect.
    pub(crate) fn deselect(&self) -> Done {
        let sent = std::mem::take(&mut *self.sent.borrow_mut());
        let Some(&cmd) = sent.first() else {
            return Done::Nothing;
        };
        match cmd {
            CMD_CLEAR_STATUS => self
                .status
                .set(self.status.get() & (STATUS_READY | STATUS_UNLOCKED)),
            CMD_SET_INTERRUPT if sent.len() >= 2 => self.interrupts.set(sent[1] & 1 != 0),
            CMD_SECTOR_ERASE if sent.len() >= 3 => {
                let start =
                    (usize::from(sent[1]) << 17 | usize::from(sent[2]) << 9) & !(SECTOR - 1);
                self.data.lock().unwrap()[start % SIZE..][..SECTOR].fill(0xFF);
                return self.busy();
            }
            CMD_CHIP_ERASE if sent.len() >= 3 => {
                self.data.lock().unwrap().fill(0xFF);
                return self.busy();
            }
            CMD_PAGE_PROGRAM if sent.len() >= 5 => {
                let start = address(&sent[1..5]) % SIZE;
                let page = &sent[5..];
                let n = page.len().min(PAGE).min(SIZE - start);
                self.data.lock().unwrap()[start..start + n].copy_from_slice(&page[..n]);
                return self.busy();
            }
            _ => {}
        }
        Done::Nothing
    }

    fn busy(&self) -> Done {
        self.status.set(self.status.get() | STATUS_BUSY);
        Done::Busy
    }

    /// An erase or a program finishes.
    pub(crate) fn finish(&self) {
        self.status.set(self.status.get() & !STATUS_BUSY);
        if let Some(saver) = &self.saver {
            saver.changed();
        }
    }
}

/// Keeps a card's file up to date from a thread of its own. A save programs hundreds of pages
/// within a frame, and writing the whole file after each one stopped the game, its sound and
/// its picture for a fifth of a second; now the card notes the change, and the thread writes
/// the file once the changes stop for a moment, through a temporary file so it's never left
/// half written. `flush_cards` writes what's pending at once, as a run ends.
struct Saver {
    data: Arc<Mutex<Vec<u8>>>,
    path: PathBuf,
    state: Mutex<SaveState>,
    cond: Condvar,
}

#[derive(Default)]
struct SaveState {
    /// Changes made, and how many the file has.
    changes: u64,
    written: u64,
    writing: bool,
}

static SAVERS: Mutex<Vec<Arc<Saver>>> = Mutex::new(Vec::new());

/// How long the card's contents stay unchanged before they're written, and the longest a
/// steady stream of changes waits.
const SETTLE: Duration = Duration::from_millis(100);
const SETTLE_MAX: Duration = Duration::from_secs(1);

impl Saver {
    fn start(data: Arc<Mutex<Vec<u8>>>, path: PathBuf) -> Arc<Self> {
        let saver = Arc::new(Self {
            data,
            path,
            state: Mutex::default(),
            cond: Condvar::new(),
        });
        let s = saver.clone();
        std::thread::Builder::new()
            .name("memory card".to_owned())
            .spawn(move || s.run())
            .expect("the memory card's thread");
        SAVERS.lock().unwrap().push(saver.clone());
        saver
    }

    fn changed(&self) {
        self.state.lock().unwrap().changes += 1;
        self.cond.notify_all();
    }

    fn run(&self) {
        loop {
            let mut st = self.state.lock().unwrap();
            while st.changes == st.written || st.writing {
                st = self.cond.wait(st).unwrap();
            }
            // Let a save's run of programs finish first.
            let first = Instant::now();
            loop {
                let seen = st.changes;
                st = self.cond.wait_timeout(st, SETTLE).unwrap().0;
                if st.changes == seen || first.elapsed() >= SETTLE_MAX {
                    break;
                }
            }
            if st.writing || st.changes == st.written {
                continue;
            }
            self.write(st);
        }
    }

    /// Writes the contents as they stand, marking the changes so far written.
    fn write(&self, mut st: std::sync::MutexGuard<'_, SaveState>) {
        st.writing = true;
        let changes = st.changes;
        drop(st);
        let data = self.data.lock().unwrap().clone();
        let tmp = self.path.with_extension("tmp");
        if let Err(e) = std::fs::write(&tmp, &data).and_then(|()| std::fs::rename(&tmp, &self.path)) {
            eprintln!("memory card {}: {e}", self.path.display());
        }
        let mut st = self.state.lock().unwrap();
        st.writing = false;
        st.written = st.written.max(changes);
        self.cond.notify_all();
    }

    /// Writes what isn't written yet, after any write under way.
    fn flush(&self) {
        let mut st = self.state.lock().unwrap();
        while st.writing {
            st = self.cond.wait(st).unwrap();
        }
        if st.changes != st.written {
            self.write(st);
        }
    }
}

/// Writes every memory card's pending changes to its file, as a run ends.
pub fn flush_cards() {
    for saver in SAVERS.lock().unwrap().iter() {
        saver.flush();
    }
}

/// The byte address a read or program command names: AD1, AD2, AD3 and BA.
fn address(b: &[u8]) -> usize {
    usize::from(b[0] & 0x7F) << 17
        | usize::from(b[1]) << 9
        | usize::from(b[2] & 0x03) << 7
        | usize::from(b[3] & 0x7F)
}

#[cfg(test)]
mod format_tests {
    use super::*;

    fn word(b: &[u8], at: usize) -> u16 {
        u16::from_be_bytes([b[at], b[at + 1]])
    }

    /// What the SDK's mount checks (CARDCheck.c's VerifyID, VerifyDir, VerifyFAT) accept.
    #[test]
    fn formatted_card_passes_the_sdks_checks() {
        let card = formatted();
        let id = &card[..0x200];
        assert_eq!(checksum(&id[..0x1FC]), (word(id, 0x1FC), word(id, 0x1FE)));
        assert_eq!((word(id, 0x20), word(id, 0x22), word(id, 0x24)), (0, 4, 0));
        // The serial, from slot A's flash ID and the time in serial[12..20].
        let flash = &crate::os::default_sram()[20..32];
        let mut rand = i64::from_be_bytes(id[12..20].try_into().unwrap());
        for i in 0..12 {
            rand = rand.wrapping_mul(1_103_515_245).wrapping_add(12345) >> 16;
            assert_eq!(id[i], flash[i].wrapping_add(rand as u8));
            rand = (rand.wrapping_mul(1_103_515_245).wrapping_add(12345) >> 16) & 0x7FFF;
        }
        for i in 0..2 {
            let dir = &card[(1 + i) * SECTOR..(2 + i) * SECTOR];
            assert_eq!(word(dir, 0x1FFA), i as u16);
            assert_eq!(checksum(&dir[..SECTOR - 4]), (word(dir, 0x1FFC), word(dir, 0x1FFE)));
            let fat = &card[(3 + i) * SECTOR..(4 + i) * SECTOR];
            assert_eq!((word(fat, 4), word(fat, 6), word(fat, 8)), (i as u16, 59, 4));
            assert_eq!(checksum(&fat[4..]), (word(fat, 0), word(fat, 2)));
        }
    }

    /// A formatted card gives back the flash ID it was formatted with; a blank one, none.
    #[test]
    fn flash_id_comes_back_from_the_serial() {
        let dir = std::env::temp_dir().join(format!("ssbm-card-id-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("card.raw");
        std::fs::write(&path, formatted()).unwrap();
        let card = Card::new(Some(path)).unwrap();
        let slot_a = &crate::os::default_sram()[20..32];
        assert_eq!(&card.flash_id().unwrap()[..], slot_a);
        assert_eq!(Card::new(None).unwrap().flash_id(), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod saver_tests {
    use super::*;

    /// Programs `page` at byte `at` as the CARD library does: the command, its address, data.
    fn program(card: &Card, at: usize, page: &[u8]) {
        card.select();
        let a = [
            (at >> 17) as u8 & 0x7F,
            (at >> 9) as u8,
            (at >> 7) as u8 & 0x03,
            at as u8 & 0x7F,
        ];
        card.write(&[CMD_PAGE_PROGRAM]);
        card.write(&a);
        card.write(page);
        assert!(matches!(card.deselect(), Done::Busy));
        card.finish();
    }

    /// A save's many programs reach the file together, after they stop, and at once on a flush.
    #[test]
    fn programs_reach_the_file_after_they_settle() {
        let dir = std::env::temp_dir().join(format!("ssbm-card-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("card.raw");
        std::fs::write(&path, formatted()).unwrap();
        let card = Card::new(Some(path.clone())).unwrap();
        for i in 0..400 {
            program(&card, 0xA000 + i * PAGE, &[i as u8; PAGE]);
        }
        // Not yet: the programs come faster than they settle.
        assert_eq!(std::fs::read(&path).unwrap(), formatted());
        std::thread::sleep(SETTLE * 4);
        let file = std::fs::read(&path).unwrap();
        assert_eq!(&file[0xA000..0xA000 + PAGE], &[0u8; PAGE]);
        assert_eq!(&file[0xA000 + 399 * PAGE..0xA000 + 400 * PAGE], &[143u8; PAGE]);
        // A flush writes what's pending without waiting.
        program(&card, 0x9000, &[7; PAGE]);
        flush_cards();
        assert_eq!(&std::fs::read(&path).unwrap()[0x9000..0x9000 + PAGE], &[7u8; PAGE]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
