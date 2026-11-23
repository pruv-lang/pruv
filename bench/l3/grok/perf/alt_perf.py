#!/usr/bin/env python3
"""Alternating old/new timing of two perf_driver builds (LG33).

  python3 bench/l3/grok/perf/alt_perf.py --exe old=PATH --exe new=PATH [--rows hist-ord-2m,...] [--rounds 10]
      [--time-ms 400] [--label rs-llvm] [--out DIR]

Each PATH is a perf_driver built by run_perf.py (`check` mode, copied aside per compiler). Every round runs, per row,
the Pruv side of each build and the hand side (from the last --exe), in an order rotated each round, so load drifts hit
every side alike. Peak RSS is the median of 3 `/usr/bin/time -l` runs per side. Writes one tools/perf_gate.py style
perf_cells_<side>.json per build into --out (default: the run_perf work dir) and prints a table with the load.
"""
import argparse, importlib.util, json, os, statistics, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("run_perf", os.path.join(HERE, "run_perf.py"))
rp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rp)


def load():
    return os.getloadavg()[0]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--exe", action="append", required=True, help="SIDE=PATH, at least two")
    ap.add_argument("--rows", default="hist-ord-2m,hist-ord-8m,hist-dense-2m,hist-dense-8m")
    ap.add_argument("--rounds", type=int, default=10)
    ap.add_argument("--time-ms", type=int, default=400)
    ap.add_argument("--label", default="")
    ap.add_argument("--out", default=rp.WORK)
    args = ap.parse_args()
    exes = [tuple(x.split("=", 1)) for x in args.exe]
    rows = rp.make_rows([r for r in args.rows.split(",") if r])
    for side, exe in exes:
        if rp.run_check(exe, rows):
            sys.exit(side + ": check mismatch")
    sides = [(s, e, "pruv") for s, e in exes] + [("hand", exes[-1][1], "hand")]
    med = {}
    l0 = load()
    for rd in range(args.rounds):
        order = sides[rd % len(sides):] + sides[:rd % len(sides)]
        for name, _, path, _ in rows:
            for side, exe, impl in order:
                r = subprocess.run([exe, "time", impl, path, str(args.time_ms)], capture_output=True, text=True, env=rp.ENV)
                if r.returncode:
                    sys.exit(r.stderr[-2000:])
                med.setdefault((name, side), []).append(int(r.stdout.split()[0]))
        print("round %d/%d load %.1f" % (rd + 1, args.rounds, load()), file=sys.stderr, flush=True)
    l1 = load()
    rss = {(name, side): statistics.median([rp.max_rss([exe, "rss", impl, path]) for _ in range(3)])
           for name, _, path, _ in rows for side, exe, impl in sides}
    print("\n%s: %d alternating rounds, %d ms per sample, load %.1f at start, %.1f at end" % (args.label, args.rounds,
                                                                                                args.time_ms, l0, l1))
    print("| row | size | " + " | ".join("%s ns (x hand)" % s for s, _, _ in sides[:-1]) + " | hand ns | " +
          " | ".join("%s RSS (x hand)" % s for s, _, _ in sides[:-1]) + " | hand RSS |")
    os.makedirs(args.out, exist_ok=True)
    cells = {s: [] for s, _, _ in sides[:-1]}
    for name, size, _, _ in rows:
        h, hr = statistics.median(med[(name, "hand")]), rss[(name, "hand")]
        line = "| %s | %s |" % (name, size)
        for s, _, _ in sides[:-1]:
            p = statistics.median(med[(name, s)])
            line += " %d (%.3fx) |" % (p, p / h)
            cells[s].append({"name": "grok history %s time" % name, "axis": "time", "size": size, "kind": "proportional",
                             "pruv": p, "hand": h, "unit": "ns", "runs": args.rounds})
            cells[s].append({"name": "grok history %s rss" % name, "axis": "rss", "size": size, "kind": "proportional",
                             "pruv": rss[(name, s)], "hand": hr, "unit": "bytes", "runs": 3})
        line += " %d |" % h
        for s, _, _ in sides[:-1]:
            line += " %d (%.3fx) |" % (rss[(name, s)], rss[(name, s)] / hr)
        print(line + " %d |" % hr)
    for s, c in cells.items():
        path = os.path.join(args.out, "perf_cells_%s%s.json" % (args.label + "_" if args.label else "", s))
        json.dump({"source": "bench/l3/grok/perf/alt_perf.py", "cells": c}, open(path, "w"), indent=1)
        print("perf_cells: " + path)


if __name__ == "__main__":
    main()
