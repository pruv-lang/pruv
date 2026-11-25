"""G4b-Q fault injection: mutants of the package resolver and of the proof cache, on both compilers.

Each fault is a set of exact string replacements applied alone to a disposable copy of `pruvc/` (py) or `pruvc-rs/`
(rs); the source tree is never edited. A fault is caught when the probe passes on the clean copy (baseline) and fails on
the faulty copy. A build failure or a probe timeout is "not started": it is reported as an error and never counts as
caught. Copies and outputs live under bench/g4b/.work; a unified diff per fault and compiler is written next to this
file as `<fault>.<py|rs>.diff`.

usage: check.py [--compiler py|rs|both] [--fault NAME]... [--keep]
"""
import sys
sys.dont_write_bytecode = True
import argparse
import difflib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
WORK = ROOT / "bench" / "g4b" / ".work"
TARGET = WORK / "rs-target"
PY_TIMEOUT = 1800
BUILD_TIMEOUT = 3600

MANIFEST_PY = "pruvc/manifest.py"
MANIFEST_RS = "pruvc-rs/crates/pruv-core/src/manifest.rs"
MODULES_PY = "pruvc/modules.py"
MODULES_RS = "pruvc-rs/crates/pruv-core/src/modules.rs"
VERIFY_PY = "pruvc/verify.py"
OBLIG_RS = "pruvc-rs/crates/pruv-proof/src/obligation.rs"
CACHE_PY = "pruvc/proof_cache.py"

UNUSED_LEGACY = {
    "py": (MODULES_PY, [("if graph is not None and graph.program.deps:", "if False:")]),
    "rs": (MODULES_RS, [("if let Some((key, _, _)) = g.program().deps.first() {",
                         "if let Some((key, _, _)) = g.program().deps.get(usize::MAX) {")]),
}
UNUSED_ROOT = {
    "py": (MODULES_PY, [("if key not in seen:", "if False:")]),
    "rs": (MODULES_RS, [("if !seen.contains(key) {", "if false {")]),
}

# name -> {compiler: (file relative to the repository root, [(old, new), ...])}
FAULTS = {
    "cycle": {"py": (MANIFEST_PY, [("if s.dir == ddir:", "if False:")]),
              "rs": (MANIFEST_RS, [("if g.pkgs[s].dir == ddir {", "if false {")])},
    "identity": {"py": (MANIFEST_PY, [("if o.name == d.name:", "if False:")]),
                 "rs": (MANIFEST_RS, [("if o.name == d.name {", "if false {")])},
    "version_line": {"py": (MANIFEST_PY, [("_line(found) == _line(_pad(r[2])) and found >= _pad(r[2])",
                                           "found >= _pad(r[2])")]),
                     "rs": (MANIFEST_RS, [("line(&found) == line(&want) && found >= want", "found >= want")])},
    "unused": {c: (UNUSED_ROOT[c][0], UNUSED_ROOT[c][1] + UNUSED_LEGACY[c][1]) for c in ("py", "rs")},
    "unused_legacy": UNUSED_LEGACY,
    "store_sat": {"py": (VERIFY_PY, [("if cache is not None and r == z3.unsat:",
                                       "if cache is not None and r in (z3.unsat, z3.sat):")]),
                  "rs": (OBLIG_RS, [("if result == Sat::Unsat {", "if [Sat::Unsat, Sat::Sat].contains(&result) {")])},
    "drop_options": {"py": (CACHE_PY, [("out = [head]", "out = []")]),
                     "rs": (OBLIG_RS, [("let mut out = head.to_string();", "let mut out = String::new();")])},
}

# probe: which test script, extra arguments (cache cases are named so a probe takes minutes, not the whole file)
PROBES = {
    "cycle": ("pkg", []), "identity": ("pkg", []), "version_line": ("pkg", []),
    "unused": ("pkg", []), "unused_legacy": ("pkg", []),
    "store_sat": ("cache", ["--case", "case_never_stored", "--case", "case_keys"]),
    "drop_options": ("cache", ["--case", "case_never_stored", "--case", "case_keys"]),
}


