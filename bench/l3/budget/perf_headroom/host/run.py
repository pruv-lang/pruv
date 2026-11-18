"""Perf headroom, part A: Pruv / hand-written Rust / TS for the budget piece's resident-state calls (M4) and host
store items (M5), run alternately with medians (results: ../HOST.md).

  python3 perf_headroom/host/run.py [--rounds 5] [--pruvc PATH] [--only m4|m5] [--no-build]

Run from bench/l3/budget. Builds:
  - the hand baselines (cargo, this folder) into work/handbuild/release/{m4_hand,m5_hand};
  - the M4 driver via m34_test/run.py build() (pruvc root_m4.pruv + driver_m4.rs) into work/m34/;
  - the M5 harness via dev_build.sh into work/m5build/harness (PRUVC=--pruvc).
Each round runs, per (item, n), the three sides in a rotating order (Pruv, hand, TS / hand, TS, Pruv / ...), so load
drift hits all three alike; the table reports the median over rounds of each side's per-run figure.
  M4 (per op; 200000 reps = 400000 ops for sessions/clocks, 600000 for tools): Pruv = driver_m4 total_ns_per_op,
  hand = m4_hand hand_ns_per_op, TS = perf_ts.mjs ts_ns_per_op. Every Rust side uses the same sink (out[0] + out[1])
  and the same accounting: total loop time, grow time included (no subtraction).
  M4 adapter table: matched baselines. The C-boundary lane (m4c) is compared with the hand Rust State behind the same
  kind of extern "C" handle (`m4_hand perfc`, "handc": black_box'd function pointers, handle/poison/ret/overlap/slice
  checks and a poison set/clear per call); the in-process lanes are compared with the in-process hand Rust (`m4_hand
  perf`). m4c/hand (unmatched) is printed for information only.
  M5 (per op, min over --m5-reps after a warm-up, per run): Pruv = harness perf, hand = m5_hand perf, TS = m5_test/perf_ts.mjs.
"""
import sys
sys.dont_write_bytecode = True
import argparse
import json
import os
import shutil
import statistics
import subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
BUD = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.join(BUD, "m34_test"))
import run as m34  # noqa: E402

HAND = os.path.join(BUD, "work", "handbuild")
HARNESS = os.path.join(BUD, "work", "m5build", "harness")
SIZES = [100, 500, 1000]
EXTRA_LANES = ["m4c", "m4eq", "m4llvm", "m4llvmraw", "m4nv", "m4llvmnv"]   # P10 lanes, built by m34_test/run.py (--lanes picks)


def sh(cmd, env=None, cwd=BUD):
    c = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, env=env)
    if c.returncode != 0:
        sys.exit("failed (%d): %s\n%s%s" % (c.returncode, " ".join(cmd), c.stdout[-2000:], c.stderr[-2000:]))
    return c.stdout


def kv(line):
    return dict(t.split("=", 1) for t in line.split() if "=" in t)


def build(args):
    env = dict(os.environ, CARGO_TARGET_DIR=HAND)
    env["PATH"] = os.path.expanduser("~/.cargo/bin") + os.pathsep + env.get("PATH", "")
    sh(["cargo", "build", "--release", "--offline", "--quiet"], env=env, cwd=HERE)
    for exe in ("m4_hand",):
        out = sh([os.path.join(HAND, "release", exe), "selftest"])
        if "selftest ok" not in out:
            sys.exit("%s selftest failed:\n%s" % (exe, out))
    out = sh(["node", os.path.join(BUD, "m34_test", "perf_ts.mjs"), "selftest"])
    if "selftest ok" not in out:
        sys.exit("perf_ts.mjs selftest failed:\n%s" % out)
    exe4 = m34.build(args.pruvc, "m4")
    # P09/P10 poison lanes: m4raw, then the LLVM/C/no-valid lanes. Any requested lane that fails to build fails the run.
    args.lane_exes = {"m4": exe4}
    if args.only in (None, "m4"):
        args.lane_exes["m4raw"] = m34.build(args.pruvc, "m4raw")
        for tag in EXTRA_LANES:
            if tag not in args.lanes:
                continue
            args.lane_exes[tag] = m34.build(args.pruvc, tag)   # a requested lane that fails to build fails the run
    if args.only == "m4" and args.pruvc.endswith(".py"):
        print("note: dev_build.sh (M5 harness) skipped: it runs PRUVC directly and %s is a Python script" % args.pruvc,
              file=sys.stderr)
    elif not args.no_build:
        sh(["sh", os.path.join(BUD, "dev_build.sh")], env=dict(env, PRUVC=args.pruvc))
    return exe4


