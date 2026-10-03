// SPDX-License-Identifier: GPL-3.0-or-later

//! The renderer on a thread of its own. The game thread keeps the GX state and sends what each
//! draw and copy needs: the state's changes since the last, the vertices, and the memory the
//! textures read. A renderer on another thread draws from a copy of the state kept by those
//! changes, so the game runs on while it draws, at most `AHEAD` frames behind.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, mpsc};

use ssbm_gx::{Delta, Draw, Memory, Sink, State, vertex};

use crate::{Renderer, bits, hash_bytes, sampled_image, texture_maps};

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
    Copy(u32),
}

/// Frames the render thread has finished.
#[derive(Default)]
struct Done {
    frames: Mutex<u64>,
    cond: Condvar,
}

/// A `Renderer` on its own thread, as a sink for the game thread's GX stream.
pub struct Threaded {
    tx: Option<mpsc::Sender<Vec<Packet>>>,
    thread: Option<std::thread::JoinHandle<()>>,
    batch: Vec<Packet>,
    /// Hashes of the memory textures read this frame, so each range hashes once a frame.
    hashes: HashMap<(u32, u32), u64>,
    /// What the render thread holds of each range, by hash.
    held: HashMap<(u32, u32), u64>,
    done: Arc<Done>,
    frames: u64,
}

impl Threaded {
    pub fn spawn(renderer: Renderer) -> Self {
        let (tx, rx) = mpsc::channel();
        let done = Arc::new(Done::default());
        let finished = done.clone();
        let thread = std::thread::Builder::new()
            .name("render".to_owned())
            .spawn(move || run(renderer, &rx, &finished))
            .expect("the render thread");
        Self {
            tx: Some(tx),
            thread: Some(thread),
            batch: Vec::new(),
            hashes: HashMap::new(),
            held: HashMap::new(),
            done,
            frames: 0,
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
        if let Some(thread) = self.thread.take() {
            thread.join().expect("the render thread");
        }
    }

    fn draw(&mut self, state: &State, draw: &Draw<'_>, mem: &dyn Memory) {
        self.changes(state);
        for map in texture_maps(state) {
            let image = sampled_image(state, map);
            let range = (image.address, image.size());
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

    fn copy(&mut self, state: &State, value: u32, _mem: &dyn Memory) {
        self.changes(state);
        self.batch.push(Packet::Copy(value));
        if bits(value, 14, 1) != 0 {
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

fn run(mut renderer: Renderer, rx: &mpsc::Receiver<Vec<Packet>>, done: &Done) {
    let mut state = State::new();
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
                Packet::Copy(value) => {
                    renderer.copy(&state, value, &memory);
                    if bits(value, 14, 1) != 0 {
                        *done.frames.lock().unwrap() += 1;
                        done.cond.notify_all();
                    }
                }
            }
        }
    }
}
