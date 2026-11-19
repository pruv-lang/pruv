"""T5c: M7 typed-lane parity and emptied-body floors, retired instructions (primary) and cycles per call.

usage: python3 bench/l3/budget/m7_test/t5c_floor.py [--sizes 100,500] [--reps 20000] [--rounds 5] [--items ...] [--no-build]

Lanes (all from one process per item and lane, `perf S reps item lane`):
  pruv_t   driver_m7 typed lane (step closure, c_direct prep match; the existing lane, unchanged)
  pruv_f   driver_m7 fair typed lane (T5c, fair_loop: const-generic c_* entry, hand16's typed loop shape)
  hand     hand_m7_u16 typed lane
  pruv_t0, pruv_f0, hand0: the same lanes with the bodies emptied (floors):
    Pruv: the five typed c_* bodies (current, is_halted, attach, reopen, maybe_close) of the shipped gen.rs become
    `out[0]=0; return 0;` (the entry length guards stay); hand: hand_m7_u16.rs built with `--cfg t5c_empty`
    (current/is_halted/attach/reopen/maybe_close return at once).
Per call: (X(5 reps) - X(reps)) / (4 reps x calls), X = instructions retired / cycles elapsed of `/usr/bin/time -l`;
setup, warm-up and the untimed rchk pass (vn = min(reps, 2000), so reps >= 2000) are equal in both runs and cancel.
closereopen: one rep is a close + reopen pair, calls = 2. Rounds are interleaved: every round runs every (size, item,
lane) once, with the lane order rotated per round. Peak memory footprint of the 5 reps run is reported per lane.
Ratios (medians): raw = pruv_t/hand, fair = pruv_f/hand, floor-subtracted = (pruv_f - pruv_f0)/(hand - hand0).
Overlap: whether the [min, max] ranges over rounds of pruv_f and hand intersect.
Output: table on stdout and work/m7/t5c/floor.json. Measurement only; nothing is shipped from here."""
import argparse, json, os, re, statistics, subprocess, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import run_m7 as m7  # noqa: E402

OUT = os.path.join(m7.WORK, "t5c")
ITEMS = ["ishalted", "current", "attach", "closereopen", "reopenmiss"]
CALLS = {"closereopen": 2}
LANES = ["pruv_t", "pruv_f", "hand", "pruv_t0", "pruv_f0", "hand0"]
BODY = re.compile(r"^    return pruv_m_f_2_13_bud_lifecycle_\d+_(current|is_halted|attach|reopen|maybe_close)"
                  r"\((?:&mut \*)?st, sk, 0i64, .*\);$", re.M)


def rc(cmd):
    c = subprocess.run(cmd, capture_output=True, text=True)
    if c.returncode != 0:
        sys.exit("build failed: %s\n%s" % (" ".join(cmd[:3]), c.stderr[-4000:]))


def build(pruvc):
    os.makedirs(OUT, exist_ok=True)
    rlib = m7.build_facts()
    gen = os.path.join(OUT, "gen.rs")
    m7.compile_gen(pruvc, gen, rlib)
    src = open(gen).read()
    empty, n = BODY.subn("    out[0]=0; return 0;", src)
    assert n == 5, "expected the 5 typed c_* body lines, found %d" % n
    open(os.path.join(OUT, "gen0.rs"), "w").write(empty)
    drv = m7.driver_source()
    flags = [m7.rustc()] + m7.RUSTC_FLAGS + ["--extern", "bud_facts=" + rlib]
    for g, name in (("gen.rs", "drv"), ("gen0.rs", "drv0")):
        main = os.path.join(OUT, "main_%s.rs" % name)
        open(main, "w").write('#![allow(dead_code, unused_imports, unused_variables, unused_mut)]\n'
                              '#[path = "%s"] mod sol;\n' % g + drv)
        rc(flags + ["-o", os.path.join(OUT, name), main])
    hand = os.path.join(m7.HERE, "hand_m7_u16.rs")
    rc(flags + ["-o", os.path.join(OUT, "hand"), hand])
    rc(flags + ["--cfg", "t5c_empty", "-o", os.path.join(OUT, "hand0"), hand])


def cmd(lane, size, reps, item):
    base, empty = lane.rstrip("0"), lane.endswith("0")
    if base == "hand":
        return [os.path.join(OUT, "hand0" if empty else "hand"), "perf", str(size), str(reps), item, "typed"]
    return [os.path.join(OUT, "drv0" if empty else "drv"), "perf", str(size), str(reps), item, base[-1]]


