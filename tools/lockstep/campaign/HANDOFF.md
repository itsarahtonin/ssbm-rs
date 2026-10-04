# Handoff: Phase 3 gate (milestone 3b), 2026-10-04

Where the campaign stood when the cloud session stopped, and what to do next. The gate: every countable block verified by lockstep or explained in `tools/lockstep/gaps.txt`, with no unexplained mismatches.

## Numbers

At snapshot f8.8 (the last entry on the coverage map, https://claude.ai/artifact/T1No6obYSvv81w5vVLpLkb; full report `local/lockstep/report-f8.8.txt`): 152,079 of 163,387 countable blocks verified (93.1%), 5,093 explained by the ledger, about 6,200 neither. Mismatches: 0 unmutated; 0 to review after the review below; 10 reviewed as coming from the inputs.

`fn_80196FFC`, the one mutated mismatch f8.8 left to review, is now in `tools/lockstep/reviewed.txt`: it needs a mutated gobj user data (a player index of 4 or more) and then reads past a stack copy, as the reason there says.

## Campaigns stopped mid-way

All were stopped on purpose, and restarted on 2026-10-04 on Sarah's desktop (14 jobs at a time between them, cov-recipes-more 2 more); `campaign.sh` reruns a job with no result line, so restarting each with the same command finishes them. Each runs a frozen binary copy in `target/alt/release` (rebuild it from the commit in `local/lockstep/binaries.txt` on a new VM, then copy it to that name).

| Results dir | Jobs file | Done | Binary (commit) |
|---|---|---|---|
| cov-f8 | jobs-f8.txt (focus.sh round f8) | 302/327 | ssbm-run-f8 (6be2b46) |
| cov-levers | jobs-levers.txt | 21/74 | ssbm-run-lv (b8f3c6a) |
| cov-recipes-menus | jobs-recipes-menus.txt | 12/99 | ssbm-run-lv (b8f3c6a) |
| cov-recipes-matches | jobs-recipes-matches.txt | 0/74 | ssbm-run-rx (70b4c01) |
| cov-recipes-more | jobs-recipes-more.txt | 0/51 | ssbm-run-rq (41497dd) |

Command shape (from the repository root):

```bash
L=local/lockstep; BIN=target/alt/release/ssbm-run-lv OUT=$L/cov-levers DONE=$L/done-f8.txt EXTRA="LOCKSTEP_NEEDED=$L/needed-f8.txt" JOBS=2 TIMEOUT=10800 nohup bash tools/lockstep/campaign/campaign.sh $L/jobs-levers.txt > $L/campaign-levers.log 2>&1 &
```

Round f8 resumes with `RESUME=1 NAME=f8 ... bash tools/lockstep/campaign/focus.sh` (its other settings are in `local/lockstep/levers.tsv` and the round's log). The jobs files and generators: `mkjobs-levers.py`, `mkjobs-recipes-menus.py`, `mkjobs-recipes-matches.py`, `mkjobs-recipes-more.py` regenerate them (each copies its cards). The bundle leaves the cards out: copy `template-saved.raw` and `faults/` into `local/lockstep/cards` (from the game data bundle) and rerun the generators to a scratch file, which recreates the cards; keep the jobs files as they are (jobs-recipes-menus.txt's seeds predate its dropped job). focus.sh makes f8's own cards on resuming, and copies BIN onto target/alt/release/ssbm-run-f8, so give it a copy from elsewhere.

`repro.sh ROUND` reads a round's done list, probes, targets and needed list (`done-ROUND.txt`, ...), which the side campaigns don't have: they ran with f8's done and needed lists, no probes or targets, BUDGET 20000000 and CALLS 5000, so rerun one of their jobs by hand with those settings.

Finished this session, all with no mismatch: cov-scripted, cov-slotb, cov-menus, cov-cardlayer, cov-scripted3 (album, slot B, menus, card layer, trophy debug procs, follow-ups).

## What worked

1. Static analysis by agents, one cluster of units at a time, writing ledger candidates (`FUNCTION+0xOFF KIND # evidence`, `UNIT:FUNCTION` for shared names) and run recipes. Before a batch goes in: every line a block start of the needed list, no duplicate, unique names (`tools/lockstep/candidates.py` checks these against a coverage bitmap and gaps.txt), and its strongest claims checked in the asm by hand. About 3,300 lines went in this session. Lines resting on weaker arguments stayed out: a joint's first matrix set right after it was loaded, a hanging mutated check, anything the analysis itself called inferred.
2. Runner levers that build the state a branch needs, then scripted jobs with `LOCKSTEP_UNLIMITED` on the target functions (per-function check caps had hidden slow screens and stage hazards). Levers added this session: `MONKEY_HOLD` (scripted holds on any port with sticks; holds on different ports overlap, a later one on the same port replaces an earlier one), `LOCKSTEP_UNLIMITED` (names or hex addresses), `NAMES`, `--card-b`/`CARD_REMOVE_B` (slot B), `CARD_REMOVE_STEP` (pulls between card layer steps), `cards.py --snaps/--saves` (crafted cards), `SAVE_POKE`, `PAD_UNPLUG`, `MATCH_CPU_KIND`, `MATCH_STAMINA`, `MATCH_VS`/`MATCH_TEAMS`, `MATCH_PLAYERS`, `EVENT_FIGHTER`, `CLASSIC_STAGE`/`ADVENTURE_STAGE`, `RECORDS_1P`, `TOU_ENTRANTS`, `MATCH_LOG`. Comments where each is read (tools/run/src) document them.
3. `coverage.py` now lists ledger lines whose blocks runs verified ("gap ledger lines whose blocks runs verified"): remove any it lists, since the ledger claimed no run could.

## Next steps

1. The campaigns above run; snapshot every few hours (`snapshot.sh f8.N f7`) and republish both pages.
2. Recipes not yet turned into jobs (`recipes.md` and `local/lockstep/notes-gr3/`, once the four recipe generators' jobs are subtracted): gmMainLib's D508/D5DC/D640 probes after SAVE_POKE, Kirby-team and KO-burst sounds and an SFX bank load at a scene exit (lbaudio_ax), the hammer and third-player grabs beyond grabthird (ftCo_Damage), challengers near unlock thresholds (gm_16F1) and trophy unlock thresholds (gm_1736), the staff roll's name shooting (gmstaffroll), the stage select's locked icons (mnstagesel), Adventure's trophy stage (grfigureget) and the Mushroom Kingdom Luigi arms of fn_8017E8A4. Runner changes still wanted: a human-controlled Master/Crazy Hand (they read ports 3 and 4; `choose()` forces bosses to CPU), forcing item kinds, `NAMES` with raw bytes, and `DVD_COVER` with a third toggle.
3. Analyse the long tail: ~4,400 open blocks sit in about a thousand units no analysis has covered (mostly under 30 blocks each). `local/lockstep/needed.txt` after a report lists them; group by unit as the analyses did.
4. When jobs stop gaining, the rest of the unverified blocks in analysed units are mostly odd state (null arms on loaded data, matrix-dirty tests, data-fixed random ranges): directed mutated checks (`LOCKSTEP_DIRECTED`, targets on those loads) or the ledger if the evidence is strong.

## Open questions

- Callbacks run from interrupt handlers: answered. Lockstep checks a callback an interrupt handler or an SDK stand-in calls when the interrupt is taken outside any check (the call goes through dispatch like any other); inside a check, the handler runs once as part of the original's interaction, and so do its callbacks, unchecked. Over all runs __CARDMountCallback has 3,707 checked calls and __CARDVerify 2,895. The job with `LOCKSTEP_UNLIMITED=__CARDMountCallback,__CARDVerify` saw none because its mounts came early, when every function still has checks left, so whatever was running when a mount's interrupts arrived (the card task loop's callers, the scene's code) was under check: to aim a job at such callbacks, leave the code running when their interrupts arrive unchecked (LOCKSTEP_DONE), or take them later in a run.
- The "first matrix set on a freshly loaded joint" argument (JObjInit sets JOBJ_MTX_DIRTY, jobj.c:1472) is kept out of the ledger (~25 stage on_init blocks, efAsync_Dispatch's 35); a directed mutation of the joint flags would verify them instead.
- `--mode` now also sets the scene machine's current mode (70b4c01). Results from before it with `--mode` saw the title as the current mode; character selects reached that way faulted on an unmapped read at 0x2 (game-side, logged in `local/lockstep/open-issues.md`).
- MSL printf's unused conversions: names that are formats read leftover registers (no arguments follow the name), so that job was dropped (its results are in `local/lockstep/quarantine/`). If the Name Entry keyboard can't type '%', those conversions are unreachable by any input the game accepts.
- `fn_8017D9C0`: the sides saw different `HSD_RandSeedPtr` in a mutated check (reviewed, harness question open).

## State outside the repository

`local/lockstep` (results, reports, ledgers of runs, cards, notes) lives on the VM and in the private release's `ssbm-state.tar.gz` (`tools/cloud/pack.sh`). The album jobs' 24-snapshot card is kept as `local/lockstep/templates/template-snaps.raw` (copy it to `$L/cards/snap/` before `mkjobs-album.py`). The static analyses' scratch files (per-unit annotated asm, notes, helper scripts) were in the session's scratch directory; `local/lockstep/notes-gr3/` keeps one batch's notes.
