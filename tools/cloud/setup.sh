#!/bin/bash
# Sets up a cloud session (Ubuntu) for lockstep campaigns: downloads the private release
# (tools/cloud/pack.sh made its bundles; the disc image goes in as it is), checks the disc is the
# reference GALE01 rev 2, unpacks the replays, card templates and campaign state, checks out the
# melee decomp at the commit the tools read and adds its disc-made files, installs the Python
# packages the tools need, writes local/env.sh and builds ssbm-run. Safe to run again: each step
# is skipped once done.
#
#   bash tools/cloud/setup.sh
set -euo pipefail
cd "$(dirname "$0")/../.."
REPO=$(pwd)
DATA=${SSBM_DATA:-$HOME/ssbm-data}
RELEASE=${RELEASE:-private-data}
GH_REPO=${GH_REPO:-itsarahtonin/ssbm-rs}
MELEE_COMMIT=af0423e32394cdade84de17376c96f928961c534
DISC_MD5=0e63d4223b01d9aba596259dc155a174
MELEE=$(dirname "$REPO")/melee
mkdir -p "$DATA"

# The reports judge each round's results stale against the commits since its binary.
if [[ $(git rev-parse --is-shallow-repository) == true ]]; then
    git fetch -q --unshallow
fi

if [[ ! -f $DATA/.downloaded ]]; then
    echo "downloading release $RELEASE"
    # gh release download goes through GraphQL, which Claude Code's cloud proxy refuses; the REST
    # asset endpoints work there.
    if ! gh release download "$RELEASE" -R "$GH_REPO" -D "$DATA" --clobber; then
        gh api "repos/$GH_REPO/releases/tags/$RELEASE" --jq '.assets[] | "\(.id) \(.name)"' |
            while read -r id name; do
                gh api -H "Accept: application/octet-stream" "repos/$GH_REPO/releases/assets/$id" > "$DATA/$name"
            done
    fi
    touch "$DATA/.downloaded"
fi
disc=$(ls "$DATA"/*.iso | head -1)
if [[ ! -f $DATA/.disc-ok ]]; then
    sum=$(md5sum "$disc" | cut -d' ' -f1)
    [[ $sum == "$DISC_MD5" ]] || { echo "$disc: MD5 $sum, not the reference $DISC_MD5" >&2; exit 1; }
    touch "$DATA/.disc-ok"
fi

if [[ ! -d $DATA/game ]]; then
    mkdir -p "$DATA/game"
    tar -C "$DATA/game" -xzf "$DATA/ssbm-game-data.tar.gz"
fi
if [[ ! -d local/lockstep ]]; then
    tar -xzf "$DATA/ssbm-state.tar.gz"
fi
mkdir -p local/lockstep/cards
cp -n "$DATA/game/cards/template-saved.raw" local/lockstep/cards/
cp -rn "$DATA/game/cards/faults" local/lockstep/cards/

if [[ ! -d $MELEE/.git ]]; then
    git clone https://github.com/doldecomp/melee.git "$MELEE"
fi
git -C "$MELEE" checkout -q "$MELEE_COMMIT"
mkdir -p "$MELEE/build"
cp -rn "$DATA/game/melee/orig" "$MELEE/"
cp -rn "$DATA/game/melee/build/." "$MELEE/build/"

if [[ ! -x $MELEE/.venv/bin/python ]]; then
    python3 -m venv "$MELEE/.venv"
    "$MELEE/.venv/bin/pip" install -q libclang==18.1.1 pyelftools ninja
fi
# c2rs reads the units from objdiff.json and their flags from build.ninja.
[[ -f $MELEE/objdiff.json ]] || (cd "$MELEE" && .venv/bin/python configure.py)

cat > local/env.sh <<EOF
# This cloud VM's paths for tools/lockstep/campaign/env.sh (written by tools/cloud/setup.sh).
SSBM_DISC="$disc"
MELEE="$MELEE"
REPLAYS="$DATA/game/slippi"
REPLAYS_FROM="C:/Users/malur/OneDrive/Documents/Slippi"
EOF

cargo build --release -p ssbm-run --target-dir target/t2
echo "ready: $(nproc) CPUs; SSBM_DISC=$disc"
