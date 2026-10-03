#!/bin/bash
# Draws a FIFO log (GX_DFF) with Dolphin's software renderer, the graphics reference, and
# saves its frames as images in OUT: runs DOLPHIN (a portable Dolphin build) unattended, with
# its panic dialogs and stop confirmation off, until FRAMES images are saved or TIMEOUT
# seconds pass, and stops it.
#
#   DOLPHIN=/c/Projects/dolphin-dev/Dolphin-x64 bash tools/gx/dolphin-render.sh LOG.dff OUT [FRAMES]
set -u
DOLPHIN=${DOLPHIN:-/c/Projects/dolphin-dev/Dolphin-x64}
LOG=$(cd "$(dirname "$1")" && pwd -W 2>/dev/null || pwd)/$(basename "$1")
OUT=$2
FRAMES=${3:-1}
TIMEOUT=${TIMEOUT:-60}
dump="$DOLPHIN/User/Dump/Frames"
rm -rf "$dump"
mkdir -p "$OUT"
"$DOLPHIN/Dolphin.exe" -b -e "$LOG" \
    -C "Dolphin.Core.GFXBackend=Software Renderer" \
    -C Dolphin.Interface.UsePanicHandlers=False \
    -C Dolphin.Interface.ConfirmStop=False \
    -C Dolphin.Movie.DumpFrames=True \
    -C Dolphin.Movie.DumpFramesSilent=True \
    -C GFX.Settings.DumpFramesAsImages=True > /dev/null 2>&1 &
pid=$!
winpid=$(cat /proc/$pid/winpid 2>/dev/null)
for _ in $(seq $((TIMEOUT * 2))); do
    n=$(ls "$dump" 2>/dev/null | wc -l)
    [[ $n -ge $FRAMES ]] && break
    kill -0 $pid 2>/dev/null || break
    sleep 0.5
done
# The last image may still be being written.
sleep 1
kill $pid 2>/dev/null
# Only this Dolphin, by its Windows process id.
[[ -n $winpid ]] && taskkill //F //PID "$winpid" > /dev/null 2>&1
cp "$dump"/* "$OUT"/ 2>/dev/null
echo "$(ls "$OUT" | wc -l) frames in $OUT"
grep -h "Condition\|Error\|error" "$DOLPHIN/User/Logs/dolphin.log" 2>/dev/null | tail -5
