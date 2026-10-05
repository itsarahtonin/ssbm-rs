# Working on ssbm-rs

Read [README.md](README.md), [architecture](docs/architecture.md), [roadmap](docs/roadmap.md), and [Contributing](CONTRIBUTING.md) before editing. For campaigns, also read [the workflow](tools/lockstep/campaign/README.md) and its dated [handoff](tools/lockstep/campaign/HANDOFF.md).

## Scope and rules

This repository preserves the faithful port and research tools. An idiomatic refactor with modern features belongs in a separate follow-on repository. Distinguish implementation, measured results, and planned verification.

- Never commit disc images, extracted assets, captures, replays, or cards. Keep them under ignored `local/` or outside the checkout.
- Choose a base deliberately and use a dedicated branch/worktree. Preserve work in a local commit before handing it back.
- Local implementation authorization does not authorize pushes, PRs, publication, settings, or history rewrites. Obtain explicit scoped authorization for those actions.
- Do not add assistant authorship or co-author trailers.
- Use conventional type-prefixed branches and concrete commit subjects with a capitalized first word. Include a short body.
- Match nearby conventions, keep new code comments concise, and do not hard-wrap Markdown prose.
- Fix generators rather than editing generated files. Format only changed files.

## Verification

Run appropriate checks and state what was tested. Disc tests skip without `SSBM_DISC`; core CI is not game-fidelity verification. Do not start expensive runs for documentation or audit tasks.

Paths come from ignored `local/env.sh`; [env.sh](tools/lockstep/campaign/env.sh) describes them. Generation Python needs libclang. Assembly is the source of truth for compiler behavior.

Run frozen binaries, keep results per binary, and account for stale ports and harness changes. Never edit an in-flight Bash script. Copy card templates before passing them to a runner because saves modify the file.

Give hidden window checks a field limit, leave the adapter alone unless testing it, and verify the process exited.

## Commands

- Build the runner: `cargo build --locked --release -p ssbm-run --target-dir target/t2`.
- Runtime tests: `cargo test --locked --release -p ssbm-ppc -p ssbm-rt -p ssbm-mem --target-dir target/t2`.
- Replay lockstep: `target/t2/release/ssbm-run DISC --replay FILE.slp --fields N --port all --lockstep` (use `.exe` on Windows).
- Regenerate ports: `$PY tools/c2rs/c2rs.py $MELEE local/typegen/types.json crates/game/src`, then `rustfmt --edition 2024` on changed output only.
- For compiler experiments, use `$MELEE/build/compilers/GC/1.2.5n/mwcceppc.exe` with the unit's flags from `$MELEE/build.ninja`; Linux uses wibo.
