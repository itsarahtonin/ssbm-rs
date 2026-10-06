"""Writes the Graphics and Audio ladder's progress page from local results: the GX stream
comparison of original-code and all-ports runs (smoke.sh), the renderer's frames against
Dolphin's software renderer (frames.sh), and the status of the rungs not measured yet. Each run
adds an entry to local/gx/progress-history.json, which the page charts.

    python3 tools/gx/progress.py [OUT_DIR] [--json SNAPSHOT]

OUT_DIR (local/gx/progress by default) gets index.html, and img/ with each run's worst frame,
ours, Dolphin's and their difference, as PNGs at full size; files.txt lists them, to publish
beside the page. --json also writes the data as the portable snapshot keeps it
(docs/progress/data/graphics.json): replays named scenario-001 on in sorted order, no images,
and the history SNAPSHOT already holds followed by the local entries measured since.
"""
import datetime
import argparse
import json
import os
import re
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
GX = os.path.join(ROOT, "local", "gx")
sys.path.insert(0, HERE)

import compare  # noqa: E402, beside this script

# A frame matches Dolphin's within tolerance when its PSNR is at least this, and no more than
# this share of its pixels is off by more than 32 levels in some channel.
TOLERANCE_PSNR = 38.0
TOLERANCE_OFF32 = 0.25

# The renderer's features, as (name, status, note): done, approximate, or missing.
FEATURES = [
    ("Vertex loading", "done", "All formats, indexed arrays, matrix indices, normal carry-over"),
    ("Transform and lighting", "done", "Dolphin's software transform unit, on the CPU"),
    ("Texture coordinate generation", "done", "Regular, emboss, color; dual transform"),
    ("Culling and clipping", "done", "GX's facing test; the GPU clips"),
    ("Rasterization", "approximate", "The GPU's, at GX's sample positions; edges differ by ties"),
    ("TEV", "done", "Integer combiners, compare modes, konst, swap tables"),
    ("Texture formats", "done", "All eleven, with palettes"),
    ("Texture sampling", "done", "By hand in integers: wrap, bilinear, mip levels and LOD"),
    ("Indirect texturing", "done", "Ported, not yet seen in Melee's frames"),
    ("Fog", "done", "All types and range adjustment"),
    ("Z textures", "done", "Late z only, as on hardware"),
    ("Alpha test and early z", "done", "Two passes when early z must still write depth"),
    ("Blending", "approximate", "The GPU's blender: off by one where GX rounds down"),
    ("Logic ops", "missing", "Not met in Melee's frames so far"),
    ("Dithering (RGBA6)", "approximate", "Exact without blending"),
    ("Lines and points", "done", "As quads, GX's caps; texture offsets missing"),
    ("EFB copies to textures", "done", "Encoded into memory as the copy writes it"),
    ("XFB copies", "done", "Copy filter, gamma, YUV 4:2:2"),
    ("Depth copies (Z24)", "missing", "Not met in Melee's frames so far"),
]

# The rungs not measured by this page's data yet: (status, detail).
WINDOW = (
    "done",
    "--window (feature window) holds 59.94 fields/s, redrawing once a frame; a gamepad plays (tried by "
    "hand), GameCube controllers on a Wii U adapter play (tried by hand, with their origins applied as "
    "the PAD library does), and the keyboard passes the unattended controls test",
)


def git(*args):
    try:
        return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()
    except (OSError, subprocess.CalledProcessError):
        return ""


def explained():
    """Explained stream differences (explained.txt): replay -> (first frame, explanation)."""
    out = {}
    for line in open(os.path.join(HERE, "explained.txt"), encoding="utf-8"):
        if line.startswith("#") or "|" not in line:
            continue
        name, first, why = (p.strip() for p in line.split("|", 2))
        out[name] = (int(first), why)
    return out


