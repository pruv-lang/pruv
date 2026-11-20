"""Checks that generated shared-table files are up to date and that every pruvc/data/*.json is generated, and that
the Python inline policy accepts and rejects at the spec thresholds (INLINE_SMALL 8/9, INLINE_BUDGET 1500/1501), so a
wrong value in pruvc/data/inline_policy.json is caught (pruvc-rs has the same pairs in inline_policy.rs tests).

python tests/check_generated.py
(--only-py, --only-rs, --py PATH, --rs PATH, --work DIR are accepted and ignored: run_all passes them.)
"""
import glob
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "tools"))
sys.path.insert(0, os.path.join(ROOT, "pruvc"))

import gen_shared  # noqa: E402
from inline_policy import inline_policy  # noqa: E402


def inline_case(lines, sites):
    """one #[inline] fn of `lines` body lines, called `sites` times from a plain fn; True when it gets always"""
    body = "".join("    let _ = x + %d;\n" % i for i in range(lines - 1)) + "    x\n"
    text = "#[inline]\nfn f(x: i64) -> i64 {\n" + body + "}\nfn main() {\n" + "    f(0);\n" * sites + "}\n"
    return inline_policy(text).startswith("#[inline(always)]\n")


# (body lines, call sites, expected always): small 8 accepts, 9 rejects (incl * (sites - 1) far above the budget);
# budget 1500 accepts, 1501 rejects (two sites, so the added copies equal the body size)
# zero call sites (a host entry called only from the harness): 8 lines accepts by INLINE_SMALL, 9 keeps #[inline]
INLINE_PAIRS = [(8, 300, True), (9, 300, False), (1500, 2, True), (1501, 2, False), (8, 0, True), (9, 0, False), (9, 1, True)]


def inline_fp_case(sites, triple=True):
    """P18d: a by-value twin `f__pruv_fp` (21 body lines, one textual call from its pinned adapter `f`) takes the call
    sites of `f`; True when the twin gets always (pruvc-rs inline_policy_fp_uses_base_sites has the same pair).
    triple=False drops the generated body `f__pruv_fpb`: the name is then an ordinary fn with its own single call site."""
    body = "".join("    let _ = x + %d;\n" % i for i in range(20)) + "    x\n"
    text = ("#[inline(always)] // pruv: explicit\nfn f(x: i64) -> i64 {\n    f__pruv_fp(x)\n}\n#[inline]\n"
            "fn f__pruv_fp(x: i64) -> i64 {\n" + body + "}\n"
            + ("#[inline(always)] // pruv: explicit\nfn f__pruv_fpb(x: i64) -> i64 {\n    x\n}\n" if triple else "") + "fn main() {\n" + "    f(0);\n" * sites + "}\n")
    return "#[inline(always)]\nfn f__pruv_fp(" in inline_policy(text)


def main():
    bad = False
    for path in sorted(glob.glob(os.path.join(ROOT, "pruvc", "data", "*.json"))):
        name = os.path.splitext(os.path.basename(path))[0]
        if name not in gen_shared.TABLES:
            print("not in tools/gen_shared.py TABLES: pruvc/data/%s.json" % name)
            bad = True
    out = os.path.join(ROOT, gen_shared.OUT)
    try:
        with open(out, "rb") as f:
            current = f.read()
    except OSError:
        current = None
    if current != gen_shared.render().encode("utf-8"):
        print("stale: %s; run tools/gen_shared.py" % gen_shared.OUT)
        bad = True
    for lines, sites, want in INLINE_PAIRS:
        if inline_case(lines, sites) != want:
            print("inline policy: %d lines, %d sites: want %s" % (lines, sites, "always" if want else "inline"))
            bad = True
    for sites, triple, want in ((300, True, False), (1, True, True), (300, False, True)):
        if inline_fp_case(sites, triple) != want:
            print("inline policy: __pruv_fp twin (triple %s), %d base sites: want %s"
                  % (triple, sites, "always" if want else "inline"))
            bad = True
    if bad:
        return 1
    print("check_generated: ok (%d tables)" % len(gen_shared.TABLES))
    return 0


if __name__ == "__main__":
    sys.exit(main())
