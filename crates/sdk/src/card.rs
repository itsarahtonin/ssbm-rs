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
