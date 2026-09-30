# c2rs

Translates the Melee decomp's C into the Rust ports in `crates/game`, one module per C file.

```sh
python tools/c2rs/c2rs.py <decomp root> local/typegen/types.json crates/game/src [unit[:function] ...]
```

With no units it translates every C file. `types.json` comes from `tools/typegen/extract.py`, and the decomp must be built (`build/GALE01`), since the translator reads the built DOL for string literal addresses and each unit's asm listing for its data sections. It writes `build/c2rs-report.json` in the decomp with what it translated and why it left each other function out.

How C maps to Rust:

- Struct fields go through the generated accessors in `ssbm-types`, and pointers are typed handles, so the code never sees raw offsets.
- Calls go through the generated dispatch stubs, so a call reaches a port, an SDK stand-in or the original code alike. Inline functions without an address are translated into the calling module, and so are functions the caller's asm never calls because MWCC inlined them: that code runs as part of the caller, as in the original, so patches to the function's own copy, such as Slippi's Gecko codes, do not reach it.
- Every float operation goes through `gekko-fp`. `a * b + c` and friends become the fused operations MWCC emits for them, and `float` results are rounded to single as the Gekko does.
- Integer arithmetic wraps, and division and shifts use the Gekko's results for the cases C leaves undefined.
- Locals whose address is taken, and struct and array locals, live on the emulated stack, so pointers to them are GameCube addresses.
- String literals point at their original addresses in the game's data.

A function using `goto`, inline asm or a static local is left out for now, so the original keeps running it. The generated code is formatted with rustfmt and should not be edited by hand: fix the translator, or port the function by hand and leave it out of translation.

Ports are checked with `ssbm-run --port UNITS --lockstep`, which runs the original next to every call of a port and compares memory and results, and without `--lockstep` against the replay oracle.
