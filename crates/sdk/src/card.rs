// SPDX-License-Identifier: GPL-3.0-or-later
// Behavior follows Dolphin's EXI memory card device (GPL-2.0-or-later).

//! A memory card in slot A, answering at the EXI level so the SDK's CARD library runs on it
//! as on a console: a 59-block flash card that takes the library's commands, raises its
//! interrupt when an erase or a program is done, and keeps its contents in a file.
//!
//! Like Dolphin's, the card reports itself unlocked from the start, so the library never runs
//! the DSP's unlock exchange; the SRAM's flash ID for slot A is Dolphin's, which the card's
//! format then carries.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;

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
    // __CARDCheckSum: sums of the big-endian words and of their complements, 0xFFFF as 0.
    let checksum = |bytes: &[u8]| {
        let (mut sum, mut inv) = (0u16, 0u16);
        for w in bytes.chunks_exact(2) {
            let v = u16::from_be_bytes([w[0], w[1]]);
            sum = sum.wrapping_add(v);
            inv = inv.wrapping_add(!v);
        }
        let fix = |v: u16| if v == 0xFFFF { 0 } else { v };
        (fix(sum), fix(inv))
    };
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
    data: RefCell<Vec<u8>>,
    /// Where the contents live between runs, if anywhere.
    path: Option<PathBuf>,
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
        Ok(Self {
            data: RefCell::new(data),
            path,
            status: Cell::new(STATUS_READY | STATUS_UNLOCKED),
            interrupts: Cell::new(false),
            sent: RefCell::default(),
            answered: Cell::new(0),
        })
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
                let data = self.data.borrow();
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
                self.data.borrow_mut()[start % SIZE..][..SECTOR].fill(0xFF);
                return self.busy();
            }
            CMD_CHIP_ERASE if sent.len() >= 3 => {
                self.data.borrow_mut().fill(0xFF);
                return self.busy();
            }
            CMD_PAGE_PROGRAM if sent.len() >= 5 => {
                let start = address(&sent[1..5]) % SIZE;
                let page = &sent[5..];
                let n = page.len().min(PAGE).min(SIZE - start);
                self.data.borrow_mut()[start..start + n].copy_from_slice(&page[..n]);
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
        if let Some(path) = &self.path {
            let data = self.data.borrow();
            std::fs::write(path, &*data)
                .unwrap_or_else(|e| panic!("memory card {}: {e}", path.display()));
        }
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

    /// __CARDCheckSum over `bytes`.
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
}
