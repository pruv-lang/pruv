#!/usr/bin/env python3
"""TIDY T0 step 6: reachability regression set and the old() increment pair (docs/TIDY_SPEC.md 2 T0.2-T0.3,
review change 4). Cases are in cases.json next to this file.

  python3 tests/vacuity/run.py [--compiler py|rs|both] [--compiler-path PATH] [--timeout S]
                              [--kinds reach,contradiction,pair]

reach         The original source must verify. `assert false; ` is inserted just before the text `before` on line
              `line` (a point a concrete input reaches; the input is written in the case as `witness`). The variant
              must be rejected with stage verify, code assert, at that line and column. Any other result (accept,
              timeout, another diagnostic, another location) is a failure.
contradiction An intended dead point (contradictory requires, a branch the requires rule out, code after a return):
              the original and the variant must both verify. Listing them keeps such points visible; normal builds
              make no extra satisfiability query.
pair          Sources checked as they are: `accept` must verify, `reject` must fail with the given stage, code and
              message (the message names the failing line). Line and column are checked when given.

--compiler both (default) runs the selected sets with the Python and the Rust compiler. --kinds defaults to all
sets. Failures are counted by class for each compiler and set. Exit code: 0 all pass, 1 a case failed, 2 a tool
problem. Python 3.9, standard library only.
"""
import argparse
import json
import os
import shutil
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.dirname(HERE))
import compiler_driver as CD  # noqa: E402

INSERT = "assert false; "
KINDS = ("reach", "contradiction", "pair")
CLASSES = ("false_reject", "false_accept", "wrong_location", "timeout", "other")
ACCEPT_FAILURE = {"reject": "false_reject", "timeout": "timeout", "crash": "other"}
REJECT_FAILURE = {"ok": "false_accept", "timeout": "timeout", "crash": "other"}


def diagnostic(res):
    """First JSON object with a "stage" key on stdout or stderr, else None."""
    for line in (res.stdout + "\n" + res.stderr).splitlines():
        line = line.strip()
        if line.startswith("{"):
            try:
                d = json.loads(line)
            except ValueError:
                continue
            if isinstance(d, dict) and "stage" in d:
                return d
    return None


def compile_src(drv, path, tmp, timeout):
    res = drv.run(drv.argv(path, os.path.join(tmp, "out.rs"), (), "rust"), timeout=timeout)
    if res.timed_out:
        return "timeout", None
    if res.returncode == 0:
        return "ok", None
    d = diagnostic(res)
    return ("reject", d) if res.returncode == 1 and d is not None else ("crash", (res.stdout + res.stderr)[-400:])


def variant(case, tmp):
    """(path of the source with `assert false; ` inserted, column of the insertion) or raises ValueError."""
    src = os.path.join(ROOT, case["src"])
    with open(src, encoding="utf-8") as fh:
        lines = fh.read().split("\n")
    i = case["line"] - 1
    line = lines[i]
    if line.count(case["before"]) != 1:
        raise ValueError("line %d of %s must contain %r exactly once: %r" % (case["line"], case["src"], case["before"], line))
    col = line.index(case["before"])
    lines[i] = line[:col] + INSERT + line[col:]
    out = os.path.join(tmp, os.path.basename(src))
    with open(out, "w", encoding="utf-8") as fh:
        fh.write("\n".join(lines))
    return out, col + 1


def run_case(drv, kind, case, timeout):
    """None when the case passes, else (failure class, detail)."""
    tmp = tempfile.mkdtemp(prefix="pruv_vacuity_")
    try:
        if kind == "pair":
            got, d = compile_src(drv, os.path.join(ROOT, case["src"]), tmp, timeout)
            if case["expect"] == "accept":
                return None if got == "ok" else (ACCEPT_FAILURE[got], "expected accept, got %s %s" % (got, d))
            want = (case["stage"], case["code"])
            if got != "reject":
                cls = REJECT_FAILURE[got]
            elif (d.get("stage"), d.get("code")) != want:
                cls = "other"
            elif case["message"] not in d.get("message", ""):
                cls = "wrong_location"
            elif any(d.get(key) != case[key] for key in ("line", "col") if key in case):
                cls = "wrong_location"
            else:
                return None
            location = {key: case[key] for key in ("line", "col") if key in case}
            return cls, "expected reject %s/%s %r %s, got %s %s" % (want[0], want[1], case["message"], location, got, d)
        got, d = compile_src(drv, os.path.join(ROOT, case["src"]), tmp, timeout)
        if got != "ok":
            return ACCEPT_FAILURE[got], "original must verify, got %s %s" % (got, d)
        path, col = variant(case, tmp)
        got, d = compile_src(drv, path, tmp, timeout)
        if kind == "contradiction":
            return None if got == "ok" else (ACCEPT_FAILURE[got], "intended dead point must verify, got %s %s" % (got, d))
        if got != "reject":
            return REJECT_FAILURE[got], "expected verify/assert at %d:%d, got %s %s" % (case["line"], col, got, d)
        where = (d.get("stage"), d.get("code"), d.get("line"), d.get("col"))
        if where != ("verify", "assert", case["line"], col):
            return "wrong_location", "expected verify/assert at %d:%d, got %s/%s at %s:%s (%s)" % (
                case["line"], col, where[0], where[1], where[2], where[3], d.get("message"))
        return None
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser(description="TIDY T0 step 6 reachability set and old() pair")
    ap.add_argument("--compiler", choices=["py", "rs", "both"], default="both")
    ap.add_argument("--compiler-path", default=None, help="only with --compiler py or rs")
    ap.add_argument("--timeout", type=int, default=120, help="seconds per compile; a timeout is a failure")
    ap.add_argument("--kinds", default=",".join(KINDS), help="comma-separated sets: reach,contradiction,pair (default all)")
    args = ap.parse_args()
    if args.compiler == "both" and args.compiler_path:
        sys.exit("--compiler-path needs --compiler py or rs")
    selected = {kind.strip() for kind in args.kinds.split(",")}
    if selected.difference(KINDS):
        ap.error("--kinds must be a comma-separated list of reach,contradiction,pair")
    compilers = ["py", "rs"] if args.compiler == "both" else [args.compiler]
    drivers = [CD.make_driver(k, args.compiler_path) for k in compilers]
    with open(os.path.join(HERE, "cases.json"), encoding="utf-8") as fh:
        cases = json.load(fh)
    total = failed = 0
    tool_error = False
    for drv in drivers:
        counts = dict.fromkeys(CLASSES, 0)
        sets = []
        for kind in KINDS:
            if kind not in selected:
                continue
            set_counts = dict.fromkeys(CLASSES, 0)
            for case in cases[kind]:
                total += 1
                try:
                    err = run_case(drv, kind, case, args.timeout)
                except (OSError, ValueError) as e:
                    err = "other", str(e)
                    tool_error = True
                if err is None:
                    print("PASS %s %s/%s" % (drv.kind, kind, case["name"]))
                else:
                    failed += 1
                    cls, detail = err
                    counts[cls] += 1
                    set_counts[cls] += 1
                    print("FAIL %s %s/%s [%s]: %s" % (drv.kind, kind, case["name"], cls, detail))
            size = len(cases[kind])
            sets.append("SET %s %s pass=%d/%d %s" % (
                drv.kind, kind, size - sum(set_counts.values()), size,
                " ".join("%s=%d" % item for item in set_counts.items())))
        print("CLASSES %s %s" % (drv.kind, " ".join("%s=%d" % item for item in counts.items())))
        for line in sets:
            print(line)
    print("TOTAL %d/%d" % (total - failed, total))
    if tool_error:
        return 2
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
