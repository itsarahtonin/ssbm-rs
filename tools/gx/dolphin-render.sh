#!/bin/bash
# Draws a FIFO log (GX_DFF) with Dolphin's software renderer, the graphics reference, and
# saves its frames as images in OUT: runs DOLPHIN (a portable Dolphin build) unattended, with
# its panic dialogs and stop confirmation off, on a desktop of its own that nobody sees
# (hidden.ps1), until FRAMES images are saved or TIMEOUT seconds pass. The copy filter
# (deflicker) is on, as on hardware: Dolphin turns it off by default, and only
# User/Config/GFX.ini's [Enhancements] DisableCopyFilter = False turns it on (the command line's
# GFX settings don't take).
#
#   DOLPHIN=/c/Projects/dolphin-dev/Dolphin-x64 bash tools/gx/dolphin-render.sh LOG.dff OUT [FRAMES]
set -u
DOLPHIN=${DOLPHIN:-/c/Projects/dolphin-dev/Dolphin-x64}
LOG=$(cygpath -wa "$1")
OUT=$2
FRAMES=${3:-1}
TIMEOUT=${TIMEOUT:-60}
dump="$DOLPHIN/User/Dump/Frames"
rm -rf "$dump"
mkdir -p "$OUT"
dolphin=$(cygpath -w "$DOLPHIN")
powershell -NoProfile -ExecutionPolicy Bypass -File "$(cygpath -w "$(dirname "$0")/hidden.ps1")" \
    -Timeout "$TIMEOUT" -WatchDir "$dolphin\\User\\Dump\\Frames" -WatchCount "$FRAMES" \
    -Exe "$dolphin\\Dolphin.exe" -CommandArgs "-b -e \"$LOG\" \
-C \"Dolphin.Core.GFXBackend=Software Renderer\" -C Dolphin.Interface.UsePanicHandlers=False \
-C Dolphin.Interface.ConfirmStop=False -C Dolphin.Movie.DumpFrames=True \
-C Dolphin.Movie.DumpFramesSilent=True -C GFX.Settings.DumpFramesAsImages=True \
-C GFX.Enhancements.DisableCopyFilter=False"
cp "$dump"/* "$OUT"/ 2>/dev/null
echo "$(ls "$OUT" | wc -l) frames in $OUT"
grep -h "Condition\|Error\|error" "$DOLPHIN/User/Logs/dolphin.log" 2>/dev/null | tail -5
