// SPDX-License-Identifier: GPL-3.0-or-later

//! The renderer on threads of its own. The game thread keeps the GX state and sends what each
//! draw and copy needs: the state's changes since the last, the vertices, and the memory the
//! textures read. A thread with a copy of the state kept by those changes decodes and transforms
//! each draw's vertices, and a renderer on another thread, with a copy of its own, draws them,
//! so the game runs on while it draws, at most `AHEAD` frames behind.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, mpsc};

use ssbm_gx::{Delta, Draw, Memory, Sink, State, vertex};

use ssbm_gx::xform::Output;

use crate::{Renderer, bits, copy, hash_bytes, prepare, sampled_image, texture_maps};

/// How many frames the game may run ahead of the renderer.
const AHEAD: u64 = 2;

/// Draws and copies sent before the render thread takes them, in order.
const BATCH: usize = 1024;

enum Packet {
    Changes(Delta),
    /// Memory a texture reads, by its address.
    Memory(u32, Vec<u8>),
    Draw {
        primitive: u8,
        vat: u8,
        count: u16,
        data: Vec<u8>,
    },
    /// A draw with its vertices transformed (`prepare`), unless its scissor leaves nothing.
    Prepared {
        primitive: u8,
        vat: u8,
        count: u16,
        data: Vec<u8>,
        outs: Option<Vec<Output>>,
    },
    Copy(u32),
    /// Memory copies went to that the game has written over since (copy::Footprints).
    Forget(u32, u32),
}

/// Frames the render thread has finished, and what its copies to textures wrote to memory
/// that the game thread hasn't taken yet, by which copy it was (counting from 0).
#[derive(Default)]
struct Done {
    frames: Mutex<u64>,
    cond: Condvar,
    written: Mutex<Vec<(u64, Vec<(u32, Vec<u8>)>)>>,
}

/// A `Renderer` on its own thread, as a sink for the game thread's GX stream.
pub struct Threaded {
    tx: Option<mpsc::Sender<Vec<Packet>>>,
    threads: Vec<std::thread::JoinHandle<()>>,
    batch: Vec<Packet>,
    /// Hashes of the memory textures read this frame, so each range hashes once a frame.
    hashes: HashMap<(u32, u32), u64>,
    /// What the render thread holds of each range, by hash.
    held: HashMap<(u32, u32), u64>,
    done: Arc<Done>,
    frames: u64,
    /// Where copies went: this thread reads the game's memory, the renderer can't.
    footprints: copy::Footprints,
    /// Copies to textures sent.
    copies: u64,
    /// The memory under each copy the renderer encodes into memory, as the copy found it, by
    /// which copy it is: its first byte's physical address and the bytes to its last.
    before: VecDeque<(u64, u32, Vec<u8>)>,
}

impl Threaded {
    pub fn spawn(mut renderer: Renderer) -> Self {
        renderer.track_copies = false;
        let (tx, rx) = mpsc::channel();
        let (prepared_tx, prepared_rx) = mpsc::channel();
        let done = Arc::new(Done::default());
        let finished = done.clone();
        let threads = vec![
            std::thread::Builder::new()
                .name("transform".to_owned())
                .spawn(move || prepare_all(&rx, &prepared_tx))
                .expect("the transform thread"),
            std::thread::Builder::new()
                .name("render".to_owned())
                .spawn(move || run(renderer, &prepared_rx, &finished))
                .expect("the render thread"),
        ];
        Self {
            tx: Some(tx),
            threads,
            batch: Vec::new(),
            hashes: HashMap::new(),
            held: HashMap::new(),
            done,
            frames: 0,
            footprints: copy::Footprints::default(),
            copies: 0,
            before: VecDeque::new(),
        }
    }

    fn send(&mut self) {
        if !self.batch.is_empty() {
            let batch = std::mem::take(&mut self.batch);
            let tx = self.tx.as_ref().expect("the render thread finished");
            tx.send(batch).expect("the render thread stopped");
        }
    }

    fn changes(&mut self, state: &State) {
        let delta = state.take_delta();
        if !delta.is_empty() {
            self.batch.push(Packet::Changes(delta));
        }
    }
}

impl Sink for Threaded {
    fn tracks_changes(&self) -> bool {
        true
    }

    fn finish(&mut self) {
        self.send();
        self.tx = None;
        for thread in self.threads.drain(..) {
            thread.join().expect("a render thread");
        }
    }

    fn draw(&mut self, state: &State, draw: &Draw<'_>, mem: &dyn Memory) {
        self.changes(state);
        for map in texture_maps(state) {
            let image = sampled_image(state, map);
            let range = (image.address, image.size());
            // A copy there that the game has written over since is gone, even within the frame.
            if let Some((start, end)) = self.footprints.take_if_overwritten(image.address, mem) {
                self.batch.push(Packet::Forget(start, end));
            }
            if self.hashes.contains_key(&range) {
                continue;
            }
            let mut bytes = vec![0; range.1 as usize];
            mem.read(range.0, &mut bytes);
            let hash = hash_bytes(&bytes);
            self.hashes.insert(range, hash);
            if self.held.insert(range, hash) != Some(hash) {
                self.batch.push(Packet::Memory(range.0, bytes));
            }
        }
        self.batch.push(Packet::Draw {
            primitive: draw.primitive,
            vat: draw.vat,
            count: draw.count,
            data: draw.data.to_vec(),
        });
        if self.batch.len() >= BATCH {
            self.send();
        }
    }

