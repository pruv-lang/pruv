#!/usr/bin/env python3
"""Merge preflight for one branch (TIDY 4.15).

Usage: python3 tools/merge_check.py <branch>

Run on the main checkout. Standard library only. Steps:
  1. CI: the newest completed push run on main must not have failed
     (schedule runs are printed, not judged).
  2. `git diff --stat main...<branch>` is printed.
  3. If the branch changes tests/: the row list is built from the branch tree
     (a temporary `git worktree`, the main checkout is not touched) with
     `tests/run_all.py --backend both --list-rows` and compared with the
     branch's tests/run_all_rows.txt.
  4. If the branch changes code generation: reminder lines only, nothing is run.
Exit code 1 if any step fails.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

# Files that emit Rust or LLVM output (matched by stem under pruvc/ and pruvc-rs/crates/*/src/).
CODEGEN_STEMS = {
    "codegen", "llvmgen", "llvm", "foreign_codegen", "foreign_shim", "parallel_shim",
    "fixed_prefix", "inline_policy", "small", "stack", "storage", "mono", "emit_common",
    "runtime",
}

results = []


def report(status, step, text):
    results.append(status)
    print("%-4s %s: %s" % (status, step, text))


def git(*args, cwd=None):
    return subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True)


def check_ci():
    p = subprocess.run(
        ["gh", "run", "list", "--branch", "main", "--limit", "3",
         "--json", "status,conclusion,event,displayTitle,databaseId,createdAt"],
        capture_output=True, text=True)
    if p.returncode != 0:
        report("FAIL", "ci", "gh run list failed: %s" % p.stderr.strip())
        return
    runs = json.loads(p.stdout)
    for r in runs:
        print("     %s %s %s %s %s %s" % (r["createdAt"], r["event"], r["status"],
                                        r["conclusion"] or "-", r["databaseId"],
                                        r["displayTitle"][:60]))
    push = [r for r in runs if r["event"] == "push" and r["status"] == "completed"]
    if not push:
        report("FAIL", "ci", "no completed push run on main in the last 3 runs")
        return
    newest = push[0]
    sched = [r for r in runs if r["event"] == "schedule" and r["conclusion"] not in ("success", None)]
    note = ""
    if sched:
        note = " (schedule run %s: %s, reported only)" % (sched[0]["databaseId"], sched[0]["conclusion"])
    if newest["conclusion"] == "success":
        report("PASS", "ci", "newest push run %s succeeded%s" % (newest["databaseId"], note))
    else:
        report("FAIL", "ci", "newest push run %s: %s%s" % (newest["databaseId"], newest["conclusion"], note))


def diff_stat(branch):
    p = git("diff", "--stat", "main...%s" % branch)
    if p.returncode != 0:
        report("FAIL", "diff", p.stderr.strip())
        return None
    print(p.stdout.rstrip())
    names = git("diff", "--name-only", "main...%s" % branch).stdout.split()
    report("INFO", "diff", "%d file(s) changed" % len(names))
    return names


def check_rows(branch, names):
    if not any(n.startswith("tests/") for n in names):
        report("PASS", "rows", "no change under tests/, row list not rebuilt")
        return
    tmp = tempfile.mkdtemp(prefix="merge_check_")
    tree = os.path.join(tmp, "tree")
    try:
        p = git("worktree", "add", "--detach", tree, branch)
        if p.returncode != 0:
            report("FAIL", "rows", "git worktree add failed: %s" % p.stderr.strip())
            return
        out = os.path.join(tmp, "rows.txt")
        p = subprocess.run([sys.executable, "tests/run_all.py", "--backend", "both", "--list-rows", out],
                           cwd=tree, capture_output=True, text=True, timeout=300)
        if p.returncode != 0:
            report("FAIL", "rows", "--list-rows exit %d: %s" % (p.returncode, (p.stderr or p.stdout).strip()[-300:]))
            return
        with open(out) as f:
            built = f.read().splitlines()
        with open(os.path.join(tree, "tests", "run_all_rows.txt")) as f:
            stored = f.read().splitlines()
        if built == stored:
            report("PASS", "rows", "built row list equals tests/run_all_rows.txt (%d rows)" % len(built))
        else:
            added = sorted(set(built) - set(stored))
            gone = sorted(set(stored) - set(built))
            for r in added[:10]:
                print("     + %s" % r)
            for r in gone[:10]:
                print("     - %s" % r)
            report("FAIL", "rows", "row list differs from tests/run_all_rows.txt (+%d -%d, %d built vs %d stored)"
                   % (len(added), len(gone), len(built), len(stored)))
    finally:
        git("worktree", "remove", "--force", tree)
        shutil.rmtree(tmp, ignore_errors=True)


def is_codegen(path):
    parts = path.split("/")
    stem = os.path.splitext(parts[-1])[0]
    if parts[0] == "pruvc":
        return stem in CODEGEN_STEMS or (len(parts) > 2 and parts[1] == "data")
    if parts[0] == "pruvc-rs":
        return "src" in parts and stem in CODEGEN_STEMS
    return False


def codegen_reminder(names):
    gen = [n for n in names if is_codegen(n)]
    rs = any(n.startswith("pruvc-rs/") for n in names)
    if gen:
        report("INFO", "codegen", "%d code generation file(s) changed: %s" % (len(gen), ", ".join(gen[:6])))
        print("     run --only g8_frontend on both compilers")
    if rs:
        report("INFO", "codegen", "pruvc-rs/ changed")
        print("     run cargo test --release and tests/diff_rust.py --fuzz-count 20")
    if not gen and not rs:
        report("INFO", "codegen", "no code generation change")


def main(argv):
    if len(argv) != 2:
        print("usage: python3 tools/merge_check.py <branch>")
        return 2
    branch = argv[1]
    if git("rev-parse", "--verify", "--quiet", branch).returncode != 0:
        print("FAIL branch: %s not found" % branch)
        return 1
    check_ci()
    names = diff_stat(branch)
    if names is not None:
        check_rows(branch, names)
        codegen_reminder(names)
    failed = results.count("FAIL")
    print("VERDICT: %s (%d fail)" % ("FAIL" if failed else "PASS", failed))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
