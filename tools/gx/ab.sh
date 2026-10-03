#!/bin/bash
# Compares two runners on one workload, alternating them ROUNDS times so that drift in the
# machine's speed falls on both alike: milliseconds per field for each run, then each binary's
# best. The workload is a four-player match on Mute City (as bench.sh's), the game alone unless
# GX_RENDER=- is set.
#
#   bash tools/gx/ab.sh OLD.exe NEW.exe [ROUNDS] [FIELDS]
cd "$(dirname "$0")/../.."
. tools/lockstep/campaign/env.sh
need_disc
a=$1 b=$2 rounds=${3:-3} fields=${4:-1200}
best_a=999 best_b=999
for ((i = 1; i <= rounds; i++)); do
    for bin in "$a" "$b"; do
        t0=$(date +%s%N)
        MATCH_STAGES=10 "$bin" "$SSBM_DISC" --mode e --matches 1 --monkey 1 --fields "$fields" --port all > /dev/null 2>&1
        ms=$(echo "$(($(date +%s%N) - t0)) $fields" | awk '{printf "%.2f", $1 / 1e6 / $2}')
        echo "round $i  $(basename "$bin")  $ms ms/field"
        if [[ $bin == "$a" ]]; then
            best_a=$(echo "$ms $best_a" | awk '{print ($1 < $2) ? $1 : $2}')
        else
            best_b=$(echo "$ms $best_b" | awk '{print ($1 < $2) ? $1 : $2}')
        fi
    done
done
echo "best: $(basename "$a") $best_a, $(basename "$b") $best_b ms/field"
