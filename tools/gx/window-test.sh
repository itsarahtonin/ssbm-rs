#!/bin/bash
# Tests --window's controls unattended: boots the game in a window on a desktop nobody sees
# (hidden.ps1), presses keys in it the way a keyboard does (keys.ps1, run on that desktop too),
# and checks the game answers: X (A) skips the opening movie to the title, Enter (Start) opens the
# main menu, and the left arrow moves the stick. BIN is a binary built with the window feature.
# It leaves a GameCube adapter alone unless ADAPTER=1, which also shows what the adapter reads.
#
#   cargo build --release -p ssbm-run --features window --target-dir target/w
#   BIN=target/w/release/ssbm-run.exe bash tools/gx/window-test.sh
cd "$(dirname "$0")/../.."
. tools/lockstep/campaign/env.sh
need_disc
BIN=${BIN:-target/w/release/ssbm-run$EXE}
out=$(mktemp -d)
cp "$L/cards/template-saved.raw" "$out/card.raw"
w=$(cygpath -w "$out")
cat > "$out/game.cmd" <<EOF
@echo off
set SSBM_MUTE=1
set MODES=1
set SSBM_TRACE_PADS=1
$([[ ${ADAPTER:-0} == 1 ]] || echo "set SSBM_NO_ADAPTER=1")
cd /d "$(cygpath -wa .)"
"$(cygpath -wa "$BIN")" "$(cygpath -w "$SSBM_DISC")" --window --card "$w\\card.raw" --fields 1200 2>"$w\\game.log"
EOF
cat > "$out/keys.cmd" <<EOF
@echo off
powershell -NoProfile -ExecutionPolicy Bypass -File "$(cygpath -wa tools/gx/keys.ps1)" -Timeout 40 -Script "wait:6,X:0.3,wait:2,Return:0.3,wait:2,Left:0.5,wait:2" > "$w\\keys.log" 2>&1
EOF
hidden() {
    powershell -NoProfile -ExecutionPolicy Bypass -File "$(cygpath -wa tools/gx/hidden.ps1)" -Timeout "$1" \
        -Exe "C:\\Windows\\System32\\cmd.exe" -CommandArgs "/c \"$w\\$2\"" > /dev/null
}
hidden 120 game.cmd &
game=$!
sleep 2
hidden 60 keys.cmd
wait $game
log=$out/game.log
ok=1
check() {
    if grep -q "$2" "$log"; then echo "ok: $1"; else echo "FAILED: $1"; ok=0; fi
}
check "the ports ran the game" "window: the Rust ports run the game"
check "A pressed" "pad 1: buttons 0100"
check "the movie skipped to the title" "game mode 0x00"
check "Start pressed" "pad 1: buttons 1000"
check "the main menu opened" "game mode 0x01"
check "the stick pushed left" "stick -127 0"
grep -E "window: (gamepad|no gamepad|port|GameCube|found)" "$log"
[[ ${ADAPTER:-0} == 1 ]] && grep -E "^pad [2-4]" "$log" | head
grep "window:.*fields" "$log" | tail -1
[[ $ok == 1 ]] || { cat "$out/keys.log"; echo "logs in $out"; exit 1; }
rm -rf "$out"
