#!/bin/bash
# Builds the player's release into local/release/vVERSION: ssbm-rs.exe (the `player` feature),
# double-clicked, plays in a window, asking for the Melee disc image (NTSC 1.02) the first time
# and keeping that choice and the memory card in its settings folder (%APPDATA%\ssbm-rs on
# Windows). Beside it: LICENSE.txt (GPL-3.0), THIRD-PARTY-NOTICES.txt (tools/run/notices.py) and
# SHA256SUMS.txt. It holds no game data; the disc is the player's own.
#
# On Windows the C runtime is linked in (+crt-static), so the executable starts on a PC
# without the Visual C++ redistributable.
#
#   bash tools/run/release.sh
set -e
cd "$(dirname "$0")/../.."
# Windows may have only the Store's python3 stub, which runs nothing.
if [[ $OS == Windows_NT ]]; then py=python; else py=$(command -v python3 || command -v python); fi
sum() { if command -v sha256sum >/dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
out=local/release/v$version
exe=ssbm-run
if [[ $OS == Windows_NT ]]; then
    exe=ssbm-run.exe
    export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C target-feature=+crt-static"
fi
cargo build --locked --release -p ssbm-run --features player --target-dir target/player
if [[ $OS == Windows_NT ]] && grep -qai "vcruntime140" "target/player/release/$exe"; then
    echo "target/player/release/$exe still imports the Visual C++ runtime" >&2
    exit 1
fi
rm -rf "$out"
mkdir -p "$out"
cp "target/player/release/$exe" "$out/${exe/ssbm-run/ssbm-rs}"
cp LICENSE "$out/LICENSE.txt"
"$py" tools/run/notices.py "$out/THIRD-PARTY-NOTICES.txt" "$(rustc -vV | sed -n 's/^host: //p')"
(cd "$out" && sum "${exe/ssbm-run/ssbm-rs}" LICENSE.txt THIRD-PARTY-NOTICES.txt > SHA256SUMS.txt)
echo "$out:"
cat "$out/SHA256SUMS.txt"
