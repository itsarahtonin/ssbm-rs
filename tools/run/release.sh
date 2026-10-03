#!/bin/bash
# Builds the player's executable (the `player` feature) into local/release: double-clicked, it
# plays in a window, asking for the Melee disc image (NTSC 1.02) the first time. It keeps that
# choice and the memory card in its settings folder (%APPDATA%\ssbm-rs on Windows). It holds
# no game data; the disc is the player's own.
#
#   bash tools/run/release.sh
set -e
cd "$(dirname "$0")/../.."
cargo build --release -p ssbm-run --features player --target-dir target/player
mkdir -p local/release
exe=ssbm-run
[[ -f target/player/release/ssbm-run.exe ]] && exe=ssbm-run.exe
cp "target/player/release/$exe" "local/release/${exe/ssbm-run/ssbm-rs}"
echo "local/release/${exe/ssbm-run/ssbm-rs}"
