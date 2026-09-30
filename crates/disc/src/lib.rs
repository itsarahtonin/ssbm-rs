// SPDX-License-Identifier: GPL-3.0-or-later

//! Loads and verifies a Super Smash Bros. Melee (NTSC 1.02) disc image.
//!
//! Reads ISO, RVZ, CISO, GCZ and the other formats nod supports. The constants identify the
//! retail disc; no game data is embedded here.

use std::io::{self, Read};
use std::path::Path;

use nod::common::PartitionKind;
use nod::read::{DiscOptions, DiscReader, PartitionMeta, PartitionOptions};
use sha1::{Digest, Sha1};
use ssbm_mem::Mem;

/// Game ID of the supported disc.
pub const GAME_ID: &str = "GALE01";
/// Disc revision of NTSC 1.02.
pub const REVISION: u8 = 2;
/// Redump SHA-1 of the full disc image.
pub const DISC_SHA1: &str = "d4e70c064cc714ba8400a849cf299dbd1aa326fc";
/// SHA-1 of `main.dol`, which the decomp rebuilds byte for byte.
pub const DOL_SHA1: &str = "08e0bf20134dfcb260699671004527b2d6bb1a45";

#[derive(Debug, thiserror::Error)]
pub enum DiscError {
    #[error(transparent)]
    Nod(#[from] nod::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Mem(#[from] ssbm_mem::MemError),
    #[error("not Melee NTSC 1.02: found {game_id} revision {revision}")]
    WrongGame { game_id: String, revision: u8 },
    #[error("main.dol has SHA-1 {actual}, expected {DOL_SHA1}")]
    WrongDol { actual: String },
    #[error("invalid DOL: {0}")]
    InvalidDol(&'static str),
    #[error("invalid file system table: {0}")]
    InvalidFst(&'static str),
    #[error("no file at {0} on the disc")]
    FileNotFound(String),
}

pub type Result<T> = std::result::Result<T, DiscError>;

/// An opened Melee NTSC 1.02 disc.
pub struct Disc {
    reader: DiscReader,
}

impl Disc {
    /// Opens a disc image and checks its game ID and revision. The full-image hash is
    /// separate, see [`Disc::disc_sha1`].
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let reader = DiscReader::new(path, &DiscOptions::default())?;
        let header = reader.header();
        if header.game_id_str() != GAME_ID || header.disc_version != REVISION {
            return Err(DiscError::WrongGame {
                game_id: header.game_id_str().to_owned(),
                revision: header.disc_version,
            });
        }
        Ok(Self { reader })
    }

    /// The image's container format, such as ISO or RVZ.
    pub fn format(&self) -> String {
        self.reader.meta().format.to_string()
    }

    fn partition_meta(&self) -> Result<PartitionMeta> {
        let mut partition = self
            .reader
            .open_partition_kind(PartitionKind::Data, &PartitionOptions::default())?;
        Ok(partition.meta()?)
    }

    /// Reads `main.dol` and checks that it is the exact executable the decomp targets.
    pub fn main_dol(&self) -> Result<Dol> {
        let raw = self.partition_meta()?.raw_dol.to_vec();
        let actual = hex(&Sha1::digest(&raw));
        if actual != DOL_SHA1 {
            return Err(DiscError::WrongDol { actual });
        }
        Dol::parse(raw)
    }

    /// Reads a file from the disc's file system, such as `PlFxNr.dat`.
    pub fn read_file(&self, path: &str) -> Result<Vec<u8>> {
        let mut partition = self
            .reader
            .open_partition_kind(PartitionKind::Data, &PartitionOptions::default())?;
        let meta = partition.meta()?;
        let fst = meta.fst().map_err(DiscError::InvalidFst)?;
        let (_, node) = fst
            .find(path)
            .filter(|(_, node)| node.is_file())
            .ok_or_else(|| DiscError::FileNotFound(path.to_owned()))?;
        let mut data = Vec::with_capacity(node.length() as usize);
        partition.open_file(node)?.read_to_end(&mut data)?;
        Ok(data)
    }

    /// SHA-1 of the full image as lowercase hex, reporting `(done, total)` bytes as it reads.
    pub fn disc_sha1(&mut self, mut progress: impl FnMut(u64, u64)) -> Result<String> {
        let total = self.reader.disc_size();
        let mut hasher = Sha1::new();
        let mut buf = vec![0; 1 << 20];
        let mut done = 0;
        loop {
            let n = self.reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            done += n as u64;
            progress(done, total);
        }
        Ok(hex(&hasher.finalize()))
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionKind {
    Text,
    Data,
}

/// One loadable DOL section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Section {
    pub kind: SectionKind,
    pub offset: u32,
    pub addr: u32,
    pub size: u32,
}

/// A parsed DOL executable.
#[derive(Debug, Clone)]
pub struct Dol {
    raw: Vec<u8>,
    pub sections: Vec<Section>,
    pub bss_addr: u32,
    pub bss_size: u32,
    pub entry: u32,
}

impl Dol {
    pub fn parse(raw: Vec<u8>) -> Result<Self> {
        if raw.len() < 0x100 {
            return Err(DiscError::InvalidDol("header is truncated"));
        }
        let word = |at: usize| u32::from_be_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]]);

        // Offsets, addresses and sizes are three tables of 7 text then 11 data entries.
        let mut sections = Vec::new();
        for i in 0..18 {
            let section = Section {
                kind: if i < 7 {
                    SectionKind::Text
                } else {
                    SectionKind::Data
                },
                offset: word(4 * i),
                addr: word(0x48 + 4 * i),
                size: word(0x90 + 4 * i),
            };
            if section.size == 0 {
                continue;
            }
            let end = u64::from(section.offset) + u64::from(section.size);
            if end > raw.len() as u64 {
                return Err(DiscError::InvalidDol(
                    "a section extends past the end of the file",
                ));
            }
            sections.push(section);
        }

        Ok(Self {
            bss_addr: word(0xD8),
            bss_size: word(0xDC),
            entry: word(0xE0),
            sections,
            raw,
        })
    }

