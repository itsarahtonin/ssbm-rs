#!/bin/bash
# Corpus fuzzing between rounds: collects real calls of the functions a round still needs from
# one of its jobs, then checks them again many times with mutated inputs, apart from any run, for
# a fraction of a run's time. Results go to a results directory report-all.sh counts like a
# round's.
#
#   bash tools/lockstep/campaign/corpus.sh collect ROUND JOB [FIELDS]
#   bash tools/lockstep/campaign/corpus.sh fuzz ROUND NAME [MUTATE] [SEED] [REPEAT]
#
# collect reruns JOB (its line in jobs-ROUND*.txt) for FIELDS fields (3000) checking only the
# functions needed-ROUND.txt names, unmutated, and saves real calls of each (spread out, and
# those that run new code) into DIR/ROUND-JOB.corpus. fuzz checks every corpus of ROUND in DIR
# (or those CORPORA names) REPEAT times (1) with MUTATE mutated checks each (200), aimed by
# targets-ROUND.txt, into $L/cov-NAME (one results file per corpus, skipping corpora done
# there), and adds cov-NAME to binaries.txt at HEAD. DIR is CORPUS_DIR, $L/corpus by default.
cd "$(dirname "$0")/../../.."
. tools/lockstep/campaign/env.sh
need_disc
BIN=${BIN:-target/t2/release/ssbm-run$EXE}
DIR=${CORPUS_DIR:-$L/corpus}
mkdir -p $DIR
case $1 in
collect)
    ROUND=$2 JOB=$3 FIELDS=${4:-3000}
    line=$(cat $L/jobs-$ROUND.txt $L/jobs-$ROUND-*.txt 2>/dev/null | grep "^$JOB|" | head -1)
    [[ -z $line ]] && { echo "no job $JOB in jobs-$ROUND*.txt"; exit 1; }
    args=$(map_replays "${line#*|}")
    envs=(); rest=()
    for a in $args; do
        if [[ ${#rest[@]} -eq 0 && $a =~ ^[A-Z_]+= ]]; then envs+=("$a"); else rest+=("$a"); fi
    done
    for i in "${!rest[@]}"; do
        if [[ ${rest[$i]} == --card ]]; then
            copy=$DIR/$ROUND-$JOB.raw
            cp $L/cards/template-saved.raw "$copy"
            rest[$((i + 1))]=$copy
        fi
    done
    # Everything but the needed functions goes unchecked, so the run is quick.
    done=$DIR/done-not-needed-$ROUND.txt
    if [[ ! -f $done ]]; then
        $PY - "$MELEE" $L/needed-$ROUND.txt $done <<'EOF'
import re, sys
need = {l.split()[0].lower() for l in open(sys.argv[2]) if l.strip()}
out = ["# every function but those the needed list names"]
for line in open(sys.argv[1] + "/config/GALE01/symbols.txt", encoding="utf-8"):
    m = re.match(r"(\S+) = \.text:0x([0-9A-F]+); // type:function", line)
    if m and "0x" + m.group(2).lower() not in need:
        out.append(f"0x{m.group(2).lower()} # {m.group(1)}")
open(sys.argv[3], "w").write("\n".join(out) + "\n")
EOF
    fi
    out=$DIR/$ROUND-$JOB
    rm -f $out.corpus
    env "${envs[@]}" LOCKSTEP_MUTATE=0 PROBES= LOCKSTEP_DONE=$done \
        LOCKSTEP_CALLS=100000 LOCKSTEP_BUDGET=1000000000 \
        CAPTURE=$out.d CORPUS=$out.corpus CAPTURE_FUNCS=@$L/needed-$ROUND.txt \
        timeout 7200 $BIN "$SSBM_DISC" "${rest[@]}" --fields $FIELDS --port all --lockstep > $out.txt 2>&1
    echo "$JOB: $(tail -1 $out.txt); corpus $(du -h $out.corpus | cut -f1)"
    ;;
fuzz)
    ROUND=$2 NAME=$3 MUTATE=${4:-200} SEED=${5:-1} REPEAT=${6:-1}
    OUT=$L/cov-$NAME
    mkdir -p $OUT
    grep -q "^cov-$NAME=" $L/binaries.txt || echo "cov-$NAME=$(git rev-parse --short HEAD)" >> $L/binaries.txt
    for c in ${CORPORA:-$DIR/$ROUND-*.corpus}; do
        [[ -f $c ]] || { echo "no corpus $c"; continue; }
        n=$(basename $c .corpus)
        grep -q "^lockstep:" $OUT/$n.txt 2>/dev/null && continue
        env LOCKSTEP_MUTATE=$MUTATE LOCKSTEP_SEED=$SEED LOCKSTEP_TARGETS=$L/targets-$ROUND.txt \
            LOCKSTEP_NEEDED=$L/needed-$ROUND.txt LOCKSTEP_DONE=$L/done-$ROUND.txt \
            LOCKSTEP_COVERAGE=$OUT/$n.bin LOCKSTEP_LEDGER=$OUT/$n.csv \
            timeout 7200 $BIN "$SSBM_DISC" --corpus $c --repeat $REPEAT --port all --lockstep > $OUT/$n.txt 2>&1
        echo "$n: $(grep '^lockstep:' $OUT/$n.txt)"
    done
    ;;
*)
    sed -n '2,15p' "$0"; exit 1 ;;
esac
