"""Preserve ledger claims and scope exclusions without treating them as measurements."""
import collections
import hashlib
from pathlib import Path

CATEGORIES = {
    "dead": "Unreachable branch",
    "dev": "Developer-only code",
    "fail": "Crash or failure handling",
    "hw": "Hardware or SDK substitution",
    "oracle": "Provisional oracle coverage",
}


def ledger(path, functions, units):
    text = Path(path).read_text(encoding="utf-8")
    names = collections.defaultdict(list)
    qualified = {}
    for i, row in enumerate(functions):
        names[row[0]].append(i)
        qualified[f"{units[row[1]]}:{row[0]}"] = i
    entries, groups, pending, group = [], [], [], None
    by_function = collections.defaultdict(list)
    for line_number, line in enumerate(text.splitlines(), 1):
        if line.startswith("#"):
            pending.append(line.lstrip("# "))
            continue
        if not line.strip():
            continue
        if pending:
            groups.append(" ".join(pending))
            group, pending = len(groups) - 1, []
        body, _, reason = line.partition("#")
        claim, kind = body.split()
        name, offset = claim.rsplit("+", 1)
        if kind not in CATEGORIES:
            raise ValueError(f"{path}:{line_number}: unknown category {kind}")
        if offset != "*":
            int(offset, 16)
        indices = [qualified[name]] if name in qualified else names.get(name, [])
        if ":" not in name and len(indices) > 1:
            raise ValueError(f"{path}:{line_number}: ambiguous ledger function {name}")
        entry = {"claim": claim, "kind": kind, "reason": reason.strip(),
                 "line": line_number, "group": group}
        for i in indices:
            by_function[str(i)].append(len(entries))
        entries.append(entry)
    return {"categories": CATEGORIES, "entries": entries, "groups": groups,
            "byFunction": dict(by_function),
            "sha256": hashlib.sha256(text.encode()).hexdigest()}


def exclusions(path, kind):
    entries = []
    for line_number, line in enumerate(Path(path).read_text(encoding="utf-8").splitlines(), 1):
        body, _, reason = line.partition("#")
        fields = body.split()
        if not fields:
            continue
        address_first = fields[0].startswith("0x")
        entries.append({"name": fields[-1] if address_first else fields[0],
                        "unit": None if address_first or len(fields) < 2 else fields[1],
                        "address": fields[0] if address_first else None,
                        "reason": reason.strip(), "kind": kind, "line": line_number})
    return entries


def build_evidence(root, data, revision=None):
    root = Path(root)
    directory = root / "tools/lockstep"
    result = ledger(directory / "gaps.txt", data["fns"], data["units"])
    result["revision"] = revision
    result["historicalCopiesAvailable"] = False
    result["exclusions"] = exclusions(directory / "dead-card.txt", "unreachable function") \
        + exclusions(directory / "stand-ins-card.txt", "SDK substitution")
    result["codePaths"] = {
        unit: f"crates/game/src/tu/{unit.replace('/', '__')}.rs"
        for unit in data["units"]
        if (root / "crates/game/src/tu" / f"{unit.replace('/', '__')}.rs").exists()
    }
    return result
