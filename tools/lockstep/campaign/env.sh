# Settings the campaign scripts share, sourced from the repository root. The machine's own paths
# go in local/env.sh (ignored), which this reads first:
#
#   SSBM_DISC     the disc image (ISO, RVZ, CISO or GCZ), the reference GALE01 rev 2
#   MELEE         the melee decomp, built (default: ../melee)
#   PY            the Python the tools run with (default: the decomp's .venv, else python3)
#   REPLAYS       where the Slippi replays are; REPLAYS_FROM is the folder job lists and replay
#                 lists name them under, when that differs (another machine's)
#
# Campaign state lives in local/lockstep (L); these scripts live in tools/lockstep/campaign (T).
[[ -f local/env.sh ]] && . local/env.sh
L=local/lockstep
T=tools/lockstep/campaign
MELEE=${MELEE:-$(cd .. && pwd)/melee}
if [[ -z ${PY:-} ]]; then
    for p in "$MELEE/.venv/Scripts/python" "$MELEE/.venv/bin/python"; do
        [[ -x $p || -x $p.exe ]] && PY=$p && break
    done
    PY=${PY:-python3}
fi
# ssbm-run's file name: Windows builds end in .exe.
EXE=""
[[ ${OS:-} == Windows_NT ]] && EXE=.exe
export SSBM_DISC MELEE PY REPLAYS REPLAYS_FROM L T EXE

# The replay paths in $1 (a job's arguments, or a list), with REPLAYS_FROM moved to REPLAYS.
# REPLAYS_FROM is a Windows folder (C:/Users/...), which lists also name as Git Bash does
# (/c/Users/...).
map_replays() {
    if [[ -n ${REPLAYS_FROM:-} && -n ${REPLAYS:-} ]]; then
        local s=${1//\\//} from=${REPLAYS_FROM//\\//}
        local bash_form="/${from:0:1}"
        bash_form="${bash_form,,}${from:2}"
        s=${s//$from/$REPLAYS}
        printf '%s' "${s//$bash_form/$REPLAYS}"
    else
        printf '%s' "$1"
    fi
}
export -f map_replays

need_disc() {
    [[ -n ${SSBM_DISC:-} && -f $SSBM_DISC ]] || {
        echo "set SSBM_DISC (in local/env.sh) to the disc image" >&2
        exit 1
    }
}
