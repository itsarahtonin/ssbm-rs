#!/bin/bash
# Reviews one function's mutated mismatches by fuzzing it alone: reruns JOB (its line in
# jobs-ROUND*.txt) for FIELDS fields (6000) checking only FUNC and saving its real calls (spread
# out, and those that run new code) into a corpus, then checks every saved call again with
# MUTATE mutated checks (2000) and feedback, aimed by the latest targets, saving the mismatching
# ones for --call. Writes local/lockstep/review/FUNC-*; prints the fuzz pass's summary.
#
#   bash tools/lockstep/campaign/review.sh FUNC ROUND JOB [FIELDS] [MUTATE]
cd "$(dirname "$0")/../../.."
. tools/lockstep/campaign/env.sh
need_disc
FUNC=$1 ROUND=$2 JOB=$3 FIELDS=${4:-6000} MUTATE=${5:-2000}
BIN=${BIN:-target/t2/release/ssbm-run$EXE}
R=$L/review
mkdir -p $R
done=$R/done-not-$FUNC.txt
$PY - "$MELEE" "$FUNC" $done <<'EOF'
import re, sys
out = ["# every function but the one under review"]
for line in open(sys.argv[1] + "/config/GALE01/symbols.txt", encoding="utf-8"):
    m = re.match(r"(\S+) = \.text:0x([0-9A-F]+); // type:function", line)
    if m and m.group(1) != sys.argv[2]:
        out.append(f"0x{m.group(2).lower()} # {m.group(1)}")
open(sys.argv[3], "w").write("\n".join(out) + "\n")
EOF
line=$(cat $L/jobs-$ROUND.txt $L/jobs-$ROUND-*.txt 2>/dev/null | grep "^$JOB|" | head -1)
[[ -z $line ]] && { echo "no job $JOB in jobs-$ROUND*.txt"; exit 1; }
args=$(map_replays "${line#*|}")
envs=(); rest=()
for a in $args; do
    if [[ ${#rest[@]} -eq 0 && $a =~ ^[A-Z_]+= ]]; then envs+=("$a"); else rest+=("$a"); fi
done
for i in "${!rest[@]}"; do
    if [[ ${rest[$i]} == --card ]]; then
        cp $L/cards/template-saved.raw $R/$FUNC.raw
        rest[$((i + 1))]=$R/$FUNC.raw
    fi
done
out=$R/$FUNC
rm -f $out.corpus
env "${envs[@]}" LOCKSTEP_MUTATE=0 PROBES= LOCKSTEP_DONE=$done LOCKSTEP_CALLS=100000 \
    LOCKSTEP_BUDGET=1000000000 CAPTURE=$out.d CORPUS=$out.corpus CAPTURE_FUNCS=$FUNC \
    timeout 7200 $BIN "$SSBM_DISC" "${rest[@]}" --fields $FIELDS --port all --lockstep > $out-collect.txt 2>&1
[[ -f $out.corpus ]] || { echo "$FUNC: no calls saved ($(tail -1 $out-collect.txt))"; exit 1; }
targets=$(ls -t $L/targets-*.txt | head -1)
env LOCKSTEP_MUTATE=$MUTATE LOCKSTEP_SEED=${SEED:-1} LOCKSTEP_TARGETS=$targets LOCKSTEP_DONE=$done \
    CORPUS_FEEDBACK=${FEEDBACK:-50} CAPTURE=$out-mismatches \
    timeout 7200 $BIN "$SSBM_DISC" --corpus $out.corpus --port all --lockstep > $out-fuzz.txt 2>&1
grep "^checking\|^checked\|^lockstep:\|mismatch with their inputs" $out-fuzz.txt
ls $out-mismatches 2>/dev/null
