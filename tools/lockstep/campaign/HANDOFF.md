# Handoff: Lockstep coverage, 2026-10-05

A dated checkpoint, not a live status: check local processes and reports before resuming anything. Current project scope is in the [roadmap](../../../docs/roadmap.md). The gate: every countable block verified by lockstep or explained in `tools/lockstep/gaps.txt`, with no unexplained mismatches.

## Numbers

The last report (October 5, after round f10): 152,971 of 163,387 countable blocks verified (93.6%), 5,124 explained by the ledger.

Mismatches: none with real inputs (the overnight one below came from a match the game can't set up, and `nighthands13`'s results are in `local/lockstep/quarantine/`); every mismatch from changed inputs is reviewed in `tools/lockstep/reviewed.txt` (12 functions; round f10's `ftCo_Fall_IASA_Inner` and `ftCo_800A75DC` were the last: a mutated air special lookup that calls a Mute City callback, and a floor-line walk sent into the hardware registers).

## Runtime data

Everything lives in this clone, ignored by Git: `local/` (env.sh, lockstep results and reports, cards, replays, captures, traces) and the frozen runners in `target/alt/release`. `local/lockstep/binaries.txt` names each results directory's commit in this repository's history; the predecessor repository's hashes are kept in `binaries-old-history.txt` (the trees of `crates/game/src` match pair by pair, so staleness carries over). The private data release stays in the predecessor repository (`tools/cloud/`). The album jobs' 24-snapshot card is `local/lockstep/templates/template-snaps.raw` (copy it to `$L/cards/snap/` before `mkjobs-album.py`).

The overnight wave's controller, queue, binary provenance and reports are in `C:/Users/malur/.codex/reports/ssbm-overnight-2026-10-05/`.

## Campaigns

Finished: cov-f8, cov-levers, cov-recipes-menus, cov-recipes-matches and cov-recipes-more (624 of 625 jobs with a result; `lvvsnostage` stopped after asking for an 11 GB allocation, undiagnosed), and cov-f10 (287 focused jobs on 3d9e1c3, the first round whose probes pass arguments past r10 on the stack; about four hours, 145 more blocks verified, its two mutated mismatches reviewed).

The overnight wave (`jobs-f9night.txt`, 275 focused jobs, and `jobs-night-targeted.txt`, 24 human-hands and 16 item jobs, on `ssbm-run-f9night`) ran 53 of 315 jobs before a real-call mismatch stopped its controller; 262 never ran. That mismatch, `OSResumeThread` in `nighthands13`, came from a match with a Crazy Hand and no Master Hand, which failed one of the game's own assertions and met its crash path; the original code alone fails the same match too. `nighthands08` and `nighthands11` (only hands) stopped on the hands' null nearest opponent (`ftBossLib_8015C208`), identically with the original alone, as did `rxhandsstamina` once its one-life Foxes were out. The runner now gives matches with the hands a line-up the game can have, so the hands jobs need regenerating before they run again; the rest of the queue can run as written.

Command shape for a side campaign (from the repository root):

```bash
L=local/lockstep; BIN=target/alt/release/ssbm-run-lv OUT=$L/cov-levers DONE=$L/done-f8.txt EXTRA="LOCKSTEP_NEEDED=$L/needed-f8.txt" JOBS=2 TIMEOUT=10800 nohup bash tools/lockstep/campaign/campaign.sh $L/jobs-levers.txt > $L/campaign-levers.log 2>&1 &
```

A focused round resumes with `RESUME=1 NAME=... bash tools/lockstep/campaign/focus.sh`; it copies BIN onto `target/alt/release/ssbm-run-NAME`, so give it a copy from elsewhere. The generators `mkjobs-levers.py`, `mkjobs-recipes-menus.py`, `mkjobs-recipes-matches.py` and `mkjobs-recipes-more.py` rewrite their jobs files and copy their cards from `local/lockstep/cards/template-saved.raw`.

`repro.sh ROUND` reads a round's done list, probes, targets and needed list (`done-ROUND.txt`, ...), which the side campaigns don't have: they ran with f8's done and needed lists, no probes or targets, BUDGET 20000000 and CALLS 5000, so rerun one of their jobs by hand with those settings (as the `Fighter_procMap` review did).

## What worked

1. Static analysis by agents, one cluster of units at a time, writing ledger candidates (`FUNCTION+0xOFF KIND # evidence`, `UNIT:FUNCTION` for shared names) and run recipes. Before a batch goes in: `tools/lockstep/candidates.py` checks each line is the start of a block the coverage leaves unverified, names a unique function and repeats nothing; then its strongest claims are checked in the assembly and the data by hand. Lines resting on weaker arguments stay out: a joint's first matrix set right after it was loaded, a hanging mutated check, anything the analysis itself called inferred.
2. Runner levers that build the state a branch needs, then scripted jobs with `LOCKSTEP_UNLIMITED` on the target functions. Comments where each is read (tools/run/src) document them; `MONKEY_HOLD` holds on different ports overlap, and a later hold on a port replaces an earlier one.
3. `coverage.py` lists ledger lines whose blocks runs verified ("gap ledger lines whose blocks runs verified"): remove any it lists, since the ledger claimed no run could.

## Next steps

1. Regenerate the human-hands jobs (`jobs-night-targeted.txt`) with the fixed runner and run them with its item jobs. Round f10 regenerated the focused jobs, so the rest of the f9night queue is superseded.
2. The long tail: about 2,200 open blocks in units no analysis covered, grouped in `local/lockstep/notes-tail/units-*.tsv`. Only the fighters group was analysed (`notes-tail/fighters.txt` has its recipes; 66 of its 68 ledger lines are in gaps.txt, the other two in `fighters-ledger-held.txt`); items, libraries, other melee code and gm/mn remain.
3. Recipes not yet turned into jobs: `recipes.md`, `local/lockstep/notes-gr3/` and the fighters notes, less what the generators cover. Runner changes still wanted: `NAMES` with raw bytes and `DVD_COVER` with a third toggle.
4. When jobs stop gaining, the rest of the unverified blocks in analysed units are mostly odd state (null arms on loaded data, matrix-dirty tests, data-fixed random ranges): directed mutated checks (`LOCKSTEP_DIRECTED`, targets on those loads) or the ledger if the evidence is strong.

## Open questions

- `nighthands13` diverged between its run with the ports (an assertion at field 37) and the original alone (a fault at field 2,951) from the same inputs. The match can't happen in the game, but the divergence shows some unchecked port or unwritten memory differs from the original there.
- `lvvsnostage`'s 11 GB allocation (`stage_mask=0` in the VS rules): the runner or the game.
- The "first matrix set on a freshly loaded joint" argument (JObjInit sets JOBJ_MTX_DIRTY, jobj.c:1472) is kept out of the ledger (~25 stage on_init blocks, efAsync_Dispatch's 35); a directed mutation of the joint flags would verify them instead.
- MSL printf's unused conversions: names that are formats read leftover registers, so that job was dropped (its results are in `local/lockstep/quarantine/`). If the Name Entry keyboard can't type '%', those conversions are unreachable by any input the game accepts.
- Lockstep checks a callback an interrupt handler calls only when the interrupt is taken outside any check; to aim a job at such callbacks, leave the code running when their interrupts arrive unchecked (LOCKSTEP_DONE), or take them later in a run.
