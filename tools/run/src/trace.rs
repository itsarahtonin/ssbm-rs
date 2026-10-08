//! `STATE_TRACE=FILE`: the game's state at the end of every engine tick, for melee-hd's oracle
//! to compare its own simulation with.
//!
//! The scene loop (`gm_801A4D34`) runs one engine tick per queued controller sample and ends
//! each with `HSD_PerfSetCPUTime`, so its port is wrapped to write a record after it. A record
//! holds raw memory, big-endian as the game keeps it; melee-hd decodes it.
//!
//! The file is a series of zstd frames, one per `CHUNK` ticks. Decompressed, it starts with
//! `MAGIC` and a version (u32), then each tick: its number (u32, from 0), a region count
//! (u32), and per region its kind (u8, `Kind`), address (u32), owning GObj (u32, 0 for
//! globals) and length (u32), then its bytes. Header integers are little-endian.
//!
//! With `STATE_TRACE_MEMORY=FROM..TO` (ticks; `TO` may be left out, and `1` means from the
//! first), the ticks in that window also hold main memory as `Page` regions (version 3): every
//! 4 KiB page that changed since the previous tick, or every nonzero page at the window's first
//! tick, XORed with what it held then, so a reader keeps an image of memory and XORs each page
//! into it. The main thread's stack and the GX FIFO are left out: neither is game state, and
//! both change every tick. Since version 4, those ticks also hold the time base (`Time`).

use std::cell::{Cell, RefCell};
use std::fs::File;
use std::io::{BufWriter, Write};

use ssbm_rt::{Ctx, Mode, Native};

const MAGIC: &[u8; 8] = b"SSBMTRC\0";
const VERSION: u32 = 4;
const CHUNK: u32 = 300;

/// What a region holds.
#[repr(u8)]
#[derive(Clone, Copy)]
enum Kind {
    /// `seed`, the game's random number state.
    Seed = 0,
    /// `player_slots`, the six players' static blocks.
    Players = 1,
    /// `stage_info`.
    Stage = 2,
    /// `game_camera`.
    Camera = 3,
    /// A fighter's `Fighter`, from the fighters' p_link.
    Fighter = 4,
    /// An item's `Item`, from the items' p_link.
    Item = 5,
    /// A joint of a fighter's model (`HSD_JObj`), in the tree's pre-order; the region's GObj is
    /// the fighter's.
    JObj = 6,
    /// A page of main memory XORed with its contents at the previous tick (`STATE_TRACE_MEMORY`).
    Page = 7,
    /// The time base at the tick's end (8 bytes, big-endian), with memory: what melee-hd's
    /// audio timeline starts from when it takes up a tick.
    Time = 8,
}

const FIGHTER_SIZE: u32 = 0x23EC;
const ITEM_SIZE: u32 = 0xFCC;
const PLINK_FIGHTER: u32 = 8;
const PLINK_ITEM: u32 = 9;
const JOBJ_SIZE: u32 = 0x88;
const JOBJ_INSTANCE: u32 = 1 << 12;
const MEM1: u32 = 0x8000_0000;
const MEM1_SIZE: usize = 24 << 20;
const PAGE: usize = 0x1000;
/// The main thread's stack (`__init_registers`' r1 and the 64 KiB below it).
const STACK: std::ops::Range<u32> = 0x804D_EC00..0x804E_EC00;

struct Trace {
    out: Option<zstd::Encoder<'static, BufWriter<File>>>,
    file: Option<BufWriter<File>>,
    tick: u32,
    globals: [(Kind, u32, u32); 4],
    plink_heads: u32,
    /// The ticks whose memory the trace holds, and main memory at the previous one.
    window: Option<std::ops::Range<u32>>,
    memory: Option<Vec<u8>>,
    fifo_obj: u32,
}

thread_local! {
    static ORIGINAL: Cell<Option<Native>> = const { Cell::new(None) };
    static TRACE: RefCell<Option<Trace>> = const { RefCell::new(None) };
}

