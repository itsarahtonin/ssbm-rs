// SPDX-License-Identifier: GPL-3.0-or-later

//! GameCube main memory (MEM1): 24 MB of big-endian RAM at its original 32-bit addresses, and
//! the 16 KB of cache the CPU can lock as scratch memory at `0xE000_0000`.
//!
//! Writes take `&self` so typed handles can share one `Mem`. Floats go through `gekko-fp`'s
//! `lfs`/`stfs`, so loads and stores match the CPU bit for bit. A page journal records the
//! original contents of written pages, which is how lockstep checks snapshot and roll back, and
//! a write log records the writes themselves, so they can be replayed.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::fmt;

/// Size of main memory.
pub const MEM1_SIZE: u32 = 0x0180_0000;
/// Journal granularity.
pub const PAGE_SIZE: u32 = 0x1000;
/// Where the locked cache is mapped, and its size.
pub const LOCKED_CACHE: u32 = 0xE000_0000;
pub const LOCKED_CACHE_SIZE: u32 = 0x4000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MemError {
    #[error("unmapped access of {len} bytes at {addr:#010X}")]
    Unmapped { addr: u32, len: u32 },
}

pub type Result<T> = std::result::Result<T, MemError>;

/// Page contents keyed by page index, used both for journals and snapshots.
pub type Pages = BTreeMap<u32, Box<[u8]>>;

/// Main memory, mapped at `0x8000_0000` (cached) and mirrored at `0xC000_0000` (uncached), and
/// the locked cache, which journals and logs cover alike: it follows MEM1 in `mem1`, so its
/// pages come after MEM1's.
pub struct Mem {
    mem1: Box<[Cell<u8>]>,
    /// `JOURNAL` and `LOG` bits: what writes must also record.
    recording: Cell<u8>,
    /// Nested journals, innermost last; each holds pages first written at its level.
    journal: RefCell<Vec<Pages>>,
    log: RefCell<Vec<(u32, Vec<u8>)>>,
}

const JOURNAL: u8 = 1;
const LOG: u8 = 2;

impl Default for Mem {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Mem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Mem")
            .field("size", &self.mem1.len())
            .finish()
    }
}

macro_rules! access {
    ($read:ident, $write:ident, $t:ty) => {
        #[inline]
        pub fn $read(&self, addr: u32) -> Result<$t> {
            Ok(<$t>::from_be_bytes(self.read_array(addr)?))
        }

        #[inline]
        pub fn $write(&self, addr: u32, value: $t) -> Result<()> {
            self.write_bytes(addr, &value.to_be_bytes())
        }
    };
}

/// Physical offset of `addr` in MEM1, if `len` bytes there are mapped.
#[inline]
pub fn phys(addr: u32, len: u32) -> Option<u32> {
    let phys = addr & 0x3FFF_FFFF;
    let in_range = phys.checked_add(len).is_some_and(|end| end <= MEM1_SIZE);
    (matches!(addr >> 30, 0b10 | 0b11) && in_range).then_some(phys)
}

/// Offset of `addr` in `Mem`'s storage, if `len` bytes there are mapped: MEM1's physical
/// address, or past MEM1 for the locked cache.
#[inline]
fn offset(addr: u32, len: u32) -> Option<u32> {
    phys(addr, len).or_else(|| {
        let off = addr.wrapping_sub(LOCKED_CACHE);
        (off < LOCKED_CACHE_SIZE && off + len <= LOCKED_CACHE_SIZE).then_some(MEM1_SIZE + off)
    })
}

/// The address of a page of `Mem`'s storage, as a journal or snapshot keys it.
pub fn page_addr(page: u32) -> u32 {
    let at = page * PAGE_SIZE;
    if at < MEM1_SIZE {
        0x8000_0000 | at
    } else {
        LOCKED_CACHE + (at - MEM1_SIZE)
    }
}

impl Mem {
    /// Zeroed memory, as the console starts.
    pub fn new() -> Self {
        Self {
            mem1: vec![Cell::new(0); (MEM1_SIZE + LOCKED_CACHE_SIZE) as usize].into_boxed_slice(),
            recording: Cell::new(0),
            journal: RefCell::default(),
            log: RefCell::default(),
        }
    }

    #[inline]
    fn cells(&self, addr: u32, len: u32) -> Result<&[Cell<u8>]> {
        match offset(addr, len) {
            Some(p) => Ok(&self.mem1[p as usize..(p + len) as usize]),
            None => Err(MemError::Unmapped { addr, len }),
        }
    }

