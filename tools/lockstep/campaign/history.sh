#!/bin/bash
# Saves the coverage report as it stood after a round, for the coverage map's history:
# local/lockstep/history/N-LABEL.csv (spaces in LABEL as underscores), a report.csv over the results directories binaries.txt
# lists up to and including DIR, with staleness judged at that directory's commit.
#
#   bash tools/lockstep/campaign/history.sh DIR LABEL      (e.g. cov-f3 f3)
#   bash tools/lockstep/campaign/history.sh --all          (every round so far, from wave 3 on)
cd "$(dirname "$0")/../../.."
. tools/lockstep/campaign/env.sh
mkdir -p $L/history
one() {
    local dir=$1 label=${2// /_} n
    local list=$L/history/binaries-$label.txt
    awk -v d="$dir" '/^cov/ {print} $0 ~ "^"d"=" {exit}' $L/binaries.txt > $list
    local at; at=$(grep "^$dir=" $L/binaries.txt | sed 's/^[^=]*=\([0-9a-f]*\).*/\1/')
    n=$(grep -c "^cov" $list)
    OUT=$L/history/tmp-$label BINARIES=$list AT=$at BAR=1 bash $T/report-all.sh > $L/history/report-$label.txt 2>&1
    cp $L/history/tmp-$label/report.csv "$L/history/$(printf %02d $n)-$label.csv"
    rm -rf $L/history/tmp-$label
    echo "$label: $(grep 'blocks verified' $L/history/report-$label.txt)"
}
if [[ $1 == --all ]]; then
    one cov-w3p "wave 3"; one cov-w4 "wave 4"; one cov-w5 "wave 5"; one cov-w6 "wave 6"
    one cov-f1 f1; one cov-f2 f2; one cov-f3 f3
else
    one "$1" "$2"
fi
