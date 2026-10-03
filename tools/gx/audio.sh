#!/bin/bash
# Checks the AX microcode (ssbm-ax) against Dolphin's on the smoke-set replays and a menu run:
# runs each whole, every port, with AX_CHECK=1 (a binary built with the ax-check feature), which
# compares every command list's writes with Dolphin's AX code built from its source. Each run's
# log goes to local/gx/OUT/NAME.log, and a line per run to local/gx/OUT/summary.txt: command lists
# checked (in thousands, as the check counts them), those that differ, and whether the replay
# still synced with the DSP mixing. A run already checked is skipped.
#
#   cargo build --release -p ssbm-run --features ax-check --target-dir target/t2
#   OUT=audio bash tools/gx/audio.sh
cd "$(dirname "$0")/../.."
. tools/lockstep/campaign/env.sh
need_disc
export BIN=${BIN:-target/t2/release/ssbm-run$EXE} OUT=local/gx/${OUT:-audio} SSBM_DISC L
mkdir -p "$OUT"
{
    grep -v '^#' local/replays/smoke-set.txt | tr -d '\r' | while read -r f; do echo "$(map_replays "$f")"; done
    echo menu
} | xargs -P "${JOBS:-6}" -d '\n' -I{} bash -c '
    f="{}"
    name=$(basename "$f" .slp)
    log=$OUT/$name.log
    grep -q "fields, .* M instructions" "$log" 2>/dev/null && exit 0
    if [[ $f == menu ]]; then
        cp "$L/cards/template-saved.raw" "$OUT/menu-card.raw"
        input=(--monkey 101 --mode 01 --card "$OUT/menu-card.raw" --fields 7200)
    else
        input=(--replay "$f" --fields 100000)
    fi
    AX_CHECK=1 timeout 7200 $BIN "$SSBM_DISC" "${input[@]}" --port all > "$log" 2>&1
    echo "checked $name"'
: > "$OUT/summary.txt"
for log in "$OUT"/*.log; do
    name=$(basename "$log" .log)
    last=$(grep "AX check: .* command lists" "$log" | tail -1 | sed 's/AX check: //')
    first=$(grep -m1 "AX check: list" "$log" | sed 's/AX check: //')
    sync=$(grep -m1 "replay check:" "$log" | sed 's/.*replay check: //')
    echo "$name | ${last:-no lists} | ${sync:-no replay} | ${first:-}" >> "$OUT/summary.txt"
done
cat "$OUT/summary.txt"
