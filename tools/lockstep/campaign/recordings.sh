#!/bin/bash
# Lockstep checks of played sessions: takes the controller recordings (--record, which the
# player makes when its folder has a recordings folder) from RECORDINGS into $L/recordings,
# each with the card its session started on, adds a job playing each one to jobs-rec.txt and
# runs those not yet done on BIN (built from HEAD) into cov-rec, with LOCKSTEP_KNOWN from the
# latest report so each job tells what it verified beyond earlier results. Then reports again.
#
#   RECORDINGS=$APPDATA/ssbm-rs/recordings bash tools/lockstep/campaign/recordings.sh
#
# A recording written to in the last 5 minutes is left for later, as its session may still be
# going. A job plays the whole recording; it is kept as a job of round rec, so
# corpus.sh collect rec JOB FIELDS saves its calls for fuzzing.
cd "$(dirname "$0")/../../.."
. tools/lockstep/campaign/env.sh
need_disc
SRC=${RECORDINGS:-${APPDATA:?RECORDINGS}/ssbm-rs/recordings}
COMMIT=$(git rev-parse --short HEAD)
mkdir -p $L/recordings
touch $L/jobs-rec.txt
for f in $(find "$SRC" -maxdepth 1 -name '*.inputs' -mmin +5 | sort); do
    stamp=$(basename "$f" .inputs)
    job=rec${stamp//[-_]/}
    grep -q "^$job|" $L/jobs-rec.txt && continue
    cp "$f" $L/recordings/
    [[ -f ${f%.inputs}.card ]] && cp "${f%.inputs}.card" $L/recordings/
    echo "$job|--inputs $L/recordings/$stamp.inputs" >> $L/jobs-rec.txt
    echo "new recording $stamp as $job"
done
grep -q "^cov-rec=" $L/binaries.txt || echo "cov-rec=$COMMIT" >> $L/binaries.txt
BINCOPY=target/alt/release/ssbm-run-rec$EXE
mkdir -p target/alt/release
cp "${BIN:-target/t2/release/ssbm-run$EXE}" $BINCOPY
bash $T/report-all.sh > $L/report-rec-start.txt 2>&1
cp $L/known.bin $L/known-rec.bin
cp $L/needed.txt $L/needed-rec.txt
BIN=$BINCOPY EXTRA="LOCKSTEP_KNOWN=$L/known-rec.bin LOCKSTEP_TRACE_CALLS=1" DONE=$L/done.txt \
    OUT=$L/cov-rec BUDGET=5000000 CALLS=3000 JOBS=${JOBS:-4} TIMEOUT=${TIMEOUT:-21600} \
    bash $T/campaign.sh $L/jobs-rec.txt
bash $T/report-all.sh > $L/report-rec.txt 2>&1
