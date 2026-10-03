#!/bin/bash
# Compares the renderer with Dolphin's software renderer on the smoke-set replays and a menu
# run: records a FIFO log of COUNT frames from frame START of each (GX_DFF, every port), draws it
# with gxplay and with Dolphin (dolphin-render.sh), and compares them frame by frame (gxplay
# diff). Each goes to local/gx/OUT/NAME/{log.dff,ours,ref,diffs,diff.txt}, and a line per run to
# local/gx/OUT/summary.txt: its frames' lowest PSNR and largest shares of pixels off by more than
# 8 and 32 levels. A run already compared is skipped; to compare again after changing the
# renderer, delete the runs' diff.txt (Dolphin's frames are kept).
#
#   OUT=frames bash tools/gx/frames.sh
cd "$(dirname "$0")/../.."
. tools/lockstep/campaign/env.sh
need_disc
export BIN=${BIN:-target/t2/release/ssbm-run$EXE} PLAY=${PLAY:-target/t2/release/gxplay$EXE}
export OUT=local/gx/${OUT:-frames} START=${START:-600} MENU_START=${MENU_START:-184} COUNT=${COUNT:-4}
export SSBM_DISC L
mkdir -p "$OUT"
{
    grep -v '^#' local/replays/smoke-set.txt | tr -d '\r' | while read -r f; do echo "$(map_replays "$f")"; done
    echo menu
} | xargs -P "${JOBS:-4}" -d '\n' -I{} bash -c '
    f="{}"
    name=$(basename "$f" .slp)
    dir=$OUT/$name
    mkdir -p "$dir"
    [[ -s $dir/log.dff ]] && exit 0
    start=$START
    if [[ $f == menu ]]; then
        # Random input from the main menu with a saved card: its panels are up by frame 184.
        cp "$L/cards/template-saved.raw" "$dir/card.raw"
        input=(--monkey 101 --mode 01 --card "$dir/card.raw")
        start=$MENU_START
    else
        input=(--replay "$f")
    fi
    GX_DFF=$dir/log.dff GX_DFF_FRAMES=$start,$COUNT timeout 3600 \
        $BIN "$SSBM_DISC" "${input[@]}" --fields $((start + 200)) --port all > "$dir/run.log" 2>&1
    echo "logged $name"'
: > "$OUT/summary.txt"
for dir in "$OUT"/*/; do
    dir=${dir%/}
    name=$(basename "$dir")
    [[ -s $dir/log.dff ]] || { echo "$name no log" >> "$OUT/summary.txt"; continue; }
    if [[ ! -s $dir/diff.txt ]]; then
        rm -rf "$dir/ours" "$dir/diffs"
        $PLAY "$dir/log.dff" "$dir/ours" > "$dir/play.log" 2>&1
        # Dolphin's frames stay until the log changes.
        if [[ $(ls "$dir/ref" 2>/dev/null | wc -l) -lt $COUNT ]]; then
            rm -rf "$dir/ref"
            TIMEOUT=120 bash tools/gx/dolphin-render.sh "$dir/log.dff" "$dir/ref" "$COUNT" > "$dir/dolphin.log" 2>&1
        fi
        $PLAY diff "$dir/ours" "$dir/ref" "$dir/diffs" > "$dir/diff.txt"
    fi
    # Worst frame: lowest PSNR, most pixels off.
    awk -v name="$name" 'NR > 1 && NF == 4 { n++; if (n == 1 || $2 < p) p = $2; if ($3 > o8) o8 = $3; if ($4 > o32) o32 = $4 }
        END { if (n) printf "%s frames %d psnr %.2f off8 %.3f off32 %.3f\n", name, n, p, o8, o32; else print name, "no frames" }' \
        "$dir/diff.txt" >> "$OUT/summary.txt"
done
cat "$OUT/summary.txt"
