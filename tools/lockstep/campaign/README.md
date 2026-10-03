# Lockstep campaigns

Phase 3's evidence comes from lockstep coverage campaigns: many runs of `ssbm-run --lockstep`, each checking every port against its original under real and mutated inputs, merged into one report. The Phase 3 gate (milestone 3b) is every countable block verified by lockstep or explained in the reviewed gap ledger (`tools/lockstep/gaps.txt`), with no unexplained mismatches, mutated ones included.

## Setup

The scripts read the machine's paths from `local/env.sh` (ignored; see `env.sh` for the variables). On a cloud VM, `bash tools/cloud/setup.sh` downloads the private release (disc, replays, card templates, the decomp's disc-made files, campaign state) and writes it. Never commit anything from `local/`: it holds game data and captured memory.

Campaign state lives in `local/lockstep` (`L`); the scripts in `tools/lockstep/campaign` (`T`). Run them from the repository root.

## Rounds

A focused round runs the jobs most likely to verify what is still open:

```bash
NAME=f6 BAR=1 SEED=800 MUTATE=8 STALE=1800 BIN=target/t2/release/ssbm-run JOBS=15 bash tools/lockstep/campaign/focus.sh
```

It reports what all results so far leave open (`report-all.sh`), writes probes, mutation targets (`tools/lockstep/targets.py`), the known and needed lists, picks jobs (`mkjobs-focus.py`) and appends `$L/jobs-extra-NAME.txt`, copies the binary to `target/alt/release/ssbm-run-NAME`, runs `campaign.sh` into `$L/cov-NAME`, and reports again. `RESUME=1` continues a round whose inputs are written. Jobs stop once `STALE` fields pass with nothing new beyond `LOCKSTEP_KNOWN`; checks of a function end once its needed blocks are verified.

Results live in one directory per binary; `$L/binaries.txt` maps each to the commit its ports came from, optionally with lists of functions whose results there don't count (harness-fixed lists, for lockstep fixes that change how functions check). `report-all.sh` counts old results only for ports unchanged since their commit.

After each round, save it for the coverage map and republish the map (Sarah follows it at https://claude.ai/artifact/T1No6obYSvv81w5vVLpLkb):

```bash
bash tools/lockstep/campaign/history.sh cov-f6 f6
```

```bash
python tools/lockstep/campaign/treemap.py f6 local/lockstep/coverage-map.html
```

During a round, `bash tools/lockstep/campaign/snapshot.sh f6.1 f5` (then f6.2, ...; f5 the last finished round) saves the map's state so far as a new history entry and writes the coverage map and the levers page, to republish every few hours; the round's own entry comes at its end, as above. The levers page (https://claude.ai/artifact/SANQTUxB7JV4kjD7kVP5ER, from `levers.py`) compares what each lever yields per cost, from the runs listed in `local/lockstep/levers.tsv` and the findings in `levers-notes.tsv`.

## Job options

A job line is `name|[VAR=V ...] ssbm-run args`; its own variables override the round's. Useful ones: `DBLEVEL=N` (debug level 1-4; levels 3-4 verify several times what a retail run does), `MENU=KIND,SELECTION` with `--mode 01`, `--mode HEX`, `UNLOCK_ALL=1`, `RECORDS=SEED`, `MATCH_STAGES`/`MATCH_FIGHTERS`/`MATCH_TIME` with `--matches SEED --mode e`, `EVENT=N`, `GAME_LANGUAGE=jp`, `CARD_REMOVE`, `DVD_*`, `--card FILE`, `--monkey SEED`, `LOCKSTEP_SEED`, `LOCKSTEP_MUTATE`. Seeds are decimal. Card files a job names are copied from `$L/cards/template-saved.raw` by `focus.sh`; never pass a template itself to `--card`, since the SDK writes the card back. Stage kinds 33-90 (1P stages) load in debug VS too.

## Mismatches

Mutated checks run against null hardware and are dropped when they fault, run away, or read what only the original keeps; `LOCKSTEP_DROP_LOG=1` prints why. A mismatch under mutated inputs is "to review": either a port bug, or something the inputs make impossible, which goes into `tools/lockstep/reviewed.txt` with the reason.

- `repro.sh ROUND JOB FUNC` reruns a round's job exactly, with a deep trace of FUNC (call traces and both sides' writes).
- Faster: `CAPTURE=DIR` saves each mismatching call, and `ssbm-run DISC --call FILE --port all --lockstep` checks it again in a fraction of a second. `CORPUS=FILE CAPTURE_FUNCS=@local/lockstep/needed-fN.txt` collects real calls for `--corpus FILE` (with `LOCKSTEP_MUTATE`, a fuzzer). `calls-check.sh DIR` confirms saved calls end as in their run. Mismatches from the corpus fuzzer were replay artifacts when last checked (see `local/lockstep/open-issues.md`).

## The gap ledger

`tools/lockstep/gaps.txt` explains blocks no run can reach, by kind (dead, dev, fail, hw, oracle) with evidence. `tools/lockstep/callers.py MELEE UNIT...` lists what refers into a set of units from outside, the usual evidence for code only certain entry points lead to. `tools/lockstep/frontier.py` sorts the unreached blocks next to verified ones by what keeps mutations out of them.

## Habits

- Don't edit a bash script while a run of it is in flight: bash reads scripts as it goes.
- Run binary copies, never the build output itself, so rebuilding doesn't disturb running jobs.
- Check a crash with an interpreter-only rerun (no `--port`) before suspecting a port: some modes entered directly crash the game itself.
- A cloud VM pauses when the session goes idle, and work running then is lost if the VM is reclaimed: keep the session working (reviewing, building tools, checking on the round) while a round runs.
