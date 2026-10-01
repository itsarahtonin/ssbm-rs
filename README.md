# ssbm-rs

A Rust port of Super Smash Bros. Melee (NTSC 1.02, GALE01 revision 2) for Windows, macOS and Linux, built from the [doldecomp/melee](https://github.com/doldecomp/melee) decompilation.

- **Stage 1** is a faithful port, verified bit for bit against the original game.
- **Stage 2** refactors it into idiomatic Rust with modern features, verified frame by frame against Stage 1.

This repository contains no game data. The port reads everything from the player's own disc image and checks its hash first.

The full plan lives in the [Melee Rust Port Plan](https://claude.ai/code/artifact/341d5fed-6bcb-4070-89e3-c0e1276f0816).

## Status

Phase 3: everything else. Every function of the game is in Rust: `tools/c2rs` translates the decomp's C, and transliterates from machine code what only MWCC can compile. Lockstep checks each port against the original code, call by call, and the replay oracle checks whole games against their Slippi recordings.

| Crate | Purpose |
| --- | --- |
| [`gekko-fp`](crates/gekko-fp) | Bit-exact Gekko (GameCube CPU) floating-point operations |
| [`ssbm-mem`](crates/mem) | GameCube main memory: 24 MB, big-endian, 32-bit addresses |
| [`ssbm-disc`](crates/disc) | Disc image loading and verification, through nod |
| [`ssbm-rt`](crates/rt) | Runtime core: machine context, handles, calls and lockstep |
| [`ssbm-ppc`](crates/ppc) | Gekko interpreter that runs the original code in dev builds |
| [`ssbm-sdk`](crates/sdk) | Stand-ins for the hardware and the parts of Nintendo's SDK that talk to it |
| [`ssbm-types`](crates/types) | Generated handles, constants, globals and call stubs for the decomp's types and functions |
| [`ssbm-game`](crates/game) | The game's code ported to Rust |
| [`ssbm-slippi`](crates/slippi) | Slippi replay playback and recording |
| [`ssbm-run`](tools/run) | Runs the game headless, with lockstep, probes and the replay oracle |
| [`replay-sort`](tools/replay-sort) | Sorts Slippi replays and picks the smoke set |
| [`fp-fuzz`](tools/fp-fuzz) | Differential fuzzing of `gekko-fp` against Dolphin |

The Python tools in `tools/typegen`, `tools/c2rs` and `tools/lockstep` generate the types and ports and track lockstep coverage.

## Building

Install Rust with [rustup](https://rustup.rs); `rust-toolchain.toml` pins the version. On Windows, the MSVC C++ build tools are also required.

```sh
cargo test
```

Tests that need the disc run only when `SSBM_DISC` points at an image:

```sh
SSBM_DISC=/path/to/melee.iso cargo test -p ssbm-disc
```

## Design rules

1. No game data in Git. Anything derived from a disc lives under `local/`, which is ignored.
2. Every float operation in game code goes through `gekko-fp`. The original assembly decides which operations are fused and which stay in double precision.
3. Game code reaches game state only through generated accessors such as `f.pos().x()`. They hide whether the storage is GameCube memory (Stage 1) or a native struct (Stage 2).

## License

Code in this repository is licensed under GPL-3.0-or-later; see [LICENSE](LICENSE). Parts of `gekko-fp` are ported from [Dolphin](https://github.com/dolphin-emu/dolphin) (GPL-2.0-or-later).

The decomp this port is based on carries no license. Super Smash Bros. Melee and its assets belong to Nintendo and HAL Laboratory.