def stream_results():
    """Per smoke replay: frames compared, frames differing, and in shape, and an explanation of
    the difference when there is one (its first differing frame must match)."""
    out = []
    known = explained()
    d = os.path.join(GX, "smoke")
    if not os.path.isdir(d):
        return out
    for f in sorted(os.listdir(d)):
        if not f.endswith(".orig.txt"):
            continue
        name = f[: -len(".orig.txt")]
        a, b = os.path.join(d, f), os.path.join(d, name + ".ports.txt")
        done = all(
            os.path.exists(p) and "M instructions" in open(p, encoding="utf-8", errors="replace").read()
            for p in (os.path.join(d, name + ".orig.log"), os.path.join(d, name + ".ports.log"))
        )
        if not (done and os.path.exists(b)):
            out.append({"name": name, "done": False})
            continue
        fa, fb = compare.frames(a), compare.frames(b)
        n = min(len(fa), len(fb))
        differ = [i for i in range(n) if fa[i] != fb[i]]
        shape = [i for i in differ if fa[i][1:3] != fb[i][1:3]]
        first = differ[0] if differ else None
        why = known.get(name)
        out.append({"name": name, "done": True, "frames": n, "differ": len(differ), "shape": len(shape),
                    "first": first, "explained": why[1] if why and why[0] == first else None})
    return out


def image(path, img_dir, name):
    """Copies a frame image beside the page; its path relative to the page, or None."""
    if not os.path.exists(path):
        return None
    shutil.copyfile(path, os.path.join(img_dir, name))
    return "img/" + name


def audio_results():
    """Per run of audio.sh: command lists checked against Dolphin's AX, those that differ, and
    whether the replay synced with mixing: diverged exactly where its known list says."""
    out = []
    path = os.path.join(GX, "audio", "summary.txt")
    if not os.path.exists(path):
        return out
    for line in open(path, encoding="utf-8"):
        parts = [p.strip() for p in line.split("|")]
        if len(parts) < 3:
            continue
        name, lists, sync = parts[0], parts[1], parts[2]
        w = lists.split()
        checked = int(w[0]) if w and w[0].isdigit() else 0
        differ = int(w[3]) if len(w) > 3 and w[3].isdigit() else None
        m = re.search(r" 0 not reached, (\d+) diverge \((\d+) known\), 0 known no longer", sync)
        synced = sync == "no replay" or bool(m and m[1] == m[2])
        out.append({"name": name, "lists": checked, "differ": differ, "sync": sync, "synced": synced})
    return out


def frame_results(img_dir):
    out = []
    d = os.path.join(GX, "frames")
    if not os.path.isdir(d):
        return out
    for name in sorted(os.listdir(d)):
        run = os.path.join(d, name)
        diff = os.path.join(run, "diff.txt")
        if not os.path.isdir(run):
            continue
        frames = []
        if os.path.exists(diff):
            for line in open(diff, encoding="utf-8"):
                w = line.split()
                if len(w) == 4 and w[0].isdigit():
                    frames.append({"n": int(w[0]), "psnr": float(w[1]), "off8": float(w[2]), "off32": float(w[3])})
        if not frames:
            out.append({"name": name, "frames": []})
            continue
        worst = min(frames, key=lambda f: f["psnr"])
        ok = all(f["psnr"] >= TOLERANCE_PSNR and f["off32"] <= TOLERANCE_OFF32 for f in frames)
        n = worst["n"]
        out.append({
            "name": name, "frames": frames, "ok": ok, "worst": worst,
            "ours": image(os.path.join(run, "ours", f"frame_{n}.png"), img_dir, f"{name}-ours.png"),
            "ref": image(os.path.join(run, "ref", f"framedump_{n}.png"), img_dir, f"{name}-dolphin.png"),
            "diff": image(os.path.join(run, "diffs", f"diff_{n}.png"), img_dir, f"{name}-diff.png"),
        })
    return out