def run(cmd, timeout, env=None, cwd=None):
    """-> (returncode or None on timeout, combined output)"""
    try:
        c = subprocess.run([str(x) for x in cmd], capture_output=True, text=True, timeout=timeout, env=env, cwd=cwd)
    except subprocess.TimeoutExpired as e:
        out = (e.stdout or b"")
        out = out.decode("utf-8", "replace") if isinstance(out, bytes) else out
        return None, out + "\nTIMEOUT"
    return c.returncode, c.stdout + c.stderr


def failing_names(out):
    """names of the failing checks from the probe output: `FAIL name: detail` lines"""
    names = []
    for line in out.splitlines():
        m = re.match(r"FAIL (.*?)(?:: |$)", line)
        if m:
            names.append(m.group(1))
    return names


def copy_tree(kind, dest):
    if kind == "py":
        shutil.copytree(ROOT / "pruvc", dest / "pruvc", ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
    else:
        shutil.copytree(ROOT / "pruvc-rs", dest / "pruvc-rs", ignore=shutil.ignore_patterns("target"))
        # pruv-core embeds stdlib/std.pruv (include_bytes!) and pruvc/build.rs compiles pruvc/runtime.c,
        # both read relative to the workspace, so the copy carries them too.
        shutil.copytree(ROOT / "stdlib", dest / "stdlib")
        (dest / "pruvc").mkdir()
        shutil.copy2(ROOT / "pruvc" / "runtime.c", dest / "pruvc" / "runtime.c")


def apply_fault(fault, kind, dest):
    """apply the replacements of FAULT to the copy under DEST; write the diff next to this file"""
    rel, edits = FAULTS[fault][kind]
    path = dest / rel
    original = path.read_text()
    altered = original
    for old, new in edits:
        assert altered.count(old) == 1, (fault, kind, old, altered.count(old))
        altered = altered.replace(old, new)
    path.write_text(altered)
    patch = "".join(difflib.unified_diff(original.splitlines(True), altered.splitlines(True),
                                         fromfile="a/" + rel, tofile="b/" + rel))
    (HERE / ("%s.%s.diff" % (fault, kind))).write_text(patch)


def build_rs(src, log):
    """build the Rust copy SRC into the shared target dir; -> (ok, output)"""
    env = dict(os.environ, CARGO_TARGET_DIR=str(TARGET))
    rc, out = run(["cargo", "build", "--release", "--bins"], BUILD_TIMEOUT, env=env, cwd=src / "pruvc-rs")
    log.write_text(out)
    return rc == 0, out[-600:]


def probe(fault, kind, tree, tmp):
    """run the probe of FAULT on the compiler copy in TREE -> dict(started, passed, failing, output)"""
    script, extra = PROBES[fault]
    cmd = [sys.executable, ROOT / "tests" / "g4b" / ("check_%s.py" % script), tmp]
    if script == "pkg":
        cmd += ["--rs", TARGET / "release" / "pruvc"] if kind == "rs" else ["--py", tree / "pruvc" / "pruvc.py"]
    else:
        cmd += ["--only", kind] + (["--vs-py"] if kind == "py" else [])   # PF: py is opt-in behind --vs-py
        cmd += ["--rs", TARGET / "release" / "pruvc"] if kind == "rs" else ["--py", tree / "pruvc" / "pruvc.py"]
        cmd += extra
    rc, out = run(cmd, PY_TIMEOUT)
    # rc 0: all passed; rc 1: some check failed (a normal result); anything else (usage error, timeout): not started
    started = rc in (0, 1)
    return {"started": started, "passed": rc == 0, "rc": rc, "failing": failing_names(out), "output": out[-1500:]}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--compiler", choices=("py", "rs", "both"), default="both")
    ap.add_argument("--fault", action="append", choices=sorted(FAULTS))
    ap.add_argument("--keep", action="store_true", help="keep the temporary copies under bench/g4b/.work")
    a = ap.parse_args()
    faults = a.fault or list(FAULTS)
    kinds = ["py", "rs"] if a.compiler == "both" else [a.compiler]
    WORK.mkdir(parents=True, exist_ok=True)
    rows, errors = [], []
    base = {}  # (kind, probe script) -> baseline result; the clean copy is built and probed once per kind

    def baseline(kind, script_fault, clean):
        script = PROBES[script_fault][0]
        key = (kind, script_fault if script == "cache" else script)
        if key not in base:
            with tempfile.TemporaryDirectory(prefix="base_%s_" % kind, dir=WORK) as tmp:
                base[key] = probe(script_fault, kind, clean, tmp)
        return base[key]

    for kind in kinds:
        clean_root = Path(tempfile.mkdtemp(prefix="clean_%s_" % kind, dir=WORK))
        copy_tree(kind, clean_root)
        built, build_out = True, ""
        if kind == "rs":
            built, build_out = build_rs(clean_root, WORK / "build_clean.log")
            if not built:
                errors.append("clean rs build failed: " + build_out)
        if built:
            # probe the clean copy for every requested fault now: later faulty rs builds overwrite the shared binary
            for fault in faults:
                baseline(kind, fault, clean_root)
        for fault in faults:
            row = {"fault": fault, "compiler": kind, "baseline": None, "faulty": None, "caught": False, "failing": [],
                   "error": None}
            rows.append(row)
            if not built:
                row["error"] = "clean build failed"
                continue
            base_res = baseline(kind, fault, clean_root)
            row["baseline"] = base_res["passed"] if base_res["started"] else None
            if not base_res["started"] or not base_res["passed"]:
                row["error"] = "baseline did not pass: rc=%s %s" % (base_res["rc"], base_res["output"][-300:])
                errors.append("%s/%s: %s" % (fault, kind, row["error"]))
                continue
            fdir = Path(tempfile.mkdtemp(prefix="%s_%s_" % (fault, kind), dir=WORK))
            try:
                copy_tree(kind, fdir)
                apply_fault(fault, kind, fdir)
                if kind == "rs":
                    ok, out = build_rs(fdir, WORK / ("build_%s.log" % fault))
                    if not ok:
                        row["error"] = "faulty build failed: " + out[-300:]
                        errors.append("%s/%s: %s" % (fault, kind, row["error"]))
                        continue
                with tempfile.TemporaryDirectory(prefix="run_", dir=WORK) as tmp:
                    res = probe(fault, kind, fdir, tmp)
                row["faulty"] = res["passed"] if res["started"] else None
                row["failing"] = res["failing"]
                if not res["started"]:
                    row["error"] = "probe not started: rc=%s %s" % (res["rc"], res["output"][-300:])
                    errors.append("%s/%s: %s" % (fault, kind, row["error"]))
                else:
                    row["caught"] = base_res["passed"] and not res["passed"]
            finally:
                if not a.keep:
                    shutil.rmtree(fdir, ignore_errors=True)
            print(fault, kind, "caught" if row["caught"] else "NOT CAUGHT", flush=True)
        if not a.keep:
            shutil.rmtree(clean_root, ignore_errors=True)

    def cell(v):
        return "error" if v is None else ("pass" if v else "FAIL")

    print()
    print("| fault | compiler | baseline | faulty | caught | failing checks |")
    print("|---|---|---|---|---|---|")
    for r in rows:
        names = "; ".join(r["failing"][:6]) + (" ..." if len(r["failing"]) > 6 else "")
        print("| %s | %s | %s | %s | %s | %s |" % (r["fault"], r["compiler"], cell(r["baseline"]), cell(r["faulty"]),
                                                  "yes" if r["caught"] else "NO", names))
    (WORK / "faults.json").write_text(json.dumps({"rows": rows, "errors": errors}, indent=2))
    for e in errors:
        print("ERROR", e)
    return 0 if rows and all(r["caught"] for r in rows) and not errors else 1


if __name__ == "__main__":
    sys.exit(main())
