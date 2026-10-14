"""Checks regflow's following of registers through calls, on small made-up listings.

    python -m unittest discover -s tools/c2rs -p test_regflow.py
"""

import json
import os
import tempfile
import unittest

import regflow


def listing(funcs):
    """A listing file's text for {name: [instruction or "label:"]}, at made-up addresses, with
    branches naming labels by name."""
    at, labels = 0x80003000, {}
    for body in funcs.values():
        for insn in body:
            if insn.endswith(":"):
                labels[insn[:-1]] = at
            else:
                at += 4
    lines, at = [], 0x80003000
    for name, body in funcs.items():
        lines.append(f".fn {name}, global")
        for insn in body:
            if insn.endswith(":"):
                lines.append(f".L_{labels[insn[:-1]]:08X}:")
                continue
            mnem, _, ops = insn.partition(" ")
            for label, a in labels.items():
                ops = ops.replace(label, f".L_{a:08X}")
            lines.append(f"/* {at:08X} 00000000  00 00 00 00 */\t{mnem} {ops}".rstrip())
            at += 4
        lines.append(f".endfn {name}")
    return "\n".join(lines) + "\n"


class Regflow(unittest.TestCase):
    def program(self, funcs):
        root = tempfile.mkdtemp()
        asm = os.path.join(root, "build", "GALE01", "asm")
        os.makedirs(asm)
        with open(os.path.join(asm, "t.s"), "w", encoding="utf-8") as f:
            f.write(listing(funcs))
        one_int = {"params": [{"k": "int", "size": 4}], "ret": {"k": "void"}}
        types = {"functions": [{"name": n, "addr": i, "type": one_int, "tu": "t"}
                               for i, n in enumerate(funcs)]}
        path = os.path.join(root, "types.json")
        with open(path, "w", encoding="utf-8") as f:
            json.dump(types, f)
        return regflow.Program(root, path)

    # The reader passes on r5 its caller left, as ftCo_800AC5A0 passes its stick x.
    READER = ["mr r4, r5", "blr"]

    def test_follows_registers_a_callee_may_leave_as_it_found_them(self):
        prog = self.program({
            "reader": self.READER,
            # Sets r5 on one way back only.
            "sometimes": ["cmpwi r3, 0x0", "beq skip", "li r5, 0x1", "skip:", "blr"],
            "middle": ["bl sometimes", "bl reader", "blr"],
            "top": ["li r5, 0x7", "bl middle", "blr"],
        })
        callers, left = prog.callers_to_port(["reader"])
        self.assertEqual((set(callers), left), ({"middle", "top"}, []))
        down = prog.callees_to_port({"reader"} | set(callers))
        self.assertEqual(set(down), {"sometimes"})
        self.assertEqual(down["sometimes"][1], {("r", 5)})

    def test_stops_at_a_callee_that_always_sets_the_register(self):
        prog = self.program({
            "reader": self.READER,
            "always": ["li r5, 0x1", "blr"],
            "middle": ["bl always", "bl reader", "blr"],
            "top": ["li r5, 0x7", "bl middle", "blr"],
        })
        callers, _ = prog.callers_to_port(["reader"])
        self.assertEqual(set(callers), {"middle"})
        self.assertEqual(set(prog.callees_to_port({"reader", "middle"})), {"always"})

    def test_writes_before_a_failed_assertion_leave_nothing(self):
        prog = self.program({
            "reader": self.READER,
            "__assert": ["blr"],
            "checks": ["cmpwi r3, 0x0", "bne fine", "li r5, 0x1", "bl __assert",
                       "fine:", "blr"],
            "middle": ["bl checks", "bl reader", "blr"],
            "top": ["li r5, 0x7", "bl middle", "blr"],
        })
        prog.writes()
        self.assertEqual(prog.some["checks"], set())
        callers, _ = prog.callers_to_port(["reader"])
        self.assertEqual(set(callers), {"middle", "top"})
        self.assertEqual(prog.callees_to_port({"reader"} | set(callers)), {})


if __name__ == "__main__":
    unittest.main()
