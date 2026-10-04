#!/bin/bash
# Runs lockstep coverage jobs in parallel. Each line of JOBS_FILE is "name|ssbm-run arguments",
# where the arguments may start with VAR=VALUE settings for the run's environment; each job
# writes its coverage bitmap, ledger and log to $OUT/<name>.{bin,csv,txt} (OUT defaults to
# local/lockstep/cov) and is skipped when its log already has a result. BIN picks the binary,
# JOBS the parallelism, BUDGET and CALLS the checks per port, DONE a list of ports to leave
# unchecked, EXTRA more environment for every job, TIMEOUT the seconds a job may run (3 hours).
cd "$(dirname "$0")/../../.."
. tools/lockstep/campaign/env.sh
need_disc
export DISC="$SSBM_DISC"
export BIN="${BIN:-./target/alt/release/ssbm-run-cov$EXE}"
export BUDGET="${BUDGET:-20000000}" CALLS="${CALLS:-5000}" DONE="${DONE:-}" OUT="${OUT:-local/lockstep/cov}"
export EXTRA="${EXTRA:-}"
mkdir -p "$OUT"
grep -v '^#' "${1:?JOBS_FILE}" | tr -d '\r' | xargs -P "${JOBS:-8}" -d '\n' -I{} bash -c '
    line="{}"; name="${line%%|*}"; args="${line#*|}"
    # Replays named under another machine'"'"'s folder (env.sh).
    args=$(map_replays "$args")
    out=$OUT/$name
    grep -q "fields, .* M instructions" $out.txt 2>/dev/null && exit 0
    rm -f $out.bin $out.csv
    envs=(); rest=()
    for a in $args; do
        if [[ ${#rest[@]} -eq 0 && $a =~ ^[A-Z_][A-Z0-9_]*= ]]; then envs+=("$a"); else rest+=("$a"); fi
    done
    env LOCKSTEP_BUDGET=$BUDGET LOCKSTEP_CALLS=$CALLS ${DONE:+LOCKSTEP_DONE=$DONE} \
      LOCKSTEP_COVERAGE=$out.bin LOCKSTEP_LEDGER=$out.csv $EXTRA "${envs[@]}" \
      timeout ${TIMEOUT:-10800} $BIN "$DISC" "${rest[@]}" --port all --lockstep > $out.txt 2>&1
    echo "$? $name: $(grep "^lockstep:" $out.txt)"'