    #[inline]
    fn read_array<const N: usize>(&self, addr: u32) -> Result<[u8; N]> {
        let cells = self.cells(addr, N as u32)?;
        Ok(std::array::from_fn(|i| cells[i].get()))
    }

    pub fn read_bytes(&self, addr: u32, out: &mut [u8]) -> Result<()> {
        let cells = self.cells(addr, out.len() as u32)?;
        for (byte, cell) in out.iter_mut().zip(cells) {
            *byte = cell.get();
        }
        Ok(())
    }

    #[inline]
    pub fn write_bytes(&self, addr: u32, data: &[u8]) -> Result<()> {
        let cells = self.cells(addr, data.len() as u32)?;
        if self.recording.get() != 0 {
            self.note_write(addr, data);
        }
        for (cell, byte) in cells.iter().zip(data) {
            cell.set(*byte);
        }
        Ok(())
    }

    #[cold]
    fn note_write(&self, addr: u32, data: &[u8]) {
        let recording = self.recording.get();
        if recording & JOURNAL != 0 {
            let len = data.len() as u32;
            let phys = offset(addr, len).expect("a write that was mapped");
            let mut journals = self.journal.borrow_mut();
            let journal = journals.last_mut().expect("journaling without a journal");
            for page in phys / PAGE_SIZE..=(phys + len.max(1) - 1) / PAGE_SIZE {
                journal
                    .entry(page)
                    .or_insert_with(|| self.page_contents(page));
            }
        }
        if recording & LOG != 0 {
            self.log.borrow_mut().push((addr, data.to_vec()));
        }
    }

    fn page_contents(&self, page: u32) -> Box<[u8]> {
        let start = (page * PAGE_SIZE) as usize;
        self.mem1[start..start + PAGE_SIZE as usize]
            .iter()
            .map(Cell::get)
            .collect()
    }

    access!(read_u8, write_u8, u8);
    access!(read_u16, write_u16, u16);
    access!(read_u32, write_u32, u32);
    access!(read_u64, write_u64, u64);

    /// `lfs`: a single from memory as a register value.
    pub fn read_f32(&self, addr: u32) -> Result<f64> {
        Ok(gekko_fp::lfs(self.read_u32(addr)?))
    }

    /// `stfs`: stores a register value as a single (truncating, like the CPU).
    pub fn write_f32(&self, addr: u32, value: f64) -> Result<()> {
        self.write_u32(addr, gekko_fp::stfs(value))
    }

    /// `lfd`
    pub fn read_f64(&self, addr: u32) -> Result<f64> {
        Ok(f64::from_bits(self.read_u64(addr)?))
    }

    /// `stfd`
    pub fn write_f64(&self, addr: u32, value: f64) -> Result<()> {
        self.write_u64(addr, value.to_bits())
    }

    /// Starts recording the original contents of every page written from now on. Journals
    /// nest: an inner one records what is written while it is open.
    pub fn begin_journal(&self) {
        self.journal.borrow_mut().push(Pages::new());
        self.recording.set(self.recording.get() | JOURNAL);
    }

    /// Stops the innermost journal and returns the original contents of the pages written
    /// while it was open. The enclosing journal takes over the pages it had not yet seen, so
    /// it can still roll back to where it began.
    pub fn end_journal(&self) -> Pages {
        let mut journals = self.journal.borrow_mut();
        let done = journals.pop().expect("no journal to end");
        match journals.last_mut() {
            Some(outer) => {
                for (page, data) in &done {
                    outer.entry(*page).or_insert_with(|| data.clone());
                }
            }
            None => self.recording.set(self.recording.get() & !JOURNAL),
        }
        done
    }

    pub fn is_journaling(&self) -> bool {
        self.recording.get() & JOURNAL != 0
    }

    /// Starts logging every write, in order.
    pub fn begin_log(&self) {
        assert!(self.recording.get() & LOG == 0, "write logs do not nest");
        self.log.borrow_mut().clear();
        self.recording.set(self.recording.get() | LOG);
    }

    /// Stops logging and returns the writes, as (address, bytes).
    pub fn end_log(&self) -> Vec<(u32, Vec<u8>)> {
        self.recording.set(self.recording.get() & !LOG);
        std::mem::take(&mut *self.log.borrow_mut())
    }

