"""Control for run_m7.py perf's per-cell result check (rchk): the hand16 lane is wrapped so that it prints a wrong
`rchk <item> ...` value (or none) for one item; perf must then mark that cell FAIL, drop its ratio and exit non-zero,
while the untouched cells still pass.
  python3 rchk_control.py [--item current] [--size 100]
Small run (2 rounds, 3000 reps, no instruction counts). Exit 0 only if both the mismatch and the missing case fail
exactly the tampered cell."""
import sys
sys.dont_write_bytecode = True
import argparse
import contextlib
import io
import os
import stat
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.dirname(HERE))

import run_m7  # noqa: E402

WRAP = """#!/bin/sh
"%s" "$@" | awk -v item="%s" -v mode="%s" '$1 == "rchk" && $2 == item { if (mode == "mismatch") { $4 = $4 "1"; print } ; next } { print }'
"""


def run(item, size, mode, tmp):
    real = run_m7.build_hand16()
    wrap = os.path.join(tmp, "hand16_" + mode)
    with open(wrap, "w") as f:
        f.write(WRAP % (real, item, mode))
    os.chmod(wrap, os.stat(wrap).st_mode | stat.S_IEXEC)
    orig = run_m7.build_hand16
    run_m7.build_hand16 = lambda: wrap
    buf = io.StringIO()
    try:
        ap_args = argparse.Namespace(sizes=str(size), profile=None, no_instr=True, pruvc=run_m7.PRUVC_DEFAULT, py=False,
                                     rounds=2, reps=3000)
        with contextlib.redirect_stdout(buf):
            rc = run_m7.cmd_perf(ap_args)
    finally:
        run_m7.build_hand16 = orig
    return rc, buf.getvalue()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--item", default="current")
    ap.add_argument("--size", type=int, default=100)
    a = ap.parse_args()
    want = {"mismatch": "RCHK MISMATCH", "missing": "RCHK MISSING"}
    failed = []
    with tempfile.TemporaryDirectory() as tmp:
        for mode, tag in want.items():
            rc, out = run(a.item, a.size, mode, tmp)
            fails = [l for l in out.splitlines() if l.startswith("FAIL ")]
            ok = rc != 0 and fails == ["FAIL %s %d: %s" % (a.item, a.size, tag)] and \
                "result checks: %d cells compared, 1 mismatches" % len(run_m7.PERF_ITEMS) in out
            print(("PASS " if ok else "FAIL ") + mode + ": exit %d, %s" % (rc, fails))
            if not ok:
                failed.append(mode)
                print(out[-1500:])
    print("SUMMARY %d/2 controls passed" % (2 - len(failed)))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
