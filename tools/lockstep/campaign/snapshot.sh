#!/bin/bash
# Saves the coverage map's state as it stands now, mid-round included, as a new entry of its
# history (history.sh over every results directory binaries.txt lists so far), reports again,
# and writes the coverage map and the levers page (levers.py, gains counted from BASE, the last
# finished round) for republishing. LABEL names the entry, such as f6.1, f6.2, ... during round
# f6; the round's last entry, once it ends, is plain f6.
#
#   bash tools/lockstep/campaign/snapshot.sh LABEL BASE
cd "$(dirname "$0")/../../.."
. tools/lockstep/campaign/env.sh
LABEL=${1:?LABEL} BASE=${2:?BASE}
last=$(grep "^cov" $L/binaries.txt | tail -1 | cut -d= -f1)
bash $T/history.sh "$last" "$LABEL"
BAR=1 bash $T/report-all.sh > $L/report-$LABEL.txt 2>&1
$PY $T/treemap.py "$LABEL" $L/coverage-map.html
$PY $T/levers.py "$BASE" $L/levers.html
