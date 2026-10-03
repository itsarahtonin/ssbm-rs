#!/bin/bash
# Runs every smoke-set replay twice, once on original code and once with every port, logging the
# GX stream's digests (GX_STREAM), then compares each pair (compare.py). Logs go to
# local/gx/OUT/NAME.{orig,ports}.{txt,log}; a pair already logged is skipped. BIN picks the
# binary, JOBS the parallelism.
#
#   OUT=smoke bash tools/gx/smoke.sh
cd "$(dirname "$0")/../.."
. tools/lockstep/campaign/env.sh
need_disc
export BIN=${BIN:-target/alt/release/ssbm-run-gx$EXE} OUT=local/gx/${OUT:-smoke}
mkdir -p "$OUT"
grep -v '^#' local/replays/smoke-set.txt | tr -d '\r' | while read -r f; do
    f=$(map_replays "$f")
    name=$(basename "$f" .slp)
    for side in orig ports; do echo "$side|$name|$f"; done
done | xargs -P "${JOBS:-6}" -d '\n' -I{} bash -c '
    IFS="|" read -r side name f <<<"{}"
    log=$OUT/$name.$side
    grep -q "fields, .* M instructions" $log.log 2>/dev/null && exit 0
    ports=(); [[ $side == ports ]] && ports=(--port all)
    GX_STREAM=$log.txt timeout 7200 $BIN "$SSBM_DISC" --replay "$f" --fields 100000 "${ports[@]}" > $log.log 2>&1
    echo "$side $name: $(tail -1 $log.log)"'
for o in "$OUT"/*.orig.txt; do
    name=$(basename "$o" .orig.txt)
    echo "$name: $($PY tools/gx/compare.py "$o" "$OUT/$name.ports.txt" | head -1)"
done
