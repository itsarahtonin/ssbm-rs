// SPDX-License-Identifier: GPL-3.0-or-later

//! A WAV file of the audio interface's output: 16-bit stereo at 32 kHz.

use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

pub struct Wav {
    file: File,
    frames: u32,
}

const RATE: u32 = 32_000;

impl Wav {
    pub fn create(path: &Path) -> std::io::Result<Self> {
        let mut w = Wav {
            file: File::create(path)?,
            frames: 0,
        };
        w.header()?;
        Ok(w)
    }

    fn header(&mut self) -> std::io::Result<()> {
        let data = self.frames * 4;
        let mut h = Vec::with_capacity(44);
        h.extend_from_slice(b"RIFF");
        h.extend_from_slice(&(36 + data).to_le_bytes());
        h.extend_from_slice(b"WAVEfmt ");
        h.extend_from_slice(&16u32.to_le_bytes());
        h.extend_from_slice(&1u16.to_le_bytes());
        h.extend_from_slice(&2u16.to_le_bytes());
        h.extend_from_slice(&RATE.to_le_bytes());
        h.extend_from_slice(&(RATE * 4).to_le_bytes());
        h.extend_from_slice(&4u16.to_le_bytes());
        h.extend_from_slice(&16u16.to_le_bytes());
        h.extend_from_slice(b"data");
        h.extend_from_slice(&data.to_le_bytes());
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&h)?;
        self.file.seek(SeekFrom::End(0))?;
        Ok(())
    }

    /// Appends a DMA block: big-endian pairs, right first as AX writes them. The header stays
    /// current, so the file is whole however the run ends.
    pub fn push(&mut self, block: &[u8]) -> std::io::Result<()> {
        let mut out = Vec::with_capacity(block.len());
        for p in block.chunks_exact(4) {
            out.extend_from_slice(&[p[3], p[2], p[1], p[0]]);
        }
        self.file.write_all(&out)?;
        self.frames += (block.len() / 4) as u32;
        self.header()
    }
}