def rotate(xs, r):
    r %= len(xs)
    return xs[r:] + xs[:r]


def m4(args, exe4):
    reps = 200000
    vals = {}  # (axis, n, side) -> [ns]
    for r in range(args.rounds):
        for axis in ("sessions", "tools", "clocks"):
            for n in SIZES:
                sides = [
                    ("pruv", [exe4, "perf", axis, str(n), str(reps)], "total_ns_per_op"),
                    ("hand", [os.path.join(HAND, "release", "m4_hand"), "perf", axis, str(n), str(reps)], "hand_ns_per_op"),
                    ("ts", ["node", os.path.join(BUD, "m34_test", "perf_ts.mjs"), axis, str(n), str(reps)], "ts_ns_per_op"),
                ]
                for side, cmd, key in rotate(sides, r):
                    v = float(kv(sh(cmd).strip().splitlines()[-1])[key])
                    vals.setdefault((axis, n, side), []).append(v)
    print("### M4 resident-state calls (ns per op, median of %d alternating runs, 200000 reps each)" % args.rounds)
    print("| axis | n | Pruv | hand Rust | TS | Pruv/hand | Pruv/TS |")
    print("|---|---:|---:|---:|---:|---:|---:|")
    for axis in ("sessions", "tools", "clocks"):
        for n in SIZES:
            p, h, t = (statistics.median(vals[(axis, n, s)]) for s in ("pruv", "hand", "ts"))
            print("| %s | %d | %.1f | %.1f | %.1f | %.2f | %.2f |" % (axis, n, p, h, t, p / h, p / t))
    print()


def m4_poison(args, exe4):
    """P09/P10 adapter lanes with matched baselines. Sides: hand (m4_hand perf, hand Rust in-process), handc (m4_hand
    perfc, the same hand Rust State behind an extern "C" handle: the C-boundary baseline), pruv m4 (Plain store, no
    poison), m4raw (pruv_raw::PruvStore), m4c (LLVM + raw_entries "c", extern "C" calls) and m4llvm / m4llvmraw / m4nv /
    m4llvmnv (any requested lane that fails to build fails the run).
    Matching: the C-boundary lane m4c is compared with handc (MATCHED: both pay handle/poison/ret/overlap/slice checks
    and an opaque call); every in-process lane is compared with the in-process hand Rust. m4c/hand is printed for
    information only (UNMATCHED). All sides use the identical sink (out[0] + out[1]) and identical accounting: total
    loop time with grows included (driver_m4 total_ns_per_op, not core_ns_per_op).
    Separate rounds from the lanes above; the sides rotate order each round; >= 7 rounds."""
    reps = 200000
    rounds = max(args.rounds, 7)
    hand = os.path.join(HAND, "release", "m4_hand")
    lanes = [s for s in ["m4", "m4raw"] + EXTRA_LANES if s in args.lane_exes]
    vals = {}  # (axis, n, side) -> [ns]
    for r in range(rounds):
        for axis in ("sessions", "tools", "clocks"):
            for n in SIZES:
                sides = [(s, [args.lane_exes[s], "perf", axis, str(n), str(reps)], "total_ns_per_op") for s in lanes]
                sides.append(("hand", [hand, "perf", axis, str(n), str(reps)], "hand_ns_per_op"))
                sides.append(("handc", [hand, "perfc", axis, str(n), str(reps)], "hand_ns_per_op"))
                for side, cmd, key in rotate(sides, r):
                    v = float(kv(sh(cmd).strip().splitlines()[-1])[key])
                    vals.setdefault((axis, n, side), []).append(v)
    print("### M4 adapter lanes (ns per op, total time incl. grows, sink out[0]+out[1]; median of %d alternating runs, 200000 reps each; [min-max] = spread over rounds)" % rounds)
    ratio_cols = []
    for s in lanes:
        if s == "m4c":
            ratio_cols.append("m4c/handc (MATCHED)")
            ratio_cols.append("m4c/hand (unmatched, info)")
        else:
            ratio_cols.append("%s/hand (in-process, matched)" % s)
    print("| axis | n | hand Rust (in-process) | hand C (handc: extern C handle) | " + " | ".join(lanes) + " | "
          + " | ".join(ratio_cols) + " | m4raw/m4 |")
    print("|---|---:|---:|---:|" + "---:|" * (len(lanes) + len(ratio_cols) + 1))
    for axis in ("sessions", "tools", "clocks"):
        for n in SIZES:
            cells = []
            med = {}
            for s in ["hand", "handc"] + lanes:
                v = vals[(axis, n, s)]
                med[s] = statistics.median(v)
                cells.append("%.1f [%.1f-%.1f]" % (med[s], min(v), max(v)))
            ratios = []
            for s in lanes:
                if s == "m4c":
                    ratios.append("%.2f" % (med[s] / med["handc"]))
                    ratios.append("%.2f" % (med[s] / med["hand"]))
                else:
                    ratios.append("%.2f" % (med[s] / med["hand"]))
            poison = "%.3f" % (med["m4raw"] / med["m4"]) if "m4raw" in med else "n/a"
            print("| %s | %d | %s | %s |" % (axis, n, " | ".join(cells), " | ".join(ratios + [poison])))
    print()


