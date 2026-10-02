#!/bin/bash
# Screens replays before rounds use them: runs each for FIELDS fields (default 900) with every
# port and no lockstep, and keeps those whose Slippi device read frames, which a replay from
# another build of the game never does. Appends the playable ones to local/replays/playable.txt
# and the rest, with why, to local/replays/unplayable.txt; replays listed in either are skipped.
#
#   bash tools/replays/screen.sh REPLAY.slp...      (or none: every replay in the Slippi folder)
cd "$(dirname "$0")/../.."
. tools/lockstep/campaign/env.sh
R=local/replays
DISC="$SSBM_DISC"
BIN=${BIN:-target/t2/release/ssbm-run$EXE}
FIELDS=${FIELDS:-900}
touch $R/playable.txt $R/unplayable.txt
if [[ $# -eq 0 ]]; then
    set -- "${REPLAYS:?set REPLAYS in local/env.sh}"/*/*.slp
fi
mkdir -p $R/screen
for slp in "$@"; do
    slp=$(map_replays "$slp")
    name=$(basename "$slp" .slp)
    grep -q "$name" $R/playable.txt $R/unplayable.txt && continue
    log=$R/screen/$name.txt
    timeout 300 $BIN "$DISC" --replay "$slp" --fields $FIELDS --port all > $log 2>&1
    frames=$(grep -o "slippi: [0-9]* frames read" $log | grep -o "[0-9]*" | head -1)
    if [[ ${frames:-0} -gt 0 ]]; then
        echo "$slp" >> $R/playable.txt
        echo "playable $name ($frames frames read)"
    else
        why=$(grep -m1 "stopped:\|not found stage\|stopping:" $log | cut -c1-120)
        echo "$slp # ${why:-no Slippi frames read}" >> $R/unplayable.txt
        echo "unplayable $name: ${why:-no Slippi frames read}"
    fi
done