def run(c):
    r = subprocess.run(["/usr/bin/time", "-l"] + c, capture_output=True, text=True, cwd=m7.HERE)
    if r.returncode != 0:
        sys.exit("run failed: %s\n%s" % (" ".join(c), r.stderr[-2000:]))
    g = lambda k: int(re.search(r"(\d+)\s+" + k, r.stderr).group(1))
    return g("instructions retired"), g("cycles elapsed"), g("peak memory footprint")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--sizes", default="100,500")
    ap.add_argument("--reps", type=int, default=20000)
    ap.add_argument("--rounds", type=int, default=5)
    ap.add_argument("--items", nargs="*", default=ITEMS)
    ap.add_argument("--pruvc", default=m7.PRUVC_DEFAULT)
    ap.add_argument("--no-build", action="store_true")
    a = ap.parse_args()
    assert a.reps >= 2000, "reps < 2000: the rchk pass would not cancel"
    if not a.no_build:
        build(a.pruvc)
    sizes = [int(s) for s in a.sizes.split(",")]
    data = {}  # (item, size, lane) -> {"i": [], "c": [], "m": []}
    for rd in range(a.rounds):
        order = LANES[rd % len(LANES):] + LANES[:rd % len(LANES)]
        for size in sizes:
            for item in a.items:
                k = 4 * a.reps * CALLS.get(item, 1)
                for lane in order:
                    x1 = run(cmd(lane, size, a.reps, item))
                    x5 = run(cmd(lane, size, 5 * a.reps, item))
                    d = data.setdefault((item, size, lane), {"i": [], "c": [], "m": []})
                    d["i"].append((x5[0] - x1[0]) / k)
                    d["c"].append((x5[1] - x1[1]) / k)
                    d["m"].append(x5[2])
        print("round %d done" % (rd + 1), file=sys.stderr, flush=True)
    med = lambda v: statistics.median(v)
    ov = lambda p, h: "yes" if min(p) <= max(h) and min(h) <= max(p) else "no"
    rows = []
    for metric, label in (("i", "instructions"), ("c", "cycles")):
        print("\n%s per call: median [min..max] over %d interleaved rounds, reps %d" % (label, a.rounds, a.reps))
        print("item | S | " + " | ".join(LANES) + " | raw t/hand | fair f/hand | floor-sub (f-f0)/(hand-hand0)"
              " | floor-sub (t-t0)/(hand-hand0) | f vs hand overlap")
        for size in sizes:
            for item in a.items:
                d = {ln: data[(item, size, ln)][metric] for ln in LANES}
                m = {ln: med(v) for ln, v in d.items()}
                hs = m["hand"] - m["hand0"]
                row = {"item": item, "S": size, "metric": label,
                       "lanes": {ln: {"median": m[ln], "min": min(d[ln]), "max": max(d[ln]), "all": d[ln]} for ln in LANES},
                       "raw": m["pruv_t"] / m["hand"], "fair": m["pruv_f"] / m["hand"],
                       "floor_sub_fair": (m["pruv_f"] - m["pruv_f0"]) / hs if hs > 0 else None,
                       "floor_sub_typed": (m["pruv_t"] - m["pruv_t0"]) / hs if hs > 0 else None,
                       "overlap_fair_hand": ov(d["pruv_f"], d["hand"]),
                       "peak_mem": {ln: med(data[(item, size, ln)]["m"]) for ln in LANES}}
                rows.append(row)
                fs = lambda x: "-" if x is None else "%.2f" % x
                print("%s | %d | %s | %s | %s | %s | %s | %s" % (
                    item, size, " | ".join("%.1f [%.1f..%.1f]" % (m[ln], min(d[ln]), max(d[ln])) for ln in LANES),
                    fs(row["raw"]), fs(row["fair"]), fs(row["floor_sub_fair"]), fs(row["floor_sub_typed"]),
                    row["overlap_fair_hand"]))
    print("\npeak memory footprint (bytes, 5 reps run, median)")
    print("item | S | " + " | ".join(LANES))
    for r in rows:
        if r["metric"] == "instructions":
            print("%s | %d | %s" % (r["item"], r["S"], " | ".join("%d" % r["peak_mem"][ln] for ln in LANES)))
    json.dump({"reps": a.reps, "rounds": a.rounds, "sizes": sizes, "lanes": LANES, "rows": rows},
              open(os.path.join(OUT, "floor.json"), "w"), indent=1)


if __name__ == "__main__":
    main()