    /// What the render thread's copies wrote so far, without waiting for it, each with the
    /// memory as its copy found it: the game runs on, up to `AHEAD` frames, before the bytes
    /// land, and what it wrote meanwhile stays (`Written::before`). A copy whose memory this
    /// thread didn't keep, one kept on the GPU, writes nothing.
    fn take_written(&mut self) -> Vec<ssbm_gx::Written> {
        let done = std::mem::take(&mut *self.done.written.lock().unwrap());
        let mut out = Vec::new();
        for (copy, rows) in done {
            while self.before.front().is_some_and(|b| b.0 < copy) {
                self.before.pop_front();
            }
            let Some((_, start, before)) = self.before.pop_front_if(|b| b.0 == copy) else {
                continue;
            };
            for (at, bytes) in rows {
                let from = (at & 0x01FF_FFFF).wrapping_sub(start) as usize;
                let Some(old) = before.get(from..from + bytes.len()) else {
                    continue;
                };
                out.push(ssbm_gx::Written {
                    address: at,
                    bytes,
                    before: Some(old.to_vec()),
                });
            }
        }
        out
    }

    fn copy(&mut self, state: &State, value: u32, mem: &dyn Memory) {
        self.changes(state);
        let frame_end = bits(value, 14, 1) != 0;
        if frame_end {
            for (start, end) in self.footprints.take_overwritten(mem) {
                self.batch.push(Packet::Forget(start, end));
            }
        } else if let Some(f) = copy::footprint(state, value) {
            if copy::request(state, value).is_none() {
                let (start, end) = f.span();
                let start = start & 0x01FF_FFFF;
                let mut bytes = vec![0; end.saturating_sub(f.address) as usize];
                mem.read(start, &mut bytes);
                self.before.push_back((self.copies, start, bytes));
            }
            self.footprints.record(f, mem);
        }
        if !frame_end {
            self.copies += 1;
        }
        self.batch.push(Packet::Copy(value));
        if frame_end {
            // A frame: hand it over, and wait while the renderer is too far behind.
            self.send();
            self.hashes.clear();
            self.frames += 1;
            let mut done = self.done.frames.lock().unwrap();
            while *done + AHEAD < self.frames {
                done = self.done.cond.wait(done).unwrap();
            }
        }
    }
}

/// The memory textures read, as the game thread sent it.
struct Held(HashMap<(u32, u32), Vec<u8>>);

impl Memory for Held {
    fn read(&self, phys: u32, out: &mut [u8]) {
        match self.0.get(&(phys, out.len() as u32)) {
            Some(bytes) => out.copy_from_slice(bytes),
            None => out.fill(0),
        }
    }
}

/// Decodes and transforms each draw's vertices ahead of the render thread, with a copy of the
/// state of its own.
fn prepare_all(rx: &mpsc::Receiver<Vec<Packet>>, tx: &mpsc::Sender<Vec<Packet>>) {
    let mut state = State::new();
    let mut normals = [[0.0; 3]; 3];
    let mut inputs = Vec::new();
    while let Ok(batch) = rx.recv() {
        let mut out = Vec::with_capacity(batch.len());
        for packet in batch {
            match packet {
                Packet::Changes(delta) => {
                    state.apply(&delta);
                    out.push(Packet::Changes(delta));
                }
                Packet::Draw {
                    primitive,
                    vat,
                    count,
                    data,
                } => {
                    let layout = vertex::layout(&state.cp, usize::from(vat));
                    let draw = Draw {
                        primitive,
                        vat,
                        count,
                        layout: &layout,
                        data: &data,
                    };
                    let mut outs = Vec::new();
                    let drawn = prepare(&state, &draw, &mut normals, &mut inputs, &mut outs);
                    out.push(Packet::Prepared {
                        primitive,
                        vat,
                        count,
                        data,
                        outs: drawn.then_some(outs),
                    });
                }
                other => out.push(other),
            }
        }
        if tx.send(out).is_err() {
            return;
        }
    }
}

fn run(mut renderer: Renderer, rx: &mpsc::Receiver<Vec<Packet>>, done: &Done) {
    let mut state = State::new();
    let mut copies = 0u64;
    let mut memory = Held(HashMap::new());
    while let Ok(batch) = rx.recv() {
        for packet in batch {
            match packet {
                Packet::Changes(delta) => state.apply(&delta),
                Packet::Memory(at, bytes) => {
                    memory.0.insert((at, bytes.len() as u32), bytes);
                }
                Packet::Draw {
                    primitive,
                    vat,
                    count,
                    data,
                } => {
                    let layout = vertex::layout(&state.cp, usize::from(vat));
                    let draw = Draw {
                        primitive,
                        vat,
                        count,
                        layout: &layout,
                        data: &data,
                    };
                    renderer.draw(&state, &draw, &memory);
                }
                Packet::Prepared {
                    primitive,
                    vat,
                    count,
                    data,
                    outs,
                } => {
                    let layout = vertex::layout(&state.cp, usize::from(vat));
                    let draw = Draw {
                        primitive,
                        vat,
                        count,
                        layout: &layout,
                        data: &data,
                    };
                    renderer.draw_prepared(&state, &draw, outs.as_deref(), &memory);
                }
                Packet::Forget(start, end) => renderer.forget(start, end),
                Packet::Copy(value) => {
                    renderer.copy(&state, value, &memory);
                    if bits(value, 14, 1) != 0 {
                        *done.frames.lock().unwrap() += 1;
                        done.cond.notify_all();
                    } else {
                        let rows: Vec<(u32, Vec<u8>)> = renderer
                            .take_written()
                            .into_iter()
                            .map(|w| (w.address, w.bytes))
                            .collect();
                        if !rows.is_empty() {
                            done.written.lock().unwrap().push((copies, rows));
                        }
                        copies += 1;
                    }
                }
            }
        }
    }
}
