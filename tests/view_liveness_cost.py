"""R-6g9-4 cost check: the borrowed-view liveness check grows linearly with the size of the function.

Compiles one function with C else-if chains of N conditions each and 16 views that stay live across the chains,
then the same function with 2N, and fails if the compile time more than triples (a quadratic check would quadruple it).
Each size takes the best of a few runs so a loaded machine does not cause a false failure.

    .venv/bin/python tests/view_liveness_cost.py [--compiler py|rs] [--n 40] [--chains 20]
"""
import argparse
import os
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import compiler_driver as CD  # noqa: E402


def program(n, chains):
    views = 16
    out = ["// R-6g9-4 cost probe, %d chains of %d conditions, %d views" % (chains, n, views),
           "pub fn f(k: i64) -> i64 ! alloc", "{",
           "  let mut b: [u8] = alloc(4, 7);", "  let mut s: i64 = 0;"]
    out += ["  let v%d: ref [u8] = subview(b, 0, 2);" % i for i in range(views)]
    chain = "  " + " else ".join("if k == %d { s = v%d[0]; }" % (i, i % views) for i in range(n))
    out += [chain + " else { s = s + 1; }"] * chains
    out.append("  s = s + " + " + ".join("v%d[1]" % i for i in range(views)) + ";")
    out += ["  b[0] = 1;", "  return s + b[0];", "}"]
    return "\n".join(out) + "\n"


def best_time(drv, src, work, runs):
    best = None
    for _ in range(runs):
        res = drv.run(drv.argv(src, os.path.join(work, "out.rs"), ["--no-verify"], "rust"), timeout=600)
        if res.returncode != 0:
            sys.exit("compile failed for %s:\n%s%s" % (src, res.stdout[-2000:], res.stderr[-2000:]))
        best = res.elapsed if best is None else min(best, res.elapsed)
    return best


def main():
    ap = argparse.ArgumentParser()
    CD.add_args(ap)
    ap.add_argument("--n", type=int, default=40)
    ap.add_argument("--chains", type=int, default=20)
    ap.add_argument("--runs", type=int, default=3)
    ns = ap.parse_args()
    drv = CD.from_args(ns)
    with tempfile.TemporaryDirectory() as work:
        times = []
        for n in (ns.n, 2 * ns.n):
            src = os.path.join(work, "cost_%d.pruv" % n)
            with open(src, "w") as fh:
                fh.write(program(n, ns.chains))
            times.append(best_time(drv, src, work, ns.runs))
    ratio = times[1] / max(times[0], 1e-6)
    print("N=%d %.3fs, 2N=%d %.3fs, ratio %.2f" % (ns.n, times[0], 2 * ns.n, times[1], ratio))
    if ratio > 3.0:
        sys.exit("FAIL: doubling the function more than tripled the compile time (ratio %.2f)" % ratio)
    print("OK")


if __name__ == "__main__":
    main()
