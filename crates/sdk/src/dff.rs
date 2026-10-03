// SPDX-License-Identifier: GPL-3.0-or-later
// The file format follows Dolphin's FifoDataFile (GPL-2.0-or-later).

//! Dolphin FIFO logs (.dff): a run's GPU command stream, frame by frame, with the memory the
//! GPU read, for Dolphin's FIFO player to draw with its software renderer as a reference for
//! ours. Memory goes in as updates at the point in a frame's commands where the GPU first reads
//! bytes the player doesn't hold yet: a shadow of the player's memory tells which.

use std::io::{Seek, SeekFrom, Write};

const FILE_ID: u32 = 0x0D01_F1F0;
const VERSION: u32 = 6;
const MIN_LOADER_VERSION: u32 = 1;
const HEADER_SIZE: u64 = 128;
const FRAME_INFO_SIZE: u64 = 64;
const UPDATE_SIZE: u64 = 24;
/// A memory update's kind, which the player only shows: texture data (1), XF data (2), vertex
/// arrays and display lists (4), palettes (8). Ours are all noted as vertex data.
pub const VERTEX_STREAM: u8 = 0x04;

const MEM1_SIZE: usize = 0x0180_0000;

struct Update {
    position: u32,
    address: u32,
    data: Vec<u8>,
    kind: u8,
}

struct Frame {
    fifo: Vec<u8>,
    updates: Vec<Update>,
    /// The game's FIFO ring in memory, which the player writes the commands through.
    fifo_start: u32,
    fifo_end: u32,
}

/// A FIFO log being recorded: the GPU's registers when it started, then frames.
pub struct Log {
    bp: Vec<u32>,
    cp: Vec<u32>,
    xf: Vec<u32>,
    tmem: Vec<u8>,
    frames: Vec<Frame>,
    current: Frame,
    /// Memory as the player will hold it: zero until an update sets it.
    shadow: Vec<u8>,
}

impl Log {
    /// Starts a log from the GPU's registers, XF memory (matrices then registers) and texture
    /// memory.
    pub fn new(bp: &[u32; 256], cp: &[u32; 256], xf: &[u32], tmem: &[u8]) -> Self {
        Log {
            bp: bp.to_vec(),
            cp: cp.to_vec(),
            xf: xf.to_vec(),
            tmem: tmem.to_vec(),
            frames: Vec::new(),
            current: Frame { fifo: Vec::new(), updates: Vec::new(), fifo_start: 0, fifo_end: 0 },
            shadow: vec![0; MEM1_SIZE],
        }
    }

    /// Bytes of commands fed to the GPU.
    pub fn fifo(&mut self, data: &[u8]) {
        self.current.fifo.extend_from_slice(data);
    }

    /// The GPU read `data` at physical `address`: what of it the player doesn't hold goes in
    /// as an update, at the start of the bytes fed last.
    pub fn read(&mut self, address: u32, data: &[u8], kind: u8, position: u32) {
        let start = address as usize;
        let end = (start + data.len()).min(MEM1_SIZE);
        if start >= end || self.shadow[start..end] == data[..end - start] {
            return;
        }
        self.shadow[start..end].copy_from_slice(&data[..end - start]);
        self.current.updates.push(Update { position, address, data: data[..end - start].to_vec(), kind });
    }

    /// Where the bytes fed last start in this frame's commands.
    pub fn position(&self) -> u32 {
        self.current.fifo.len() as u32
    }

    /// Ends a frame, whose commands went through the FIFO ring `fifo_start..fifo_end`.
    pub fn end_frame(&mut self, fifo_start: u32, fifo_end: u32) {
        let empty = Frame { fifo: Vec::new(), updates: Vec::new(), fifo_start: 0, fifo_end: 0 };
        let mut frame = std::mem::replace(&mut self.current, empty);
        frame.fifo_start = fifo_start;
        frame.fifo_end = fifo_end;
        self.frames.push(frame);
    }

    pub fn frames(&self) -> usize {
        self.frames.len()
    }