/// Wraps `HSD_PerfSetCPUTime`'s port to write a record to `path` after every tick.
pub fn install(ctx: &Ctx, path: &str) {
    let sym = ssbm_sdk::sym;
    let at = sym("HSD_PerfSetCPUTime");
    let entry = ctx
        .entry(at)
        .filter(|e| e.mode == Mode::Native)
        .unwrap_or_else(|| panic!("STATE_TRACE needs HSD_PerfSetCPUTime's port (--port)"));
    ORIGINAL.with(|o| o.set(Some(entry.native)));
    ctx.register(at, after_tick);
    ctx.set_mode(at, entry.mode);
    let mut file = BufWriter::new(File::create(path).unwrap_or_else(|e| panic!("{path}: {e}")));
    file.write_all(MAGIC).unwrap();
    file.write_all(&VERSION.to_le_bytes()).unwrap();
    TRACE.with(|t| {
        *t.borrow_mut() = Some(Trace {
            out: None,
            file: Some(file),
            tick: 0,
            globals: [
                (Kind::Seed, sym("seed"), 4),
                (Kind::Players, sym("player_slots"), 0x5760),
                (Kind::Stage, sym("stage_info"), 0x748),
                (Kind::Camera, sym("game_camera"), 0x39C),
            ],
            plink_heads: sym("HSD_GObjPLinkHead"),
            window: std::env::var("STATE_TRACE_MEMORY").ok().map(|w| window(&w)),
            memory: None,
            fifo_obj: sym("DefaultFifoObj"),
        })
    });
}

/// The ticks `STATE_TRACE_MEMORY` names: `FROM..TO`, `FROM..` or `1` (every tick).
fn window(w: &str) -> std::ops::Range<u32> {
    if w == "1" {
        return 0..u32::MAX;
    }
    let tick = |s: &str| s.parse::<u32>().unwrap_or_else(|_| panic!("STATE_TRACE_MEMORY={w}"));
    match w.split_once("..") {
        Some((from, "")) => tick(from)..u32::MAX,
        Some((from, to)) => tick(from)..tick(to),
        None => panic!("STATE_TRACE_MEMORY={w}: FROM..TO, FROM.. or 1"),
    }
}

/// Finishes the file. Ticks after this are not written.
pub fn finish() {
    TRACE.with(|t| {
        if let Some(mut trace) = t.borrow_mut().take() {
            trace.end_chunk();
            if let Some(mut file) = trace.file.take() {
                file.flush().unwrap();
            }
        }
    });
}

fn after_tick(ctx: &Ctx) {
    if let Some(original) = ORIGINAL.with(Cell::get) {
        original(ctx);
    }
    TRACE.with(|t| {
        if let Some(trace) = t.borrow_mut().as_mut() {
            trace.record(ctx);
        }
    });
}

/// A joint tree in pre-order, as `HSD_JObjWalkTree` visits it: an instance's children belong
/// to the tree it instances.
fn joints(ctx: &Ctx, gobj: u32, root: u32, regions: &mut Vec<(Kind, u32, u32, u32)>) {
    let mut stack = vec![root];
    while let Some(j) = stack.pop() {
        if j == 0 {
            continue;
        }
        regions.push((Kind::JObj, j, gobj, JOBJ_SIZE));
        stack.push(ctx.read_u32(j + 8));
        if ctx.read_u32(j + 0x14) & JOBJ_INSTANCE == 0 {
            stack.push(ctx.read_u32(j + 0x10));
        }
    }
}

