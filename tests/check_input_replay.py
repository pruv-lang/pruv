#!/usr/bin/env python3
"""Self-test of the multi-file `--input` save + replay in tests/diff_compilers.py.

Covers row PF-3 of docs/RUST_PARITY_SPEC.md: an `--input FILE` whose source declares `mod x;` must be saved as a project
(whole directory copied, project_root = root basename), so `--replay CASE_DIR` can still find the sibling modules.

Steps (all in a temporary directory that is removed at the end):
  1. golden-capture examples/g4_geometry/root.pruv with --input --update-golden (one "input|" golden line expected)
  2. corrupt the digest in that golden, run again: expect rc 1 and exactly one saved case with project_root == "root.pruv"
     and project/geo.pruv present
  3. --replay the saved case: expect rc 1 (golden still corrupted), "  b: exit 0," (the replayed compile succeeds) and a
     "golden text digest: X (golden: Y)" line with X != Y

Usage: python tests/check_input_replay.py [--b-path PATH] [--timeout SEC]
Prints "PASS input_replay" and exits 0, or "FAIL: <reason>" plus the output tail and exits 1.
"""
import argparse
import glob
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
DIFF = os.path.join(HERE, "diff_compilers.py")
INPUT = "examples/g4_geometry/root.pruv"


def tail(text, n=40):
    return "\n".join(text.splitlines()[-n:])


def fail(reason, output=""):
    print("FAIL: " + reason)
    if output:
        print(tail(output))
    return 1


def run(cmd, timeout):
    env = dict(os.environ)
    env["PYTHONIOENCODING"] = "utf-8"
    env["PYTHONUTF8"] = "1"
    try:
        p = subprocess.run(cmd, cwd=ROOT, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=timeout)
    except subprocess.TimeoutExpired as e:
        return None, (e.stdout or b"").decode("utf-8", errors="replace") + "\n[timeout]"
    return p.returncode, p.stdout.decode("utf-8", errors="replace")


def main():
    exe = ".exe" if os.name == "nt" else ""
    ap = argparse.ArgumentParser(description="self-test of multi-file --input save + replay (row PF-3)")
    ap.add_argument("--b-path", default=os.environ.get("PRUVC_RS") or os.path.join(ROOT, "pruvc-rs", "target", "release", "pruvc" + exe))
    ap.add_argument("--timeout", type=int, default=300)
    args = ap.parse_args()
    b = os.path.abspath(args.b_path)
    tmp = tempfile.mkdtemp(prefix="input_replay_")
    try:
        good = os.path.join(tmp, "good.txt")
        bad = os.path.join(tmp, "bad.txt")
        save = os.path.join(tmp, "save")
        base = [sys.executable, DIFF, "--b", "rs", "--b-path", b, "--backend", "rust", "--no-default-inputs",
                "--input", INPUT]
        # step 1: capture the golden
        rc, out = run(base + ["--golden", good, "--update-golden"], args.timeout)
        if rc != 0:
            return fail("step 1: golden capture exit %s, expected 0" % rc, out)
        try:
            with open(good, encoding="utf-8") as fh:
                lines = fh.read().splitlines()
        except OSError as e:
            return fail("step 1: golden file not written: %r" % e, out)
        data = [ln for ln in lines if ln.strip() and not ln.startswith("#")]
        if len(data) != 1 or not data[0].startswith("input|"):
            return fail("step 1: expected exactly one 'input|' golden line, got %r" % data, "\n".join(lines))
        # step 2: corrupt every digest after "=" on that line, then run in check mode
        corrupted, n = re.subn(r"=[0-9a-f]{12}", "=000000000000", data[0])
        if n == 0:
            return fail("step 2: no '=<12 hex>' digest in golden line: %r" % data[0])
        new_lines = [corrupted if ln == data[0] else ln for ln in lines]
        with open(bad, "w", encoding="utf-8", newline="\n") as fh:
            fh.write("\n".join(new_lines) + "\n")
        rc, out = run(base + ["--golden", bad, "--save-dir", save], args.timeout)
        if rc != 1:
            return fail("step 2: check against corrupted golden exit %s, expected 1" % rc, out)
        metas = glob.glob(os.path.join(save, "*", "case.json"))
        if len(metas) != 1:
            return fail("step 2: expected exactly one saved case, found %d" % len(metas), out)
        case_dir = os.path.dirname(metas[0])
        try:
            with open(metas[0], encoding="utf-8") as fh:
                meta = json.load(fh)
        except (OSError, ValueError) as e:
            return fail("step 2: cannot read case.json: %r" % e)
        if meta.get("project_root") != "root.pruv":
            return fail("step 2: project_root is %r, expected 'root.pruv'" % meta.get("project_root"), out)
        if not os.path.isfile(os.path.join(case_dir, "project", "geo.pruv")):
            return fail("step 2: project/geo.pruv missing in saved case %s" % case_dir, out)
        # step 3: replay
        rc, out = run([sys.executable, DIFF, "--b-path", b, "--replay", case_dir], args.timeout)
        if rc != 1:
            return fail("step 3: replay exit %s, expected 1" % rc, out)
        if not any(ln.startswith("  b: exit 0,") for ln in out.splitlines()):
            return fail("step 3: replayed compile did not succeed (no '  b: exit 0,' line)", out)
        m = re.search(r"^golden text digest: (\S+) \(golden: (\S+)\)", out, re.M)
        if not m:
            return fail("step 3: no 'golden text digest: X (golden: Y)' line", out)
        if m.group(1) == m.group(2):
            return fail("step 3: replayed digest equals the corrupted golden digest", out)
        print("PASS input_replay")
        return 0
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
