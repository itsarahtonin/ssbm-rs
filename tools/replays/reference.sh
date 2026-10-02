#!/bin/bash
# Runs every replay of the index on the original code, as the reference for the ports: each
# replay's divergences from its recording go to known-orig/NAME.txt, its log to results-orig.
# BIN picks the binary, JOBS the parallelism. Replays already done are skipped.
cd "$(dirname "$0")/../.."
. tools/lockstep/campaign/env.sh
export DISC="$SSBM_DISC"
export BIN="${BIN:-./target/alt/release/ssbm-run-orig$EXE}"
mkdir -p local/replays/known-orig local/replays/results-orig
tail -n +2 local/replays/index.csv | cut -d, -f1 | tr -d '\r' | tr '\\' '/' | \
  xargs -P "${JOBS:-8}" -I{} bash -c '
    f=$(map_replays "{}"); name=$(basename "$f" .slp)
    [ -f local/replays/results-orig/$name.txt ] && grep -q "replay check" local/replays/results-orig/$name.txt && exit 0
    timeout 7200 $BIN "$DISC" --replay "$f" --fields 100000 \
      --write-known local/replays/known-orig/$name.txt > local/replays/results-orig/$name.txt 2>&1
    echo "$? $name: $(grep "replay check" local/replays/results-orig/$name.txt)"'
