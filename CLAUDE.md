# ssbm-rs

Super Smash Bros. Melee (NTSC 1.02, GALE01 rev 2) ported to Rust, function by function from the melee decomp, and checked against the original game in lockstep. The plan, with its phases and milestones, is the Claude Docs artifact https://claude.ai/code/artifact/341d5fed-6bcb-4070-89e3-c0e1276f0816; Phase 3's progress is on the coverage map https://claude.ai/artifact/T1No6obYSvv81w5vVLpLkb. The current work is the Phase 3 gate (milestone 3b): see `tools/lockstep/campaign/README.md`.

## Rules

- Never commit game data: disc images, DOL/DAT or other files extracted from the disc, captured memory (saved calls, corpora), Slippi replays, memory card images. They live in `local/` (ignored) or outside the repository; the cloud gets them from the private release (`tools/cloud/`).
- Commits: a conventional type (`feat:`, `fix:`, ...), the subject's first word capitalized, a body that says what changed first and why after it. Only scopes the history already uses. No AI attribution: no Co-Authored-By trailers, no generated-by footers.
- Branches take a type prefix (`feat/`, `fix/`, ...), never a person's or agent's name.
- Commit and push to the working branch as work lands (cloud VMs can be reclaimed). Ask Sarah before opening pull requests, publishing releases or changing repository settings.
- Don't hard-wrap Markdown.

## Commands

- Build the runner: `cargo build --release -p ssbm-run --target-dir target/t2`
- Tests: `cargo test --release -p ssbm-ppc -p ssbm-rt -p ssbm-mem --target-dir target/t2`
- Run a replay under lockstep: `target/t2/release/ssbm-run DISC --replay FILE.slp --fields N --port all --lockstep`
- Regenerate the ports from the decomp (about 7 minutes): `$PY tools/c2rs/c2rs.py $MELEE local/typegen/types.json crates/game/src`, then `rustfmt --edition 2024` on the files it changed (only those: the rest of the repository isn't rustfmt-clean). `PY` must have the `clang` module (libclang).
- MWCC (`$MELEE/build/compilers/GC/1.2.5n/mwcceppc.exe`, with the unit's flags from `$MELEE/build.ninja`) settles questions about the original's code generation; on Linux it runs under wibo.

Paths: `tools/lockstep/campaign/env.sh` reads `local/env.sh` for `SSBM_DISC`, `MELEE`, `PY` and the replay folders; `bash tools/cloud/setup.sh` writes it on a cloud VM.
