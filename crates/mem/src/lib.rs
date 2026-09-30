// SPDX-License-Identifier: GPL-3.0-or-later

//! GameCube main memory (MEM1): 24 MB of big-endian RAM at its original 32-bit addresses.
//!
//! Writes take `&self` so typed handles can share one `Mem`. Floats go through `gekko-fp`'s
//! `lfs`/`stfs`, so loads and stores match the CPU bit for bit.

use std::cell::Cell;
use std::fmt;

/// Size of main memory.
pub const MEM1_SIZE: u32 = 0x0180_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MemError {
    #[error("unmapped access of {len} bytes at {addr:#010X}")]
    Unmapped { addr: u32, len: u32 },
}

pub type Result<T> = std::result::Result<T, MemError>;

/// Main memory, mapped at `0x8000_0000` (cached) and mirrored at `0xC000_0000` (uncached).
pub struct Mem {
    mem1: Box<[Cell<u8>]>,
}

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
        pub fn $read(&self, addr: u32) -> Result<$t> {
            Ok(<$t>::from_be_bytes(self.read_array(addr)?))
        }

        pub fn $write(&self, addr: u32, value: $t) -> Result<()> {
            self.write_bytes(addr, &value.to_be_bytes())
        }
    };
}

impl Mem {
    /// Zeroed memory, as the console starts.
    pub fn new() -> Self {
        Self {
            mem1: vec![Cell::new(0); MEM1_SIZE as usize].into_boxed_slice(),
        }
    }

    fn cells(&self, addr: u32, len: u32) -> Result<&[Cell<u8>]> {
        let phys = addr & 0x3FFF_FFFF;
        let in_range = phys.checked_add(len).is_some_and(|end| end <= MEM1_SIZE);
        if matches!(addr >> 30, 0b10 | 0b11) && in_range {
            Ok(&self.mem1[phys as usize..(phys + len) as usize])
        } else {
            Err(MemError::Unmapped { addr, len })
        }
    }

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

    pub fn write_bytes(&self, addr: u32, data: &[u8]) -> Result<()> {
        let cells = self.cells(addr, data.len() as u32)?;
        for (cell, byte) in cells.iter().zip(data) {
            cell.set(*byte);
        }
        Ok(())
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
        assert!(mem.write_u8(0xE000_0000, 1).is_err());
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
}
