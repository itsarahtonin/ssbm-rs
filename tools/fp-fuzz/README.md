# fp-fuzz

Differential fuzzing of `gekko-fp` and the interpreter against Dolphin's float instructions, compiled from Dolphin's own source. This is the Phase 0 gate.

Every float instruction the game uses runs through both implementations on random inputs, constructed rounding ties, and every combination of special values. Register bits and CR must match exactly:

- `FmaMode::Hardware` against current Dolphin master, which matches the console.
- `FmaMode::SlippiDolphin` against Slippi's Dolphin fork, which recorded the replays.

```sh
cargo test -p fp-fuzz --release
FP_FUZZ_ITERS=2000000 cargo test -p fp-fuzz --release
```

## Vendored Dolphin source

`dolphin/` holds unmodified files from Dolphin (GPL-2.0-or-later):

- `master/`: [dolphin-emu/dolphin](https://github.com/dolphin-emu/dolphin) at `5102a0339c2177575378107b76541e47cc52122d`.
- `slippi/`: [project-slippi/dolphin](https://github.com/project-slippi/dolphin) at `41a7a3a110ed52999486ae1901c8fbb9a63d4f13`.
- `common/`: headers from master that both variants share.

`cpp/stubs/` replaces the parts of Dolphin these files include but do not need, such as the rest of the CPU state. `cpp/unity.cpp` compiles each variant inside its own namespace so both link into one test binary.
