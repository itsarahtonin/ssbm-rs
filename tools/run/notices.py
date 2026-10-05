"""Writes the license notices of the third-party crates built into the player's executable: the
crates its `player` build depends on for TARGET (normal dependencies, not build scripts' or
tests'), each with its license expression, and the license files it ships, identical texts
written once with the crates that share them.

    python tools/run/notices.py OUT [TARGET]

TARGET defaults to x86_64-pc-windows-msvc. Crates that ship no license file are listed with
their license expression alone.
"""

import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
LICENSE_NAME = re.compile(r"^(licen[cs]e|copying|notice|unlicense)", re.I)


def metadata(target):
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--features", "player",
         "--filter-platform", target, "--manifest-path", os.path.join(ROOT, "tools/run/Cargo.toml")],
        capture_output=True, text=True, encoding="utf-8", check=True).stdout
    return json.loads(out)


def built_in(meta):
    """The packages the player's executable links: ssbm-run's normal dependencies, transitively."""
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    root = next(p["id"] for p in meta["packages"] if p["name"] == "ssbm-run")
    seen, todo = set(), [root]
    while todo:
        id = todo.pop()
        if id in seen:
            continue
        seen.add(id)
        for dep in nodes[id]["deps"]:
            if any(k["kind"] is None for k in dep["dep_kinds"]):
                todo.append(dep["pkg"])
    return [p for p in meta["packages"] if p["id"] in seen and p["source"] is not None]


def license_files(package):
    folder = os.path.dirname(package["manifest_path"])
    names = []
    if package.get("license_file"):
        names.append(package["license_file"])
    names += sorted(f for f in os.listdir(folder) if LICENSE_NAME.match(f)
                    and os.path.isfile(os.path.join(folder, f)) and f not in names)
    texts = []
    for name in names:
        path = os.path.join(folder, name)
        if os.path.isfile(path):
            with open(path, encoding="utf-8", errors="replace") as f:
                texts.append((name, f.read().strip()))
    return texts


def main():
    out = sys.argv[1]
    target = sys.argv[2] if len(sys.argv) > 2 else "x86_64-pc-windows-msvc"
    packages = sorted(built_in(metadata(target)), key=lambda p: (p["name"], p["version"]))
    texts = {}  # normalized text -> (text, [package labels])
    bare = []
    for p in packages:
        label = f"{p['name']} {p['version']}"
        files = license_files(p)
        if not files:
            bare.append(f"{label} ({p.get('license') or 'no license expression'})")
        for _, text in files:
            key = re.sub(r"\s+", " ", text)
            texts.setdefault(key, (text, []))[1].append(label)
    rule = "=" * 78
    lines = [
        "Third-party notices for the ssbm-rs player executable",
        "",
        f"The executable includes these {len(packages)} Rust crates, built for {target}. Each is",
        "used under its own license; their license files follow, each text once with the crates",
        "that ship it. ssbm-rs itself is GPL-3.0-or-later (LICENSE.txt); its own provenance notes,",
        "including code adapted from Dolphin and Slippi, are in THIRD_PARTY.md in its source.",
        "",
        rule,
        "Crates and license expressions",
        rule,
    ]
    lines += [f"{p['name']} {p['version']}: {p.get('license') or 'see its license file'}"
              + (f" ({p['repository']})" if p.get("repository") else "") for p in packages]
    for text, labels in sorted(texts.values(), key=lambda t: t[1][0]):
        lines += ["", rule, "Used by: " + ", ".join(labels), rule, "", text]
    if bare:
        lines += ["", rule, "Crates that ship no license file (their license expression applies)",
                  rule, ""] + bare
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(lines) + "\n")
    print(f"{len(packages)} crates, {len(texts)} distinct license texts, {len(bare)} without a file")


if __name__ == "__main__":
    main()
