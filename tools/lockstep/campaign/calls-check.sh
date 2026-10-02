#!/bin/bash
# Checks each saved call in DIR (CAPTURE, calls.rs) again apart from its run and says whether it
# ends as the run said in its name: mutated-ok/call with no mismatch, *mismatch with one. DONE
# is the done list the run had (it decides which callees are checked too).
#
#   DONE=local/lockstep/done-f5.txt bash tools/lockstep/campaign/calls-check.sh local/calls/fid
cd "$(dirname "$0")/../../.."
. tools/lockstep/campaign/env.sh
DIR=${1:?DIR}
BIN=${BIN:-target/alt/release/ssbm-run-cap$EXE}
DISC="$SSBM_DISC"
same=0 differ=0
for f in "$DIR"/*.call; do
    out=$(env ${DONE:+LOCKSTEP_DONE=$DONE} $BIN "$DISC" --call "$f" --port all --lockstep 2>&1)
    n=$(grep -o "[0-9]* functions mismatch" <<<"$out" | cut -d' ' -f1)
    mm=$(grep -c "calls mismatch" <<<"$out")
    case $(basename "$f") in
        *mismatch*) want=yes ;;
        *) want=no ;;
    esac
    got=$([[ ${n:-0} -gt 0 || $mm -gt 0 ]] && echo yes || echo no)
    if [[ $want == "$got" ]]; then
        same=$((same + 1))
    else
        differ=$((differ + 1))
        echo "differs: $(basename "$f") mismatched=$got"
        grep "calls mismatch" <<<"$out" | head -3
    fi
done
echo "$same as in their run, $differ not"
