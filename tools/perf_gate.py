#!/usr/bin/env python3
"""Performance gate over perf_cells.json files (TIDY 4.15).

Usage: python3 tools/perf_gate.py <perf_cells.json>...

Rule (user decision, not to be changed here): hand Rust 1.0x is the floor.
Only data-proportional cells at representative sizes M and L gate: any such
cell with pruv/hand > 1.0 is a failure (exit 1). Fixed-share cells and size S
cells above 1.0 are debt, printed for docs/L3_RESTRUCTURE_SPEC.md §7.5 (exit 0).
There is no noise tolerance band. An input with no cells at all fails.

perf_cells.json: a JSON list (or {"source": str, "cells": [...]}) of cells
  {"name": str, "axis": "time"|"rss"|"allocs", "size": "S"|"M"|"L",
   "kind": "proportional"|"fixed", "pruv": number, "hand": number,
   "unit": str, "runs": int (optional, interleaved runs)}
A whole-run time under 1 ms counts as tiny: record it as size S.
"""
import json
import sys

AXES = {"time", "rss", "allocs"}
SIZES = {"S", "M", "L"}
KINDS = {"proportional", "fixed"}


def load(path):
    with open(path) as f:
        data = json.load(f)
    if isinstance(data, dict):
        data = data.get("cells")
    if not isinstance(data, list):
        raise ValueError("%s: expected a list of cells" % path)
    cells = []
    for i, c in enumerate(data):
        where = "%s cell %d" % (path, i)
        for k in ("name", "axis", "size", "kind", "pruv", "hand", "unit"):
            if k not in c:
                raise ValueError("%s: missing %s" % (where, k))
        if c["axis"] not in AXES or c["size"] not in SIZES or c["kind"] not in KINDS:
            raise ValueError("%s: bad axis/size/kind %r/%r/%r" % (where, c["axis"], c["size"], c["kind"]))
        if not isinstance(c["pruv"], (int, float)) or not isinstance(c["hand"], (int, float)):
            raise ValueError("%s: pruv and hand must be numbers" % where)
        cells.append(dict(c, file=path))
    return cells


def ratio(c):
    if c["hand"] == 0:
        return 1.0 if c["pruv"] == 0 else float("inf")
    return c["pruv"] / c["hand"]


def main(argv):
    if len(argv) < 2:
        print("usage: python3 tools/perf_gate.py <perf_cells.json>...")
        return 2
    try:
        cells = [c for p in argv[1:] for c in load(p)]
    except (OSError, ValueError) as e:
        print("ERROR %s" % e)
        return 2
    if not cells:
        print("VERDICT: FAIL (0 cells: nothing was measured)")
        return 1
    fail, debt, ok = [], [], []
    for c in cells:
        r = ratio(c)
        if r <= 1.0:
            ok.append((c, r))
        elif c["kind"] == "proportional" and c["size"] in ("M", "L"):
            fail.append((c, r))
        else:
            debt.append((c, r))

    def row(c, r):
        runs = str(c.get("runs", "-"))
        return "  %-40s %-6s %-4s %-12s %12g %12g %-8s %5s  %.3fx" % (
            c["name"][:40], c["axis"], c["size"], c["kind"], c["pruv"], c["hand"], c["unit"], runs, r)

    head = "  %-40s %-6s %-4s %-12s %12s %12s %-8s %5s  %s" % (
        "name", "axis", "size", "kind", "pruv", "hand", "unit", "runs", "pruv/hand")
    for title, group in (("at or below hand", ok), ("debt (record in §7.5)", debt),
                         ("FAIL (proportional M/L above hand)", fail)):
        print("%s: %d" % (title, len(group)))
        if group:
            print(head)
            for c, r in group:
                print(row(c, r))
    print("VERDICT: %s (%d cell(s), %d fail, %d debt)" % ("FAIL" if fail else "PASS", len(cells), len(fail), len(debt)))
    return 1 if fail else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
