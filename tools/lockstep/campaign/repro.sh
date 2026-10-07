#!/bin/bash
# Reruns one job of a round exactly as the round ran it (its line in jobs-ROUND.txt, the round's
# done list, probes, targets and settings), with a deep trace of FUNC, so a mismatch the round
# logged comes back with its call and write traces. BIN defaults to the line-table build, run in
# place so port faults name their ports. Writes local/lockstep/repro/ROUND-JOB-FUNC.txt.
#
#   bash tools/lockstep/campaign/repro.sh ROUND JOB FUNC [FIELDS]
#
# DIRECTED=1 reruns a round run with DIRECTED=1 (focus.sh, corpus.sh) as it ran: every other
# mutated check directed, which changes which inputs each mutated check gets.
cd "$(dirname "$0")/../../.."
. tools/lockstep/campaign/env.sh
ROUND=$1 JOB=$2 FUNC=$3 FIELDS=${4:-}
BIN=${BIN:-target/dbg/release/ssbm-run$EXE}
mkdir -p $L/repro
# Side campaigns of the round keep their jobs in jobs-ROUND-*.txt.
line=$(cat $L/jobs-$ROUND.txt $L/jobs-$ROUND-*.txt 2>/dev/null | grep "^$JOB|" | head -1)
[[ -z $line ]] && { echo "no job $JOB in jobs-$ROUND*.txt"; exit 1; }
args=${line#*|}
envs=(); rest=()
for a in $args; do
    if [[ ${#rest[@]} -eq 0 && $a =~ ^[A-Z_][A-Z0-9_]*= ]]; then envs+=("$a"); else rest+=("$a"); fi
done
# Fresh copies of any card the job uses, as the round started it: focus.sh's mode, event and
# removal cards from the saved template and its fault cards from their fault, since the run
# itself has since written to the job's card and would take another path.
for i in "${!rest[@]}"; do
    if [[ ${rest[$i]} == --card ]]; then
        src=${rest[$((i + 1))]}
        copy=$L/repro/$ROUND-$JOB.raw
        card=$(basename "$src" .raw)
        if [[ $card =~ ^$ROUND(w[0-9]|pm)(mode|event|remove)[0-9a-f]*$ ]]; then
            src=$L/cards/template-saved.raw
        elif [[ $card =~ ^$ROUND(w[0-9]|pm)cf([a-z0-9]*[a-z])[0-9]*$ ]]; then
            src=$L/cards/faults/${BASH_REMATCH[2]}.raw
        elif [[ $card =~ ^$ROUND(w[0-9]|pm)(card|jp|dvd|unlock) ]]; then
            # focus.sh deletes these before a round: the job started without a card file.
            src=
        fi
        if [[ -f $src ]]; then cp "$src" "$copy"; else rm -f "$copy"; fi
        rest[$((i + 1))]=$copy
    fi
done
[[ -n $FIELDS ]] && rest+=(--fields "$FIELDS")
out=$L/repro/$ROUND-$JOB-$FUNC.txt
# What else the round gave its jobs, which decides which checks run and so the mutations.
more=()
[[ -f $L/needed-$ROUND.txt ]] && more+=(LOCKSTEP_NEEDED=$L/needed-$ROUND.txt)
[[ -f $L/known-$ROUND.bin ]] && more+=(LOCKSTEP_KNOWN=$L/known-$ROUND.bin)
[[ -n $DIRECTED ]] && more+=(LOCKSTEP_DIRECTED=1)
env "${more[@]}" LOCKSTEP_BUDGET=5000000 LOCKSTEP_CALLS=3000 LOCKSTEP_DONE=$L/done-$ROUND.txt \
    PROBES=$L/probes-$ROUND.txt LOCKSTEP_TARGETS=$L/targets-$ROUND.txt LOCKSTEP_TRACE_CALLS=1 \
    "${envs[@]}" LOCKSTEP_TRACE_DEEP=$FUNC \
    timeout 5400 $BIN "$SSBM_DISC" \
    "${rest[@]}" --port all --lockstep > $out 2>&1
grep -n "^lockstep:\|$FUNC call [0-9]* had its inputs\|$FUNC writes, original\|port fault" $out | head -12
