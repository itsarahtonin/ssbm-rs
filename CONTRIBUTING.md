# Contributing

ssbm-rs preserves Melee's original behavior. Changes should improve translation, runtime fidelity, verification, or documentation while keeping the evidence reviewable. Modern gameplay features are planned for a separate project.

## Requirements

Use the Rust version in `rust-toolchain.toml`. Windows needs Visual Studio's C++ build tools. Linux player builds need pkg-config and the ALSA and udev development libraries. C++ reference tools need C++23; `fp-fuzz` additionally uses AVX2 and FMA.

Building committed Rust sources does not require the decompilation or Python. Regeneration requires a configured and built matching decompilation, Python with libclang, and disc-derived inputs described in [c2rs](tools/c2rs/README.md). The cloud setup pins the decomp revision; do not silently regenerate from a different revision.

## Checks without game data

```sh
cargo test --locked --release -p gekko-fp -p ssbm-mem -p ssbm-disc -p ssbm-rt -p ssbm-ppc -p ssbm-gx -p ssbm-ax
```

Basic CI runs this subset on the three target operating systems. Disc tests report that they skip when `SSBM_DISC` is unset. Renderer device tests and differential fuzzers are separate; this subset is not full game verification.

Format only changed files. The repository is not globally rustfmt-clean; avoid unrelated formatting changes.

## Checks with your own disc

Set `SSBM_DISC` to your Melee US NTSC 1.02 image, then run relevant checks:

```sh
SSBM_DISC=/path/to/melee.iso cargo test --locked --release -p ssbm-disc
cargo build --locked --release -p ssbm-run
target/release/ssbm-run /path/to/melee.iso --replay /path/to/game.slp --fields 3600 --port all --lockstep
```

In PowerShell, set `$env:SSBM_DISC = 'C:/path/to/melee.iso'` and use `ssbm-run.exe`. A replay needs enough fields to reach its end; 3,600 is an example limit. Known recording differences require the replay's reviewed `--known` file, not a newly accepted blanket exception.

[Campaigns](tools/lockstep/campaign/README.md) explain coverage and reproductions. [Graphics and audio](tools/gx/README.md) explain stream, image, audio, and controller checks.

## Generated code

| Output | Change its source here |
| --- | --- |
| `crates/types/src/gen/` | `tools/typegen/` |
| `crates/game/src/tu/` | `tools/c2rs/` or a reviewed implementation in `crates/game/src/manual/` |
| `crates/game/src/patched.rs` | `tools/c2rs/patched.py` |
| `crates/slippi/src/denylist.rs` | Upstream injection lists and the procedure documented in that file |

Output stays committed so the port builds without its generation environment. Include relevant regenerated output with generator fixes and explain the effect.

## Bug reports and working conventions

Include source commit, operating system, toolchain, options, supported disc revision, and the first failure. State whether original-code execution reproduces it and whether mutations or replay patches are involved.

Review logs before sharing. Never upload discs, extracted files, captures, cards, or personal replays. Keep game data under ignored `local/` or outside the checkout.

Use a separate branch and worktree. Keep fixes focused and comments concise. Commit types use a capitalized first word, such as `fix: Preserve fused operand order`. Keep prose paragraphs on one source line.

Run frozen binary copies for long campaigns; never edit a Bash script while a campaign is reading it. Copy card templates before running because saves modify them. Keep [third-party notices](THIRD_PARTY.md) accurate.
