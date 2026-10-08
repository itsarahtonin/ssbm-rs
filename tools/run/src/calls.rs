// SPDX-License-Identifier: GPL-3.0-or-later

//! Saved calls (ssbm_rt::capture) as files: CAPTURE=DIR saves each function's first two
//! mismatching checks, and with CAPTURE_FUNCS=NAME,... a few real calls of those functions
//! (spread out, and those that run code of them no check had run) and their first mutated
//! checks, named for how each ended, as DIR/NAME-KIND-N.call (zstd);
//! `--call FILE` checks one again apart from any run. With CORPUS=FILE the real calls go into
//! one corpus file instead, which keeps each distinct page of memory once, and `--corpus FILE`
//! checks every call in one again. The files hold captured game memory: keep them under local/.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::rc::Rc;

use ssbm_rt::Ctx;
use ssbm_rt::capture::{Call, Ended};

/// The instructions a call checked again may run before its check is dropped: one that waits
/// on a device whose state lives outside memory, such as the card at boot, would wait forever.
const REPLAY_MAX: u64 = 50_000_000;

thread_local! {
    /// Where the call being checked again runs past REPLAY_MAX, in instructions run, or 0.
    static DEADLINE: Cell<u64> = const { Cell::new(0) };
}

/// Whether the call being checked again has run past its deadline, at `executed`.
pub fn past_deadline(executed: u64) -> bool {
    DEADLINE.with(|d| d.get() != 0 && executed > d.get())
}

/// Whether a call is being checked again, apart from any run.
pub fn replaying() -> bool {
    DEADLINE.with(|d| d.get() != 0)
}

thread_local! {
    /// CALL_POKE=ADDR=BYTE,...: bytes to change in every call checked again, which then checks
    /// as a mutated check, to replay exactly the changed inputs a run's mismatch reported.
    static POKES: Vec<(u32, u8)> = std::env::var("CALL_POKE").map_or_else(|_| Vec::new(), |v| {
        v.split(',')
            .map(|p| {
                let (a, b) = p.split_once('=').expect("CALL_POKE=ADDR=BYTE,...");
                let num = |s: &str| u32::from_str_radix(s.trim().trim_start_matches("0x"), 16);
                (num(a).expect("ADDR"), num(b).expect("BYTE") as u8)
            })
            .collect()
    });
}

/// Checks `call` again from the state it was saved in.
fn check(ctx: &Ctx, call: &Call) {
    call.load(ctx);
    let poked = POKES.with(|pokes| {
        for &(addr, byte) in pokes {
            ctx.write_u8(addr, byte);
        }
        !pokes.is_empty()
    });
    ctx.lockstep.resume(call.mutated || poked, call.stub);
    DEADLINE.with(|d| d.set(ctx.executed() + REPLAY_MAX));
    let result = catch_unwind(AssertUnwindSafe(|| ctx.invoke(call.addr)));
    DEADLINE.with(|d| d.set(0));
    ctx.lockstep.resume(false, None);
    // A fault outside any check, as a saved mutated state makes where its function's checks
    // have ended, loses that call, not the calls after it.
    if let Err(p) = result {
        if p.is::<ssbm_rt::Stop>() {
            resume_unwind(p);
        }
        let why = p
            .downcast_ref::<ssbm_rt::Fault>()
            .map(|f| f.to_string())
            .or_else(|| p.downcast_ref::<String>().cloned())
            .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_else(|| "panic".to_owned());
        eprintln!(
            "corpus: a{} call of {} stopped outside its checks ({why}); going on",
            if call.mutated { " mutated" } else { "" },
            ctx.name_of(call.addr)
        );
    }
}

/// Mismatching checks saved per function.
const MISMATCHES: u32 = 2;

/// Mutated checks of a function CAPTURE_FUNCS names that are saved, to tell whether checking one
/// again apart from its run ends as it did there.
const MUTATED: u64 = 20;

/// The most instructions a real call's original may run to be saved: a call that runs a
/// scene's loop would run the game again when checked again.
const COST_MAX: u64 = 5_000_000;

/// Which real calls of a function CAPTURE_FUNCS asks for are saved, by count: spread out, so
/// they find the game in different states.
const SAVED_CALLS: [u64; 5] = [1, 10, 100, 1000, 10000];

/// Real calls of a function CAPTURE_FUNCS asks for that are saved besides those because they
/// ran instructions of it no check had verified before, as a fuzzer keeps inputs that reach new
/// code: their states are the ones mutated checks reach further from.
const NOVEL_CALLS: u32 = 20;

/// The most real calls CAPTURE_EVERY saves per function.
const EVERY_CALLS: u64 = 400;

