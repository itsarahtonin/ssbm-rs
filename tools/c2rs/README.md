# c2rs

Translates the Melee decomp's C into the Rust ports in `crates/game`, one module per C file.

```sh
python tools/c2rs/c2rs.py <decomp root> local/typegen/types.json crates/game/src [unit[:function] ...]
```

With no units it translates every C file. `types.json` comes from `tools/typegen/extract.py`, and the decomp must be built (`build/GALE01`), since the translator reads the built DOL for string literal addresses and each unit's asm listing for its data sections. It writes `build/c2rs-report.json` in the decomp with what it translated and why it left each other function out. Both tools read the decomp as its matching build does, with `MUST_MATCH` defined, so records get the game's layouts and code takes the game's paths.

How C maps to Rust:

- Struct fields go through the generated accessors in `ssbm-types`, and pointers are typed handles, so the code never sees raw offsets.
- Calls go through the generated dispatch stubs, so a call reaches a port, an SDK stand-in or the original code alike. Inline functions without an address are translated into the calling module, and so are functions the caller's asm never calls because MWCC inlined them: that code runs as part of the caller, as in the original, so patches to the function's own copy, such as Slippi's Gecko codes, do not reach it. Where the caller's asm calls an out-of-line copy of an inline function instead, the call goes to that copy, in the unit or the one weak copy the linker kept, such as `sqrtf__Ff` for `sqrtf`: the copy keeps the fused multiply-adds MWCC gave it, which the inline C translated into the caller would round differently.
- Every float operation goes through `gekko-fp`. `a * b + c` and friends become the fused operations MWCC emits for them, and `float` results are rounded to single as the Gekko does. The operands of an add or multiply go in MWCC's order, which puts the one that needs more registers second: of two NaNs, the result carries the first one's, and a single-precision multiply rounds its second operand.
- Integer arithmetic wraps, and division and shifts use the Gekko's results for the cases C leaves undefined. 64-bit shifts and divisions call the runtime's helpers as MWCC's code does, except on constants, which MWCC folds.
- Operands are typed as MWCC types them: under `-enum int` an enum is a signed `int`, where clang makes an enum without negative values unsigned and converts the operand it meets to match, so a compare such as `kind < 0` stays signed. `x /= c` and `x %= c` by an integer constant work in x's type, so `length /= sizeof(u16)` divides a signed `length` signed, where standard C would convert it to unsigned.
- Locals whose address is taken, and struct and array locals, live on the emulated stack, so pointers to them are GameCube addresses. Each goes at the offset the original's frame gives it, where MWCC's debug info says (see below), so pointers to locals, reads past them and reads of what a function never wrote all reach what they do in the original.
- Where an expression's operands call something, they are evaluated as MWCC does: those that call first, right to left, and the loads around them after. A plain assignment reads its right side before its target's own side effects (`a[n++] = *p` reads `*p` before storing `n`); compound and struct assignments do the target's first.
- String literals point at their original addresses in the game's data.

Functions whose source is assembly, and C functions that only MWCC can compile (inline asm, code under `#ifdef __MWERKS__`), are ported from their machine code instead: `asm2rs.py` reads each instruction word from the decomp's listing and writes what the interpreter does for it, with its fields as constants, using the same helpers (`ssbm_rt::cpu`, `gekko-fp`). Branches within the function become a state machine over its blocks, and calls go through dispatch, which runs these ports on the registers as their callers left them, as the original code reads them. `FROM_MACHINE_CODE` in `c2rs.py` lists the few C functions ported this way anyway, with the reason. So are the functions whose machine code reads hardware registers and accesses them more than once: MWCC's scheduler orders those accesses, volatile as the register arrays are, and the hardware sees the order. So are functions the listing has and clang never sees, such as whole functions under `#ifdef __MWERKS__` and out-of-line copies of inline functions.

C functions that read a value they never set or may not have set, which clang warns of, are ported from their machine code too, since the original takes whatever a register held; so are functions whose code inlines such a function. When that register is one the function's caller set without passing it, the caller is ported from its machine code as well, and so on up while a caller leaves the register as it found it: `regflow.py` follows which registers each function's code reads before writing them, from the listings. A function whose dependence would pass through more than `FOLLOWED` callers is left be, as its `IGNORED` ones are: such chains only run through paths that never set the register. The other way round, code ported from machine code may read a register that a call leaves and the callee's C doesn't return, such as r3 after a call to a `void` function, which it returns as the value its own C never sets. No port of the callee's C sets that register, so the callees whose own instructions do are ported from their machine code too, found by following the register down through callees that leave it as a callee of theirs did (`callees_to_port`).

A function the translator cannot handle is left out, so the original keeps running it, and the report says why. The generated code is formatted with rustfmt and should not be edited by hand: fix the translator, or port the function by hand in `crates/game/src/manual` and leave it out of translation.

Ports are checked with `ssbm-run --port UNITS --lockstep`, which runs the original next to every call of a port and compares memory and results, and without `--lockstep` against the replay oracle.

## Frames

The original's frame layouts come from MWCC itself: `frames.py` compiles every C unit of the decomp's build with `-sym on`, which leaves each function's code as the matching build has it (the tool checks this, function by function), and reads the DWARF that decomp-toolkit dumps, into `frames.json` next to `types.json`:

```sh
python tools/c2rs/frames.py <decomp root> local/typegen/frames.json [unit ...]
```

It lists each function's parameters and locals in the order MWCC does, each in its register or at its offset from r1. c2rs matches a function's C locals to these by name and order, where both have as many of a name, and lays out the rest of its frame around them: in declaration order, reversed as MWCC lays them out, or largest first, whichever fits in the original's frame. Locals the C never names, the decomp's padding for MWCC's frames, get no slot. The port's frame keeps the original's size where it can, so the frames of the functions it calls lie at the original's addresses, which pointers to their locals carry into memory. Without the file, frames are laid out in declaration order.

## Slippi's playback codes

Replays run with Slippi's Gecko codes applied, which patch about 80 game functions. `patched.py` ports those functions from their machine code together with the codes, into `crates/game/src/patched.rs`:

```sh
python tools/c2rs/patched.py <decomp root> <replay.slp> [<replay.slp> ...]
```

The codes are the bootloader's and playback's (`crates/slippi/data`) and those each replay's list keeps. A port reads, at each address a code writes, the word there now and runs what it says, so it behaves as the original both before and after playback applies its codes. Injected instructions run where playback placed them, which the branch at the code's site gives; only the instructions reachable from their entry are transliterated, following where each return goes, so data among them, such as per-match settings, is read from memory as the original reads it. Callers of a code that returns past its function's caller resume where the code says. Places in injected code that other code calls, such as a process a code gives a GObj, are listed in `ENTRIES` and get their ports when first called; `ssbm-run` with `ORIGINAL_ENTRIES=1` reports any code that still runs as original.

`ssbm-run` uses a port when the replay's codes match the ones it was made for, and keeps the patched original code otherwise (`PATCHED_ORIGINAL=1` keeps it everywhere).
