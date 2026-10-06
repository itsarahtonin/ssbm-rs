#!/bin/bash
# A focused round NAME (f1, f2, ...): reports what all results so far leave below the bar, then
# reruns the jobs that called those functions most with new seeds and MUTATE mutated checks
# per call, with probes and targets of what is still below it, on BIN (built from HEAD), into
# cov-NAME. Then reports again.
#
#   NAME=f1 SEED=300 MUTATE=8 BIN=target/t2/release/ssbm-run$EXE bash tools/lockstep/campaign/focus.sh
#
# BAR (default 0.9) is the share of its blocks a function needs, 1 for full block coverage. Each job
# stops once STALE fields (default 1800) pass with nothing verified beyond what earlier results
# hold (LOCKSTEP_KNOWN, from the opening report) and no other function mismatching, and checks
# of a function end once the blocks it still needs (LOCKSTEP_NEEDED) are verified. Jobs in
# jobs-extra-NAME.txt join the round's. RESUME=1 goes on with a round whose report, probes,
# targets and jobs are already written. DIRECTED=1 makes every other mutated check a directed one,
# setting exactly one target to the value its branch looks for (LOCKSTEP_DIRECTED).
cd "$(dirname "$0")/../../.."
. tools/lockstep/campaign/env.sh
need_disc
NAME=${NAME:?NAME} SEED=${SEED:-300} MUTATE=${MUTATE:-8}
export BAR=${BAR:-0.9}
COMMIT=$(git rev-parse --short HEAD)
if [[ -z $RESUME ]]; then
bash $T/report-all.sh > $L/report-$NAME-start.txt 2>&1
cp $L/done.txt $L/done-$NAME.txt
$PY tools/lockstep/probes.py $MELEE local/typegen/types.json $L/report.csv \
    $L/probes-$NAME.txt --dead tools/lockstep/dead-card.txt --bar $BAR
$PY tools/lockstep/targets.py $MELEE $L/report.csv $L/targets-$NAME.txt \
    --coverage "$L/cov*/*.bin" --bar $BAR
cp $L/known.bin $L/known-$NAME.bin
cp $L/needed.txt $L/needed-$NAME.txt
$PY $T/mkjobs-focus.py $NAME $SEED $MUTATE $L/jobs-$NAME.txt
[[ -f $L/jobs-extra-$NAME.txt ]] && grep -v '^#' $L/jobs-extra-$NAME.txt >> $L/jobs-$NAME.txt
fi
grep -q "^cov-$NAME=" $L/binaries.txt || echo "cov-$NAME=$COMMIT" >> $L/binaries.txt
BINCOPY=target/alt/release/ssbm-run-$NAME$EXE
mkdir -p target/alt/release
cp "${BIN:-target/t2/release/ssbm-run$EXE}" $BINCOPY
# Card files are named for the round, the wave tag of the job they come from, and their kind.
for f in $(grep -o "$L/cards/$NAME\(w[0-9]\|pm\)\(mode\|event\|remove\)[0-9a-f]*\.raw" $L/jobs-$NAME.txt); do
    cp $L/cards/template-saved.raw "$f"
done
for f in $(grep -o "$L/cards/${NAME}\(w[0-9]\|pm\)cf[a-z0-9]*\.raw" $L/jobs-$NAME.txt); do
    kind=$(basename "$f" .raw); kind=${kind#${NAME}}; kind=${kind#??cf}; cp $L/cards/faults/${kind%[0-9]}.raw "$f"
done
rm -f $L/cards/${NAME}??card*.raw $L/cards/${NAME}??jp*.raw $L/cards/${NAME}??dvd*.raw $L/cards/${NAME}??unlock*.raw
echo "round $NAME: $(grep -vc '^#' $L/jobs-$NAME.txt) jobs, $(wc -l < $L/done-$NAME.txt) done, binary from $COMMIT"
BIN=$BINCOPY EXTRA="PROBES=$L/probes-$NAME.txt LOCKSTEP_TARGETS=$L/targets-$NAME.txt LOCKSTEP_TRACE_CALLS=1 LOCKSTEP_KNOWN=$L/known-$NAME.bin LOCKSTEP_STALE=${STALE:-1800} LOCKSTEP_NEEDED=$L/needed-$NAME.txt${DIRECTED:+ LOCKSTEP_DIRECTED=1}" \
    DONE=$L/done-$NAME.txt OUT=$L/cov-$NAME BUDGET=5000000 CALLS=3000 JOBS=${JOBS:-14} TIMEOUT=5400 \
    bash $T/campaign.sh $L/jobs-$NAME.txt
bash $T/report-all.sh > $L/report-$NAME.txt 2>&1
