#!/bin/bash
# Packs what a cloud session needs beyond the repository into OUTDIR, for the private release
# that tools/cloud/setup.sh downloads. None of it may go into Git:
#
#   ssbm-game-data.tar.gz   the screened Slippi replays, the card templates, and the decomp's
#                           files made from the disc (orig/, the asm listings) with its compilers
#   ssbm-state.tar.gz       the lockstep campaign's state (local/lockstep, local/replays lists,
#                           local/typegen), without the per-job card copies and saved calls
#
# The disc image goes into the release as it is. Paths come from local/env.sh (env.sh).
#
#   OUTDIR=../ssbm-bundles bash tools/cloud/pack.sh
cd "$(dirname "$0")/../.."
. tools/lockstep/campaign/env.sh
OUTDIR=${OUTDIR:?OUTDIR}
mkdir -p "$OUTDIR"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

# Replays: the playable ones, under slippi/ as they sit under REPLAYS.
mkdir -p "$stage/game/slippi"
tr -d '\r' < local/replays/playable.txt | while read -r f; do
    # Slippi keeps them in month folders: slippi/2026-01/Game_....slp.
    rel="$(basename "$(dirname "$f")")/$(basename "$f")"
    mkdir -p "$stage/game/slippi/$(dirname "$rel")"
    cp "$f" "$stage/game/slippi/$rel"
done
mkdir -p "$stage/game/cards"
cp $L/cards/template-saved.raw "$stage/game/cards/"
cp -r $L/cards/faults "$stage/game/cards/"
mkdir -p "$stage/game/melee/build"
cp -r "$MELEE/orig" "$stage/game/melee/"
cp -r "$MELEE/build/GALE01" "$stage/game/melee/build/"
cp -r "$MELEE/build/compilers" "$stage/game/melee/build/"
tar -C "$stage/game" -czf "$OUTDIR/ssbm-game-data.tar.gz" .

tar -czf "$OUTDIR/ssbm-state.tar.gz" \
    --exclude='local/lockstep/cards' --exclude='local/lockstep/study' \
    --exclude='local/lockstep/repro' --exclude='*.call' --exclude='*.corp' \
    local/lockstep local/replays local/typegen
ls -la "$OUTDIR"
