#!/bin/bash
# Coverage over every campaign's results, as report.sh, with each results directory counting
# only for the ports that haven't changed since the commit its binary was built from:
# binaries.txt lists them as DIR=COMMIT, optionally followed by lists, comma-separated, of more
# functions whose results there don't count, such as those a lockstep fix since made check
# differently.
# Writes report.csv, done.txt, known.bin (the verified instructions, for LOCKSTEP_KNOWN) and
# needed.txt (the blocks each function still needs, for LOCKSTEP_NEEDED) beside this script, or
# in OUT; BINARIES names another list of directories, and AT a commit to judge staleness at,
# for the coverage as it stood then. BAR (default 0.9) sets the share of its
# blocks a function needs to count as verified, 1 for full block coverage.
cd "$(dirname "$0")/../../.."
. tools/lockstep/campaign/env.sh
OUT=${OUT:-$L}
mkdir -p $OUT
bins=(); csvs=(); stale=()
while read -r pair extra; do
    [[ -z $pair || $pair == \#* ]] && continue
    dir=${pair%%=*}; commit=${pair#*=}
    bins+=("$L/$dir/*.bin"); csvs+=("$L/$dir/*.csv")
    changed=$L/changed-since-$commit${AT:+-to-$AT}.txt
    $PY tools/lockstep/changed.py $commit $AT > $changed
    stale+=(--stale "$changed=$L/$dir/*")
    for list in ${extra//,/ }; do stale+=(--stale "$L/$list=$L/$dir/*"); done
done < <(tr -d '\r' < ${BINARIES:-$L/binaries.txt})
$PY tools/lockstep/coverage.py $MELEE \
    --coverage "${bins[@]}" --ledger "${csvs[@]}" "${stale[@]}" \
    --dead tools/lockstep/dead-card.txt --stand-ins tools/lockstep/stand-ins-card.txt \
    --bar ${BAR:-0.9} --reviewed tools/lockstep/reviewed.txt --gaps tools/lockstep/gaps.txt --csv $OUT/report.csv --done $OUT/done.txt --known $OUT/known.bin --needed $OUT/needed.txt
