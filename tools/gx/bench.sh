#!/bin/bash
# Times what --window has to keep up with, in milliseconds per field: the game alone and the game
# with the renderer drawing offscreen (GX_RENDER=-, with GX_PROFILE's breakdown), on a four-player
# match on Mute City (the debug VS mode, --mode e, with --monkey playing) and on a smoke replay.
# Each run starts from boot, so the per-field figures include it. BIN is a runner built with the
# window feature; BENCH lists the runs to do (mute, replay; both by default).
#
#   BIN=target/w/release/ssbm-run.exe bash tools/gx/bench.sh
cd "$(dirname "$0")/../.."
. tools/lockstep/campaign/env.sh
need_disc
BIN=${BIN:-target/w/release/ssbm-run$EXE}
REPLAY=${REPLAY:-$(map_replays "$(grep -v '^#' local/replays/smoke-set.txt | tr -d '\r' | head -1)")}
out=$(mktemp -d)
cp "$REPLAY" "$out/replay.slp"
run() {
    local name=$1 fields=$2; shift 2
    for mode in game render; do
        local t0=$(date +%s%N)
        if [[ $mode == render ]]; then
            GX_RENDER=- GX_PROFILE=1 "$BIN" "$SSBM_DISC" "$@" --fields "$fields" --port all > "$out/$name-$mode.log" 2>&1
        else
            "$BIN" "$SSBM_DISC" "$@" --fields "$fields" --port all > "$out/$name-$mode.log" 2>&1
        fi
        local ms=$((($(date +%s%N) - t0) / 1000000))
        printf '%-7s %-7s %6.2f ms/field' "$name" "$mode" "$(echo "$ms $fields" | awk '{print $1 / $2}')"
        [[ $mode == render ]] && printf '   %s' "$(grep "render per frame" "$out/$name-$mode.log" | tail -1 | sed 's/render per frame: //')"
        echo
    done
}
for b in ${BENCH:-mute replay}; do
    case $b in
    mute) MATCH_STAGES=10 run mute 1800 --mode e --matches 1 --monkey 1 ;;
    replay) run replay 2400 --replay "$out/replay.slp" ;;
    esac
done
rm -rf "$out"