def jlines(out):
    return [json.loads(l) for l in out.splitlines() if l.startswith("{")]


def m5(args):
    vals = {}  # (side, measure, n) -> [ns]
    for r in range(args.rounds):
        for n in SIZES:
            sides = [
                ("pruv", [HARNESS, "perf", str(n), str(args.m5_reps)]),
                ("hand", [os.path.join(HAND, "release", "m5_hand"), "perf", str(n), str(args.m5_reps)]),
                ("ts", ["node", os.path.join(BUD, "m5_test", "perf_ts.mjs"), str(n), str(args.m5_reps)]),
            ]
            for side, cmd in rotate(sides, r):
                for o in jlines(sh(cmd)):
                    v = o.get("ns_per_op", o.get("ns"))
                    vals.setdefault((side, o["measure"], o["n"]), []).append(float(v))
    med = {k: statistics.median(v) for k, v in vals.items()}
    print("### M5 host store items (ns per op, median of %d alternating runs; each run = min of %d reps)" % (args.rounds, args.m5_reps))
    print("| side | measure | " + " | ".join("n=%d" % n for n in SIZES) + " |")
    print("|---|---|" + "---:|" * len(SIZES))
    for side in ("pruv", "hand", "ts"):
        for m in sorted({k[1] for k in med if k[0] == side}):
            print("| %s | %s | " % (side, m) + " | ".join("%.1f" % med.get((side, m, n), float("nan")) for n in SIZES) + " |")
    print()
    pairs = [("store_find", "store_find_text"), ("store_find_enc", "store_find_enc_text"),
             ("store_find_lookup", "store_find_core_u16"), ("store_find_lookup", "store_find_core"), ("store_save", "store_save")]
    print("| Pruv measure / hand measure | " + " | ".join("n=%d" % n for n in SIZES) + " |")
    print("|---|" + "---:|" * len(SIZES))
    for a, b in pairs:
        print("| %s / %s | " % (a, b) + " | ".join("%.2f" % (med[("pruv", a, n)] / med[("hand", b, n)]) for n in SIZES) + " |")
    print()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rounds", type=int, default=5)
    ap.add_argument("--pruvc", default=m34.PRUVC_DEFAULT)
    ap.add_argument("--only", choices=["m4", "m5"])
    ap.add_argument("--lanes", default=",".join(EXTRA_LANES),
                    help="m4 poison table: extra lanes to build and measure (default all: %s)" % ",".join(EXTRA_LANES))
    ap.add_argument("--m5-reps", type=int, default=5)
    ap.add_argument("--no-build", action="store_true", help="skip dev_build.sh (M5 harness already built)")
    args = ap.parse_args()
    args.lanes = [s for s in args.lanes.split(",") if s]
    if not shutil.which("node"):
        sys.exit("node not found")
    exe4 = build(args)
    if args.only in (None, "m4"):
        m4(args, exe4)
        m4_poison(args, exe4)
    if args.only in (None, "m5"):
        m5(args)


if __name__ == "__main__":
    main()
