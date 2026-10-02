#!/bin/bash
# Runs every smoke-set replay through ssbm-run in parallel; one result file per replay.
# known/NAME.txt lists a replay's accepted divergences; known-new/NAME.txt gets this run's.
# BIN picks the binary, ARGS adds options (such as --port all), OUT names the results folder.
cd "$(dirname "$0")/../.."
. tools/lockstep/campaign/env.sh
export DISC="$SSBM_DISC"
export BIN="${BIN:-./target/release/ssbm-run}" ARGS="${ARGS:-}" OUT="${OUT:-results}"
mkdir -p local/replays/$OUT local/replays/known local/replays/known-new
grep -v '^#' local/replays/smoke-set.txt | tr -d '\r' | tr '\\' '/' | xargs -P 8 -I{} bash -c '
  f=$(map_replays "{}"); name=$(basename "$f" .slp)
  if $BIN "$DISC" --replay "$f" --fields 100000 $ARGS \
      --known local/replays/known/$name.txt --write-known local/replays/known-new/$name.txt \
      > local/replays/$OUT/$name.txt 2>&1; then status=PASS; else status=FAIL; fi
  echo "$status $name: $(grep "replay check" local/replays/$OUT/$name.txt)"'