impl Trace {
    fn record(&mut self, ctx: &Ctx) {
        let mut regions: Vec<(Kind, u32, u32, u32)> =
            self.globals.iter().map(|&(k, a, l)| (k, a, 0, l)).collect();
        let heads = ctx.read_u32(self.plink_heads);
        if heads != 0 {
            for (link, kind, size) in [
                (PLINK_FIGHTER, Kind::Fighter, FIGHTER_SIZE),
                (PLINK_ITEM, Kind::Item, ITEM_SIZE),
            ] {
                let mut gobj = ctx.read_u32(heads + 4 * link);
                while gobj != 0 {
                    let data = ctx.read_u32(gobj + 0x2C);
                    if data != 0 {
                        regions.push((kind, data, gobj, size));
                    }
                    if link == PLINK_FIGHTER {
                        joints(ctx, gobj, ctx.read_u32(gobj + 0x28), &mut regions);
                    }
                    gobj = ctx.read_u32(gobj + 0x8);
                }
            }
        }
        let mut pages = Vec::new();
        if self.window.as_ref().is_some_and(|w| w.contains(&self.tick)) {
            let before = self.memory.get_or_insert_with(|| vec![0; MEM1_SIZE]);
            let mut now = vec![0; MEM1_SIZE];
            ctx.mem.read_bytes(MEM1, &mut now).expect("main memory");
            // The FIFO `HSD_AllocateFifo` made, once `HSD_GXInit` has set it up.
            let fifo = ctx.read_u32(self.fifo_obj);
            let fifo = if fifo != 0 {
                let base = ctx.read_u32(fifo);
                base..base.wrapping_add(ctx.read_u32(fifo + 8))
            } else {
                0..0
            };
            let pages_iter = before.chunks_exact(PAGE).zip(now.chunks_exact(PAGE)).enumerate();
            for (i, (old, new)) in pages_iter {
                let at = MEM1 + (i * PAGE) as u32;
                let page = at..at + PAGE as u32;
                let overlaps = |r: &std::ops::Range<u32>| page.start < r.end && r.start < page.end;
                if old != new && !overlaps(&STACK) && !overlaps(&fifo) {
                    let xor: Vec<u8> = old.iter().zip(new).map(|(a, b)| a ^ b).collect();
                    pages.push((at, xor));
                }
            }
            *before = now;
        }
        let out = self.out.get_or_insert_with(|| {
            let file = self.file.take().expect("trace file");
            zstd::Encoder::new(file, 3).unwrap()
        });
        let mut put = |b: &[u8]| out.write_all(b).unwrap();
        put(&self.tick.to_le_bytes());
        let time = usize::from(
            self.memory.is_some() && self.window.as_ref().is_some_and(|w| w.contains(&self.tick)),
        );
        put(&((regions.len() + pages.len() + time) as u32).to_le_bytes());
        let mut bytes = Vec::new();
        for (kind, address, gobj, len) in regions {
            bytes.resize(len as usize, 0);
            if ctx.mem.read_bytes(address, &mut bytes).is_err() {
                bytes.fill(0);
            }
            put(&[kind as u8]);
            put(&address.to_le_bytes());
            put(&gobj.to_le_bytes());
            put(&len.to_le_bytes());
            put(&bytes);
        }
        if self.memory.is_some() && self.window.as_ref().is_some_and(|w| w.contains(&self.tick)) {
            put(&[Kind::Time as u8]);
            put(&0u32.to_le_bytes());
            put(&0u32.to_le_bytes());
            put(&8u32.to_le_bytes());
            put(&ctx.regs.tb.get().to_be_bytes());
        }
        for (address, xor) in pages {
            put(&[Kind::Page as u8]);
            put(&address.to_le_bytes());
            put(&0u32.to_le_bytes());
            put(&(PAGE as u32).to_le_bytes());
            put(&xor);
        }
        self.tick += 1;
        if self.tick.is_multiple_of(CHUNK) {
            self.end_chunk();
        }
    }

    /// Ends the current zstd frame, so a run cut short leaves a readable file.
    fn end_chunk(&mut self) {
        if let Some(out) = self.out.take() {
            let mut file = out.finish().unwrap();
            file.flush().unwrap();
            self.file = Some(file);
        }
    }
}