    /// Writes the log as Dolphin's FIFO player reads it.
    pub fn save(&self, out: &mut (impl Write + Seek)) -> std::io::Result<()> {
        let le32 = |v: u32| v.to_le_bytes();
        let le64 = |v: u64| v.to_le_bytes();
        out.write_all(&[0; HEADER_SIZE as usize])?;
        let frame_list = HEADER_SIZE;
        out.write_all(&vec![0; self.frames.len() * FRAME_INFO_SIZE as usize])?;
        let mut at = frame_list + self.frames.len() as u64 * FRAME_INFO_SIZE;
        let block = |out: &mut dyn Write, bytes: Vec<u8>, at: &mut u64| -> std::io::Result<u64> {
            let start = *at;
            out.write_all(&bytes)?;
            *at += bytes.len() as u64;
            Ok(start)
        };
        let words = |v: &[u32]| v.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>();
        let bp_at = block(out, words(&self.bp), &mut at)?;
        let cp_at = block(out, words(&self.cp), &mut at)?;
        let xf_mem_at = block(out, words(&self.xf[..0x1000]), &mut at)?;
        let mut regs = self.xf[0x1000..].to_vec();
        regs.resize(88, 0);
        let xf_regs_at = block(out, words(&regs), &mut at)?;
        let tmem_at = block(out, self.tmem.clone(), &mut at)?;

        let mut infos = Vec::new();
        for frame in &self.frames {
            let fifo_at = block(out, frame.fifo.clone(), &mut at)?;
            // The updates' data, then their table.
            let mut data_at = Vec::new();
            for u in &frame.updates {
                data_at.push(block(out, u.data.clone(), &mut at)?);
            }
            let table_at = at;
            for (u, &d) in frame.updates.iter().zip(&data_at) {
                let mut e = Vec::with_capacity(UPDATE_SIZE as usize);
                e.extend(le32(u.position));
                e.extend(le32(u.address));
                e.extend(le64(d));
                e.extend(le32(u.data.len() as u32));
                e.extend([u.kind, 0, 0, 0]);
                block(out, e, &mut at)?;
            }
            infos.push((fifo_at, frame.fifo.len() as u32, frame, table_at, frame.updates.len() as u32));
        }

        let mut header = Vec::with_capacity(HEADER_SIZE as usize);
        header.extend(le32(FILE_ID));
        header.extend(le32(VERSION));
        header.extend(le32(MIN_LOADER_VERSION));
        header.extend(le64(bp_at));
        header.extend(le32(256));
        header.extend(le64(cp_at));
        header.extend(le32(256));
        header.extend(le64(xf_mem_at));
        header.extend(le32(0x1000));
        header.extend(le64(xf_regs_at));
        header.extend(le32(88));
        header.extend(le64(frame_list));
        header.extend(le32(self.frames.len() as u32));
        header.extend(le32(0)); // flags: not Wii
        header.extend(le64(tmem_at));
        header.extend(le32(self.tmem.len() as u32));
        header.extend(le32(MEM1_SIZE as u32));
        header.extend(le32(0));
        header.extend(*b"GALE01\0\0");
        header.resize(HEADER_SIZE as usize, 0);
        out.seek(SeekFrom::Start(0))?;
        out.write_all(&header)?;
        for (i, (fifo_at, fifo_len, frame, table_at, updates)) in infos.into_iter().enumerate() {
            let mut info = Vec::with_capacity(FRAME_INFO_SIZE as usize);
            info.extend(le64(fifo_at));
            info.extend(le32(fifo_len));
            info.extend(le32(frame.fifo_start));
            info.extend(le32(frame.fifo_end));
            info.extend(le64(table_at));
            info.extend(le32(updates));
            info.resize(FRAME_INFO_SIZE as usize, 0);
            out.seek(SeekFrom::Start(frame_list + i as u64 * FRAME_INFO_SIZE))?;
            out.write_all(&info)?;
        }
        Ok(())
    }
}