pub fn read(path: &Path) -> Call {
    let packed = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let bytes = zstd::decode_all(&packed[..]).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Call::from_bytes(&bytes).unwrap_or_else(|| panic!("{}: not a saved call", path.display()))
}

fn write(path: &Path, call: &Call) {
    let packed = zstd::encode_all(&call.to_bytes()[..], 3).expect("zstd");
    std::fs::write(path, packed).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

/// Saves calls into `dir` as the module says: mismatches, and real calls of `funcs` from field
/// `from` on, into `corpus` if given. Calls during boot wait on devices whose state lives outside
/// memory, such as the card's, and would wait forever when checked again.
pub fn install_capture(
    ctx: &Ctx,
    dir: PathBuf,
    funcs: HashSet<u32>,
    corpus: Option<PathBuf>,
    from: u64,
) {
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let corpus = corpus.map(|p| RefCell::new(CorpusWriter::create(&p)));
    // CAPTURE_EVERY=N saves every Nth real call as well, up to EVERY_CALLS per function, for a
    // corpus of one function's many states.
    let every: u64 =
        std::env::var("CAPTURE_EVERY").map_or(0, |v| v.parse().expect("CAPTURE_EVERY=N"));
    // Per function: mismatches saved, real calls seen, mutated checks seen, real calls saved
    // for running new code.
    let counts: RefCell<BTreeMap<u32, (u32, u64, u64, u32)>> = RefCell::default();
    let saved = RefCell::new(0u32);
    *ctx.lockstep.capture.borrow_mut() = Some(Rc::new(move |ctx, ended: Ended, take| {
        let Ended { addr, mismatched, mutated, cost, novel, interacted } = ended;
        let mut counts = counts.borrow_mut();
        let (mismatches, calls, mutations, novels) = counts.entry(addr).or_default();
        let wanted = if mismatched {
            *mismatches += 1;
            *mismatches <= MISMATCHES
        } else if funcs.contains(&addr) && mutated {
            *mutations += 1;
            *mutations <= MUTATED
        } else if funcs.contains(&addr)
            && cost <= COST_MAX
            && (!interacted || std::env::var_os("CAPTURE_INTERACTED").is_some())
            && ctx.ext::<ssbm_sdk::Sdk>().hw.fields.get() >= from
        {
            *calls += 1;
            let spaced = every > 0 && *calls % every == 0 && *calls / every <= EVERY_CALLS;
            if SAVED_CALLS.contains(calls) || spaced {
                true
            } else if novel > 0 && *novels < NOVEL_CALLS {
                *novels += 1;
                true
            } else {
                false
            }
        } else {
            false
        };
        if !wanted {
            return;
        }
        if let Some(corpus) = corpus.as_ref().filter(|_| !mismatched && !mutated) {
            corpus.borrow_mut().add(&take());
            return;
        }
        let mut n = saved.borrow_mut();
        *n += 1;
        let kind = match (mismatched, mutated) {
            (true, true) => "mutated-mismatch",
            (true, false) => "mismatch",
            (false, true) => "mutated-ok",
            (false, false) => "call",
        };
        let path = dir.join(format!("{}-{kind}{}.call", ctx.name_of(addr), *n));
        write(&path, &take());
        eprintln!("saved {}", path.display());
    }));
}

/// Keeps original the functions whose code in `call`'s memory differs from the disc's, as a run
/// does with those its replay's Gecko codes patch (Slippi's): a call saved from a replay run
/// carries the patched code, and checking it against the unpatched port would find the patch.
fn keep_patched(ctx: &Ctx, dol: Option<&ssbm_disc::Dol>, call: &Call) {
    let Some(dol) = dol else { return };
    let mut kept = Vec::new();
    for section in dol.sections.iter().filter(|s| s.kind == ssbm_disc::SectionKind::Text) {
        let code = dol.section_data(section);
        let at = (section.addr - 0x8000_0000) as usize;
        let Some(saved) = call.mem1.get(at..at + code.len()) else { continue };
        for (i, (a, b)) in code.chunks_exact(4).zip(saved.chunks_exact(4)).enumerate() {
            if a != b {
                for f in ssbm_types::functions_overlapping(section.addr + 4 * i as u32, 4) {
                    if !kept.contains(&f) {
                        ctx.set_mode(f, ssbm_rt::Mode::Original);
                        kept.push(f);
                    }
                }
            }
        }
    }
    if !kept.is_empty() {
        let names: Vec<String> = kept.iter().map(|&f| ctx.name_of(f)).collect();
        eprintln!("patched in the saved memory, so kept original: {}", names.join(", "));
    }
}

/// Checks the saved call at `path` again, `repeat` times, each from the state it was saved in;
/// LOCKSTEP_MUTATE adds mutated checks of each as a run's checks do.
pub fn replay(ctx: &Ctx, dol: Option<&ssbm_disc::Dol>, path: &Path, repeat: u32) {
    let call = read(path);
    keep_patched(ctx, dol, &call);
    eprintln!(
        "checking {} again{}, {repeat} times",
        ctx.name_of(call.addr),
        if call.mutated { " as a mutated check" } else { "" }
    );
    for _ in 0..repeat {
        check(ctx, &call);
    }
}

const CORPUS_MAGIC: &[u8; 8] = b"SSBMCORP";
const PAGE: usize = 0x1000;

/// Writes a corpus: a page record (1) for each distinct page of memory, compressed, and a call
/// record (2) for each call, its registers and the pages its memory is made of.
struct CorpusWriter {
    out: BufWriter<std::fs::File>,
    pages: HashMap<u64, u32>,
}

impl CorpusWriter {
    fn create(path: &Path) -> Self {
        let file =
            std::fs::File::create(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut out = BufWriter::new(file);
        out.write_all(CORPUS_MAGIC).expect("corpus");
        Self { out, pages: HashMap::new() }
    }

    fn record(&mut self, kind: u8, payload: &[u8]) {
        let packed = zstd::encode_all(payload, 3).expect("zstd");
        self.out.write_all(&[kind]).expect("corpus");
        self.out.write_all(&(packed.len() as u32).to_le_bytes()).expect("corpus");
        self.out.write_all(&packed).expect("corpus");
    }

    fn add(&mut self, call: &Call) {
        let mut ids = Vec::with_capacity((call.mem1.len() + call.locked.len()) / PAGE);
        for page in call.mem1.chunks(PAGE).chain(call.locked.chunks(PAGE)) {
            let mut h = DefaultHasher::new();
            page.hash(&mut h);
            let key = h.finish();
            let id = match self.pages.get(&key) {
                Some(&id) => id,
                None => {
                    let id = self.pages.len() as u32;
                    self.record(1, page);
                    self.pages.insert(key, id);
                    id
                }
            };
            ids.push(id);
        }
        let head = Call {
            mem1: Vec::new(),
            locked: Vec::new(),
            regs: call.regs.clone(),
            ..*call
        }
        .to_bytes();
        let mut payload = Vec::with_capacity(8 + head.len() + 4 * ids.len());
        payload.extend((head.len() as u32).to_le_bytes());
        payload.extend(&head);
        payload.extend((call.mem1.len() as u32).to_le_bytes());
        ids.iter().for_each(|id| payload.extend(id.to_le_bytes()));
        self.record(2, &payload);
        self.out.flush().expect("corpus");
    }
}

/// A corpus read back: its pages, still compressed, and its calls without their memory.
pub struct Corpus {
    pages: Vec<Vec<u8>>,
    calls: Vec<(Call, usize, Vec<u32>)>,
}

impl Corpus {
    pub fn read(path: &Path) -> Self {
        let b = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(b.starts_with(CORPUS_MAGIC), "{}: not a corpus", path.display());
        let (mut pages, mut calls) = (Vec::new(), Vec::new());
        let mut at = CORPUS_MAGIC.len();
        // A record cut short ends the corpus, as one a run stopped while writing.
        while at + 5 <= b.len() {
            let kind = b[at];
            let len = u32::from_le_bytes(b[at + 1..at + 5].try_into().unwrap()) as usize;
            let Some(packed) = b.get(at + 5..at + 5 + len) else {
                break;
            };
            at += 5 + len;
            if kind == 1 {
                pages.push(packed.to_vec());
                continue;
            }
            let payload = zstd::decode_all(packed).expect("zstd");
            let n = u32::from_le_bytes(payload[..4].try_into().unwrap()) as usize;
            let head = Call::from_bytes(&payload[4..4 + n]).expect("a saved call");
            let mem1 = u32::from_le_bytes(payload[4 + n..8 + n].try_into().unwrap()) as usize;
            let ids = payload[8 + n..]
                .chunks(4)
                .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
                .collect();
            calls.push((head, mem1, ids));
        }
        Self { pages, calls }
    }

    pub fn len(&self) -> usize {
        self.calls.len()
    }

    /// Call `i`, with its memory.
    pub fn call(&self, i: usize) -> Call {
        let (head, mem1, ids) = &self.calls[i];
        let mut mem = Vec::with_capacity(ids.len() * PAGE);
        for &id in ids {
            mem.extend(zstd::decode_all(&self.pages[id as usize][..]).expect("zstd"));
        }
        let locked = mem.split_off(*mem1);
        Call {
            mem1: mem,
            locked,
            regs: head.regs.clone(),
            ..*head
        }
    }
}

/// Checks every call in the corpus at `path` again, `repeat` times each. With CORPUS_FEEDBACK=N,
/// up to N states a call's mutated checks started from are kept when they reached code no check
/// had verified and met no hardware, and are checked again in turn, mutated further: blocks
/// behind several conditions are reached a condition at a time, as a coverage-guided fuzzer
/// reaches them from the inputs it keeps.
pub fn replay_corpus(ctx: &Ctx, dol: Option<&ssbm_disc::Dol>, path: &Path, repeat: u32) {
    let corpus = Corpus::read(path);
    // A corpus comes from one run, so its first call has the code all of them run.
    if corpus.len() > 0 {
        keep_patched(ctx, dol, &corpus.call(0));
    }
    eprintln!(
        "checking the {} calls of {} again, {repeat} times each",
        corpus.len(),
        path.display()
    );
    let verbose = std::env::var_os("CORPUS_VERBOSE").is_some();
    let feedback: usize =
        std::env::var("CORPUS_FEEDBACK").map_or(0, |v| v.parse().expect("CORPUS_FEEDBACK=N"));
    let base: Rc<RefCell<Option<Rc<Call>>>> = Rc::default();
    let kept: Rc<RefCell<Vec<Derived>>> = Rc::default();
    if feedback > 0 {
        let (base, kept) = (base.clone(), kept.clone());
        // CAPTURE's own saving goes on.
        let saving = ctx.lockstep.capture.borrow().clone();
        *ctx.lockstep.capture.borrow_mut() = Some(Rc::new(move |ctx, ended: Ended, take| {
            if let Some(save) = &saving {
                save(ctx, ended, take);
            }
            let wanted = ended.mutated && !ended.mismatched && !ended.interacted && ended.novel > 0;
            let base = base.borrow();
            let Some(base) = base.as_ref().filter(|_| wanted) else { return };
            if kept.borrow().len() < feedback {
                kept.borrow_mut().push(Derived::from(base, &take()));
            }
        }));
    }
    let mut derived = 0;
    for i in 0..corpus.len() {
        let call = Rc::new(corpus.call(i));
        if verbose {
            eprintln!("call {i}: {}", ctx.name_of(call.addr));
        }
        *base.borrow_mut() = Some(call.clone());
        for _ in 0..repeat {
            check(ctx, &call);
        }
        // The kept states, oldest first; checking them may keep more, up to the limit. Each is
        // a mutated state, checked as mutated checks are (against null hardware, dropped where
        // its original fails), and mutated further.
        let mut next = 0;
        loop {
            // Once the function's checks have ended, as they do when its needed blocks are
            // verified, its calls run unchecked, and a mutated state must not.
            let checked = ctx.entry(call.addr).is_some_and(|e| e.mode == ssbm_rt::Mode::Lockstep);
            let state = kept.borrow().get(next).filter(|_| checked).map(|d| d.apply(&call));
            let Some(state) = state else { break };
            next += 1;
            ctx.lockstep.reseeding.set(true);
            check(ctx, &state);
            ctx.lockstep.reseeding.set(false);
        }
        derived += next;
        kept.borrow_mut().clear();
    }
    *base.borrow_mut() = None;
    if feedback > 0 {
        eprintln!("checked {derived} states mutated checks reached new code from");
    }
}

/// A state a mutated check started from, as what it changed in the call it mutated: a few
/// bytes, where the call holds all of memory.
struct Derived {
    regs: ssbm_rt::RegsSnapshot,
    changed: Vec<(usize, Vec<u8>)>,
}

impl Derived {
    fn from(base: &Call, state: &Call) -> Self {
        let mut changed: Vec<(usize, Vec<u8>)> = Vec::new();
        for (page, (a, b)) in base.mem1.chunks(4096).zip(state.mem1.chunks(4096)).enumerate() {
            if a == b {
                continue;
            }
            for (i, (&x, &y)) in a.iter().zip(b).enumerate() {
                if x == y {
                    continue;
                }
                let at = page * 4096 + i;
                match changed.last_mut() {
                    Some((start, bytes)) if *start + bytes.len() == at => bytes.push(y),
                    _ => changed.push((at, vec![y])),
                }
            }
        }
        Self { regs: state.regs.clone(), changed }
    }

    /// The state, rebuilt from the call it came from, to check as a call of its own.
    fn apply(&self, base: &Call) -> Call {
        let mut mem1 = base.mem1.clone();
        for (at, bytes) in &self.changed {
            mem1[*at..*at + bytes.len()].copy_from_slice(bytes);
        }
        Call {
            addr: base.addr,
            mutated: true,
            stub: None,
            regs: self.regs.clone(),
            mem1,
            locked: base.locked.clone(),
        }
    }
}
