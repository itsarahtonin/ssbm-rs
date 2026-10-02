#!/bin/bash
# Studies one function's mismatches with changed inputs: reruns a job of any wave or round with only FUNC
# checked (everything else in DONE), MUTATE mutated checks per call and a deep trace of FUNC,
# for FIELDS fields. Writes local/lockstep/study/FUNC.txt and prints the mismatches.
#
#   bash local/lockstep/study.sh FUNC JOB [FIELDS] [MUTATE] [SEED]
cd "$(dirname "$0")/../../.."
. tools/lockstep/campaign/env.sh
FUNC=$1 JOB=$2 FIELDS=${3:-6000} MUTATE=${4:-60} SEED=${5:-21}
mkdir -p $L/study
DONE=$L/study/done-not-$FUNC.txt
$PY - "$FUNC" "$DONE" <<'EOF'
import re, sys
keep, out = sys.argv[1], ["# everything but the function under study"]
for line in open("$MELEE/config/GALE01/symbols.txt", encoding="utf-8"):
    m = re.match(r"(\S+) = \.text:0x([0-9A-F]+); // type:function", line)
    if m and m.group(1) != keep:
        out.append(f"0x{m.group(2).lower()} # {m.group(1)}")
open(sys.argv[2], "w").write("\n".join(out) + "\n")
EOF
line=$(cat $L/jobs-wave*.txt $L/jobs-f*.txt 2>/dev/null | grep "^$JOB|" | head -1)
args=${line#*|}
envs=(); rest=()
for a in $args; do
    if [[ ${#rest[@]} -eq 0 && $a =~ ^[A-Z_]+= ]]; then envs+=("$a"); else rest+=("$a"); fi
done
# Fresh copies of any card the job uses.
for i in "${!rest[@]}"; do
    if [[ ${rest[$i]} == --card ]]; then
        src=${rest[$((i + 1))]}
        copy=$L/study/$FUNC.raw
        if [[ -f $src ]]; then cp "$src" "$copy"; else rm -f "$copy"; fi
        rest[$((i + 1))]=$copy
    fi
done
# --fields given last wins.
env "${envs[@]}" LOCKSTEP_DONE=$DONE LOCKSTEP_SEED=$SEED LOCKSTEP_MUTATE=$MUTATE \
    LOCKSTEP_TRACE_CALLS=1 LOCKSTEP_TRACE_DEEP=$FUNC PROBES= LOCKSTEP_TARGETS=${TARGETS:-$L/targets-wave6.txt} \
    timeout 5400 ${BIN:-./target/alt/release/ssbm-run-cov7$EXE} \
    "$SSBM_DISC" \
    "${rest[@]}" --fields "$FIELDS" --port all --lockstep > $L/study/$FUNC.txt 2>&1
grep -n "^lockstep:\|call trace, original\|with its inputs changed:$" $L/study/$FUNC.txt | head -20