def write_snapshot(path, data):
    """The page's data as the portable snapshot keeps it: replays anonymized, no images, and the
    snapshot's history (its commits mapped to this repository) before the newer local entries."""
    old = json.load(open(path, encoding="utf-8")) if os.path.exists(path) else {}
    names = sorted({r["name"] for kind in ("streams", "frames", "audio") for r in data[kind]})
    label = {n: f"scenario-{i + 1:03d}" for i, n in enumerate(names)}
    snap = json.loads(json.dumps(data))
    for kind in ("streams", "frames", "audio"):
        for r in snap[kind]:
            r["name"] = label[r["name"]]
    for r in snap["frames"]:
        r["ours"] = r["ref"] = r["diff"] = None
    history = old.get("history", [])
    last = history[-1]["at"] if history else ""
    commit = git("rev-parse", "HEAD")
    for entry in data["history"]:
        if entry["at"] > last:
            history.append(dict(entry, commit=git("rev-parse", entry["commit"]) or entry["commit"]))
    snap.update(
        branch="main",
        commit=commit,
        history=history,
        expected={"streams": len(snap["streams"]), "frames": len(snap["frames"]), "audio": len(snap["audio"])},
        imagesOmitted=True,
        provenance={
            "sourceRevision": commit,
            "exportedUtc": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d %H:%M UTC"),
            "kind": "measured by smoke.sh, frames.sh and audio.sh at the source revision",
            "measuredRevision": commit,
            "imagePolicy": "Screenshots remain in the private evidence set; scenario filenames anonymized.",
        },
    )
    with open(path, "w", encoding="utf-8", newline="\n") as file:
        json.dump(snap, file, separators=(",", ":"), allow_nan=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out_dir", nargs="?", default=os.path.join(GX, "progress"))
    parser.add_argument("--from-json")
    parser.add_argument("--json")
    args = parser.parse_args()
    out_dir = args.out_dir
    if args.from_json:
        with open(args.from_json, encoding="utf-8") as file:
            data = json.load(file)
        os.makedirs(out_dir, exist_ok=True)
        with open(os.path.join(HERE, "progress.template.html"), encoding="utf-8") as file:
            template = file.read()
        encoded = json.dumps(data, separators=(",", ":"), allow_nan=False).replace("<", "\\u003c")
        with open(os.path.join(out_dir, "index.html"), "w", encoding="utf-8", newline="\n") as file:
            file.write(template.replace("/*DATA*/null", encoded))
        return
    img_dir = os.path.join(out_dir, "img")
    shutil.rmtree(img_dir, ignore_errors=True)
    os.makedirs(img_dir)
    streams = stream_results()
    frames = frame_results(img_dir)
    now = datetime.datetime.now().strftime("%Y-%m-%d %H:%M")
    done_streams = [s for s in streams if s["done"]]
    measured = [f for f in frames if f["frames"]]
    entry = {
        "at": now,
        "commit": git("rev-parse", "--short", "HEAD"),
        "streams_zero": sum(1 for s in done_streams if s["differ"] == 0 or s.get("explained")),
        "streams_done": len(done_streams),
        "frames_ok": sum(1 for f in measured if f["ok"]),
        "frames_done": len(measured),
        "worst_psnr": min((f["worst"]["psnr"] for f in measured), default=None),
    }
    history_path = os.path.join(GX, "progress-history.json")
    history = json.load(open(history_path, encoding="utf-8")) if os.path.exists(history_path) else []
    if not history or {k: v for k, v in history[-1].items() if k != "at"} != {k: v for k, v in entry.items() if k != "at"}:
        history.append(entry)
        json.dump(history, open(history_path, "w", encoding="utf-8"), indent=1)
    data = {
        "generated": now,
        "branch": git("rev-parse", "--abbrev-ref", "HEAD"),
        "commit": entry["commit"],
        "subject": git("log", "-1", "--format=%s"),
        "tolerance": {"psnr": TOLERANCE_PSNR, "off32": TOLERANCE_OFF32},
        "streams": streams,
        "frames": frames,
        "features": [{"name": n, "status": s, "note": t} for n, s, t in FEATURES],
        "audio": audio_results(),
        "window": {"status": WINDOW[0], "detail": WINDOW[1]},
        "history": history,
    }
    if args.json:
        write_snapshot(args.json, data)
    template = open(os.path.join(HERE, "progress.template.html"), encoding="utf-8").read()
    html = template.replace("/*DATA*/null", json.dumps(data, separators=(",", ":")))
    out = os.path.join(out_dir, "index.html")
    open(out, "w", encoding="utf-8", newline="\n").write(html)
    images = sorted("img/" + f for f in os.listdir(img_dir))
    open(os.path.join(out_dir, "files.txt"), "w", encoding="utf-8", newline="\n").write("\n".join(images) + "\n")
    print(f"{out}: {len(html) // 1024} KB and {len(images)} images; streams {entry['streams_zero']}/{entry['streams_done']} at zero diff, "
          f"frames {entry['frames_ok']}/{entry['frames_done']} within tolerance")


if __name__ == "__main__":
    main()