    /// The whole executable as read from the disc.
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    pub fn section_data(&self, section: &Section) -> &[u8] {
        &self.raw[section.offset as usize..(section.offset + section.size) as usize]
    }

    /// Copies every section to its load address. BSS stays as is, since memory starts zeroed.
    pub fn load_into(&self, mem: &Mem) -> Result<()> {
        for section in &self.sections {
            mem.write_bytes(section.addr, self.section_data(section))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_dol() -> Vec<u8> {
        let mut raw = vec![0; 0x110];
        let mut put = |at: usize, value: u32| raw[at..at + 4].copy_from_slice(&value.to_be_bytes());
        put(0x00, 0x100); // text 0: offset, address, size
        put(0x48, 0x8000_3100);
        put(0x90, 8);
        put(0x1C, 0x108); // data 0
        put(0x64, 0x8040_0000);
        put(0xAC, 8);
        put(0xD8, 0x8050_0000);
        put(0xDC, 0x100);
        put(0xE0, 0x8000_3100);
        raw[0x100..0x110].copy_from_slice(b"TEXTTEXTDATADATA");
        raw
    }

    #[test]
    fn parses_and_loads_sections() {
        let dol = Dol::parse(synthetic_dol()).unwrap();
        assert_eq!(dol.sections.len(), 2);
        assert_eq!(
            dol.sections[0],
            Section {
                kind: SectionKind::Text,
                offset: 0x100,
                addr: 0x8000_3100,
                size: 8
            }
        );
        assert_eq!(dol.sections[1].kind, SectionKind::Data);
        assert_eq!(
            (dol.bss_addr, dol.bss_size, dol.entry),
            (0x8050_0000, 0x100, 0x8000_3100)
        );

        let mem = Mem::new();
        dol.load_into(&mem).unwrap();
        assert_eq!(
            mem.read_u32(0x8000_3100).unwrap(),
            u32::from_be_bytes(*b"TEXT")
        );
        assert_eq!(
            mem.read_u32(0x8040_0004).unwrap(),
            u32::from_be_bytes(*b"DATA")
        );
    }

    #[test]
    fn rejects_malformed_dols() {
        assert!(matches!(
            Dol::parse(vec![0; 0x20]),
            Err(DiscError::InvalidDol(_))
        ));
        let mut raw = synthetic_dol();
        raw[0x90..0x94].copy_from_slice(&0x1000_u32.to_be_bytes());
        assert!(matches!(Dol::parse(raw), Err(DiscError::InvalidDol(_))));
    }
}