    /// The current contents of the given pages.
    pub fn capture<'a>(&self, pages: impl IntoIterator<Item = &'a u32>) -> Pages {
        pages
            .into_iter()
            .map(|&p| (p, self.page_contents(p)))
            .collect()
    }

    /// Writes page contents back, bypassing the journal.
    pub fn restore(&self, pages: &Pages) {
        for (&page, data) in pages {
            let start = (page * PAGE_SIZE) as usize;
            for (cell, byte) in self.mem1[start..].iter().zip(data.iter()) {
                cell.set(*byte);
            }
        }
    }

    /// A copy of all of MEM1.
    pub fn to_vec(&self) -> Vec<u8> {
        self.mem1[..MEM1_SIZE as usize]
            .iter()
            .map(Cell::get)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_big_endian() {
        let mem = Mem::new();
        mem.write_u32(0x8000_1000, 0x1234_5678).unwrap();
        assert_eq!(mem.read_u8(0x8000_1000).unwrap(), 0x12);
        assert_eq!(mem.read_u16(0x8000_1002).unwrap(), 0x5678);
        assert_eq!(mem.read_u32(0x8000_1000).unwrap(), 0x1234_5678);
    }

    #[test]
    fn uncached_addresses_mirror_cached_ones() {
        let mem = Mem::new();
        mem.write_u64(0xC000_2000, 0x0102_0304_0506_0708).unwrap();
        assert_eq!(mem.read_u64(0x8000_2000).unwrap(), 0x0102_0304_0506_0708);
    }

    #[test]
    fn accesses_outside_mem1_fail() {
        let mem = Mem::new();
        assert_eq!(mem.read_u32(0x817F_FFFC), Ok(0));
        assert_eq!(
            mem.read_u32(0x817F_FFFE),
            Err(MemError::Unmapped {
                addr: 0x817F_FFFE,
                len: 4
            })
        );
        assert!(mem.read_u8(0x0000_0000).is_err());
        assert!(mem.write_u8(0xE000_4000, 1).is_err());
    }

    #[test]
    fn journals_cover_the_locked_cache() {
        let mem = Mem::new();
        mem.write_u32(0xE000_3FFC, 5).unwrap();
        mem.begin_journal();
        mem.write_u32(0xE000_3FFC, 6).unwrap();
        let original = mem.end_journal();
        let page = *original.keys().next().unwrap();
        assert_eq!(page_addr(page), 0xE000_3000);
        mem.restore(&original);
        assert_eq!(mem.read_u32(0xE000_3FFC), Ok(5));
    }

    #[test]
    fn floats_use_cpu_conversions() {
        let mem = Mem::new();
        mem.write_f32(0x8000_0100, 1.0 + 2f64.powi(-23) - 2f64.powi(-40))
            .unwrap();
        assert_eq!(
            mem.read_u32(0x8000_0100).unwrap(),
            0x3F80_0000,
            "stfs truncates"
        );
        mem.write_u32(0x8000_0104, 0x7F80_0001).unwrap();
        assert_eq!(
            mem.read_f32(0x8000_0104).unwrap().to_bits(),
            0x7FF0_0000_2000_0000
        );
    }

    #[test]
    fn journal_rolls_back_writes() {
        let mem = Mem::new();
        mem.write_u32(0x8000_1FFE, 0xAAAA_AAAA).unwrap();
        mem.begin_journal();
        mem.write_u32(0x8000_1FFE, 0x1234_5678).unwrap(); // spans two pages
        mem.write_u8(0x8010_0000, 7).unwrap();
        let original = mem.end_journal();
        assert_eq!(original.keys().copied().collect::<Vec<_>>(), [1, 2, 0x100]);
        let changed = mem.capture(original.keys());
        mem.restore(&original);
        assert_eq!(mem.read_u32(0x8000_1FFE).unwrap(), 0xAAAA_AAAA);
        assert_eq!(mem.read_u8(0x8010_0000).unwrap(), 0);
        mem.restore(&changed);
        assert_eq!(mem.read_u32(0x8000_1FFE).unwrap(), 0x1234_5678);
    }

    #[test]
    fn nested_journals_roll_back_to_their_own_start() {
        let mem = Mem::new();
        mem.write_u32(0x8000_0000, 1).unwrap();
        mem.begin_journal();
        mem.write_u32(0x8000_0000, 2).unwrap();
        mem.begin_journal();
        mem.write_u32(0x8000_0000, 3).unwrap();
        mem.write_u32(0x8000_5000, 9).unwrap();
        let inner = mem.end_journal();
        mem.restore(&inner);
        assert_eq!(mem.read_u32(0x8000_0000).unwrap(), 2);
        assert_eq!(mem.read_u32(0x8000_5000).unwrap(), 0);
        mem.write_u32(0x8000_5000, 8).unwrap();
        let outer = mem.end_journal();
        assert!(!mem.is_journaling());
        mem.restore(&outer);
        assert_eq!(mem.read_u32(0x8000_0000).unwrap(), 1);
        assert_eq!(mem.read_u32(0x8000_5000).unwrap(), 0);
    }
}
