#!/usr/bin/env python3
"""Standalone-exe test rig (M3 step 5): Rust `pruvc --emit exe`, checked against saved golden results.

    python tests/standalone/run.py [--rs-path P] [--jobs N] [--keep DIR] [--only NAME] [--pruv-loose]
                                   [--golden PATH] [--update-golden [--prune-golden]] [--vs-py]

PF: since bundle PF the Python compiler (pruvc/) is frozen. By default it is neither run nor needed: the rig uses only the
Rust pruvc (and no rustc) and compares the result of each case with tests/golden/standalone.txt (tests/golden.py).
--vs-py is an opt-in extra (never used by CI) that runs the frozen Python pruvc as the reference, as before; plain
--vs-py does not touch the golden, --vs-py --update-golden is the capture path (writes the golden from the Rust side,
and the table shows every py/rs disagreement).

Golden keys: "standalone|tests/standalone/NAME.pruv|run MODE" (digest of exit code, stdout and `pruv:` stderr lines of the
built exe) and "standalone|tests/standalone/NAME.pruv|reject MODE" (digest of the error JSON lines). exe_entry cases are
checked structurally and have no key.

For every tests/standalone/*.pruv and every mode in its `// flags:` header:
  expect: run        build the exe with the Rust pruvc, run it (timeout 60, no stdin); the exit code, stdout bytes and
                     the stderr lines starting with `pruv:` (Rust panic decoration is ignored) must match the golden.
                     With --vs-py also build the reference (Python pruvc -> Rust -> rustc -O) and compare with it.
  expect: exe_entry  the Rust pruvc must fail with JSON stage "link" code "exe_entry" (exit 1).
  expect: reject     the Rust pruvc must exit 1 and its error JSON lines must match the golden.
                     With --vs-py the Python pruvc must print the same error JSON lines.

Header comments (leading `//` lines):
  // flags: default | --no-verify | --trust-callers | --contract-budget 1000
  // expect: run|exe_entry|reject
  // outdir: <sub path under the case work dir where the exe is written; may contain spaces/Korean>
  // srcdir: <sub path the source is copied to before compiling>

Exit code: 0 all same, 1 any diff/build_fail, 2 usage error or missing binary. Python 3.9, stdlib only.
"""
import argparse
import concurrent.futures
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.join(ROOT, "tests"))
import golden  # noqa: E402
RUSTC = shutil.which("rustc") or os.path.expanduser("~/.cargo/bin/rustc")
PYTHON = os.path.join(ROOT, ".venv", "bin", "python")
if not os.path.exists(PYTHON):
    PYTHON = sys.executable
PRUVC_PY = os.path.join(ROOT, "pruvc", "pruvc.py")
EXE_SUFFIX = ".exe" if os.name == "nt" else ""
RUN_TIMEOUT = 60
BUILD_TIMEOUT = 300


class BuildFail(Exception):
    pass


def parse_header(path):
    info = {"flags": [[]], "expect": "run", "outdir": None, "srcdir": None}
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            s = line.strip()
            if not s.startswith("//"):
                if s:
                    break
                continue
            m = re.match(r"//\s*(flags|expect|outdir|srcdir)\s*:\s*(.*)$", s)
            if not m:
                continue
            key, val = m.group(1), m.group(2).strip()
            if key == "flags":
                modes = []
                for part in val.split("|"):
                    part = part.strip()
                    modes.append([] if part in ("", "default") else shlex.split(part))
                info["flags"] = modes or [[]]
            else:
                info[key] = val
    return info


def run(cmd, timeout):
    """(returncode or None on timeout, stdout bytes, stderr bytes)"""
    try:
        p = subprocess.run(cmd, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                           stderr=subprocess.PIPE, timeout=timeout)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return None, b"", b"timeout after %ds" % timeout
    except OSError as e:
        raise BuildFail("cannot run %s: %s" % (cmd[0], e))


def json_lines(data):
    out = []
    for ln in data.decode("utf-8", "replace").splitlines():
        ln = ln.strip()
        if not ln:
            continue
        try:
            out.append(json.loads(ln))
        except ValueError:
            out.append({"_raw": ln})
    return out


def pruv_lines(stderr):
    """stderr lines that start with `pruv:`. Newer rustc prints the panic message on its own line after
    `thread 'main' panicked at ...:`; older ones print it inline as `panicked at 'pruv: ...'`."""
    res = []
    for ln in stderr.decode("utf-8", "replace").splitlines():
        if ln.startswith("pruv:"):
            res.append(ln)
        else:
            m = re.search(r"panicked at '(pruv: .*?)'(?:, |$)", ln)
            if m:
                res.append(m.group(1))
    return res


def short(b, n=300):
    t = b.decode("utf-8", "replace")
    return t if len(t) <= n else t[:n] + "...(+%d)" % (len(t) - n)


def build_rust_exe(rs_path, src, exe, flags):
    rc, out, err = run([rs_path, src, "--emit", "exe", "-o", exe] + flags, BUILD_TIMEOUT)
    return rc, json_lines(out), err


def run_text(res):
    """Golden bytes of one exe run: exit code, stdout bytes as they are, `pruv:` stderr lines (no timings, no machine
    paths). Bytes, so the golden hashes them raw (line endings and invalid UTF-8 included)."""
    rc, out, err = res
    return (b"exit=%s\nstdout=" % str(rc).encode() + out
            + b"\npruv=" + json.dumps(pruv_lines(err), ensure_ascii=False).encode("utf-8") + b"\n")


def json_text(rc, js):
    return "exit=%s\n%s\n" % (rc, "\n".join(json.dumps(j, ensure_ascii=False) for j in js))


def build_reference(src, work, flags):
    """--vs-py only. Python pruvc -> Rust, plus the driver; returns (exe path or None, rc, json lines, stderr)."""
    d = os.path.join(work, "ref")
    os.makedirs(d, exist_ok=True)
    sol = os.path.join(d, "sol.rs")
    rc, out, err = run([PYTHON, PRUVC_PY, src, "-o", sol] + flags, BUILD_TIMEOUT)
    if rc != 0:
        return None, rc, json_lines(out), err
    drv = os.path.join(d, "drv.rs")
    with open(drv, "w") as fh:
        fh.write('#![allow(dead_code, unused_imports)]\n'
                 '#[path = "sol.rs"] mod sol;\nfn main() { sol::main(); }\n')
    exe = os.path.join(d, "ref" + EXE_SUFFIX)
    r_rc, r_out, r_err = run([RUSTC, "-O", "--edition", "2021", "-o", exe, drv], BUILD_TIMEOUT)
    if r_rc != 0:
        raise BuildFail("reference rustc failed: " + short(r_err, 600))
    return exe, rc, json_lines(out), err


def case_run(rs_path, src, work, info, flags, loose, vs_py):
    """-> (status, detail, golden text)"""
    exe_dir = os.path.join(work, info["outdir"]) if info["outdir"] else work
    os.makedirs(exe_dir, exist_ok=True)
    exe = os.path.join(exe_dir, "prog" + EXE_SUFFIX)
    rc, js, err = build_rust_exe(rs_path, src, exe, flags)
    if rc != 0 or not any(j.get("ok") is True for j in js) or not os.path.exists(exe):
        raise BuildFail("rust pruvc exit %s json=%s stderr=%s" % (rc, json.dumps(js, ensure_ascii=False)[:400], short(err)))
    a = run([exe], RUN_TIMEOUT)
    gtext = run_text(a)
    if not vs_py:
        if a[0] is None:
            raise BuildFail("rust exe timed out: " + short(a[2], 120))
        return "same", "", gtext
    ref, _, _, _ = build_reference(src, work, flags)
    if ref is None:
        raise BuildFail("python pruvc failed on a `run` case")
    b = run([ref], RUN_TIMEOUT)
    diffs = []
    if a[0] != b[0]:
        diffs.append("exit: rust-exe=%s reference=%s" % (a[0], b[0]))
    if a[1] != b[1]:
        diffs.append("stdout differs")
    pa, pb = pruv_lines(a[2]), pruv_lines(b[2])
    if loose:
        if bool(pa) != bool(pb):
            diffs.append("pruv lines present: rust-exe=%s reference=%s" % (bool(pa), bool(pb)))
    elif pa != pb:
        diffs.append("pruv stderr lines differ")
    if not diffs:
        return "same", "", gtext
    det = ["; ".join(diffs),
           "  rust-exe  exit=%s stdout=%r pruv=%r" % (a[0], short(a[1]), pa),
           "  reference exit=%s stdout=%r pruv=%r" % (b[0], short(b[1]), pb)]
    if a[0] is None or b[0] is None:
        det.append("  stderr(rust-exe)=%r stderr(reference)=%r" % (short(a[2], 120), short(b[2], 120)))
    return "diff", "\n".join(det), gtext


def case_exe_entry(rs_path, src, work, info, flags):
    exe = os.path.join(work, "prog" + EXE_SUFFIX)
    rc, js, err = build_rust_exe(rs_path, src, exe, flags)
    errs = [j for j in js if "code" in j]
    if rc == 1 and errs and errs[0].get("code") == "exe_entry" and errs[0].get("stage") == "link":
        return "same", "", None
    return "diff", "expected exit 1 + stage link / code exe_entry; got exit %s json=%s" % (
        rc, json.dumps(js, ensure_ascii=False)[:400]), None


def case_reject(rs_path, src, work, info, flags, vs_py):
    """-> (status, detail, golden text)"""
    exe = os.path.join(work, "prog" + EXE_SUFFIX)
    rc, js, err = build_rust_exe(rs_path, src, exe, flags)
    gtext = json_text(rc, js)
    det = []
    if rc != 1:
        det.append("rust exit %s (expected 1)" % rc)
    if not any("code" in j for j in js):
        det.append("no error JSON line")
    pjs = None
    if vs_py:
        d = os.path.join(work, "ref")
        os.makedirs(d, exist_ok=True)
        prc, pout, perr = run([PYTHON, PRUVC_PY, src, "-o", os.path.join(d, "x.rs")] + flags, BUILD_TIMEOUT)
        pjs = json_lines(pout)
        if prc != 1:
            det.append("python exit %s (expected 1)" % prc)
        if js != pjs:
            det.append("error JSON differs")
    if not det:
        return "same", "", gtext
    return "diff", "; ".join(det) + "\n  rust   =%s%s" % (
        json.dumps(js, ensure_ascii=False)[:500],
        "\n  python =%s" % json.dumps(pjs, ensure_ascii=False)[:500] if vs_py else ""), gtext


def do_case(args, root_work, path, idx, flags):
    name = os.path.splitext(os.path.basename(path))[0]
    info = parse_header(path)
    mode = " ".join(flags) or "default"
    t0 = time.time()
    work = os.path.join(root_work, "%s_%d" % (name, idx))
    os.makedirs(work, exist_ok=True)
    src = path
    if info["srcdir"]:
        sd = os.path.join(work, info["srcdir"])
        os.makedirs(sd, exist_ok=True)
        src = os.path.join(sd, os.path.basename(path))
        shutil.copyfile(path, src)
    gtext = None
    try:
        if info["expect"] == "run":
            status, detail, gtext = case_run(args.rs_path, src, work, info, flags, args.pruv_loose, args.vs_py)
        elif info["expect"] == "exe_entry":
            status, detail, gtext = case_exe_entry(args.rs_path, src, work, info, flags)
        elif info["expect"] == "reject":
            status, detail, gtext = case_reject(args.rs_path, src, work, info, flags, args.vs_py)
        else:
            status, detail = "build_fail", "unknown expect %r" % info["expect"]
    except BuildFail as e:
        status, detail = "build_fail", str(e)
    except Exception as e:  # keep the table going
        status, detail = "build_fail", "%s: %s" % (type(e).__name__, e)
    rel = os.path.relpath(path, ROOT).replace(os.sep, "/")
    key = "standalone|%s|%s %s" % (rel, info["expect"], mode)
    return name, mode, info["expect"], status, detail, time.time() - t0, key, gtext


def main():
    ap = argparse.ArgumentParser(description="Rust pruvc --emit exe against golden results (--vs-py: Python reference)")
    ap.add_argument("--rs-path", default=os.path.join(ROOT, "pruvc-rs", "target", "release", "pruvc.exe" if os.name == "nt" else "pruvc"))
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 2)
    ap.add_argument("--keep", metavar="DIR", default=None, help="keep work files under DIR")
    ap.add_argument("--only", metavar="NAME", default=None, help="only cases whose name contains NAME")
    ap.add_argument("--pruv-loose", action="store_true",
                    help="with --vs-py: compare only whether a pruv: line exists, not its text")
    ap.add_argument("--vs-py", action="store_true",
                    help="opt-in, not CI: also run the FROZEN Python pruvc as the reference (needs rustc and pruvc/pruvc.py)")
    golden.add_args(ap)
    try:
        args = ap.parse_args()
    except SystemExit as e:
        sys.exit(2 if e.code else 0)
    args.rs_path = os.path.abspath(args.rs_path)
    if args.jobs < 1:
        print("--jobs must be >= 1", file=sys.stderr)
        return 2
    needed = [("Rust pruvc", args.rs_path)]
    if args.vs_py:
        needed += [("rustc", RUSTC), ("Python pruvc", PRUVC_PY)]
    for label, p in needed:
        if not p or not os.path.exists(p):
            print("missing %s: %s" % (label, p), file=sys.stderr)
            return 2
    files = sorted(f for f in os.listdir(HERE) if f.endswith(".pruv"))
    if args.only:
        files = [f for f in files if args.only in f]
    if not files:
        print("no test programs selected", file=sys.stderr)
        return 2
    if args.keep:
        root_work = os.path.abspath(args.keep)
        os.makedirs(root_work, exist_ok=True)
    else:
        root_work = tempfile.mkdtemp(prefix="pruv_standalone_")
    jobs = []
    for f in files:
        p = os.path.join(HERE, f)
        for i, flags in enumerate(parse_header(p)["flags"]):
            jobs.append((p, i, flags))
    # Plain --vs-py never reads or writes the golden; --vs-py --update-golden captures it from the Rust side.
    use_golden = not args.vs_py or args.update_golden
    g = golden.Golden.from_args(args, "standalone.txt",
                                "standalone: result of the Rust --emit exe programs (run) and error JSON (reject)",
                                tmp_dirs=[root_work])
    raw = []
    try:
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as ex:
            futs = [ex.submit(do_case, args, root_work, p, i, fl) for p, i, fl in jobs]
            for fu in futs:
                raw.append(fu.result())
    finally:
        if not args.keep:
            shutil.rmtree(root_work, ignore_errors=True)
    results = []
    for name, mode, expect, status, detail, secs, key, gtext in raw:
        # golden: only cases whose Rust side ran normally (a build failure has no usable output)
        if use_golden and gtext is not None and status != "build_fail":
            if not g.check(key, gtext, "%s %s" % (expect, status), raw=isinstance(gtext, bytes)) and status == "same" and not args.update_golden:
                status, detail = "diff", "output differs from the golden (see GOLDEN lines below)"
        results.append((name, mode, expect, status, detail, secs))
    w = max(len(r[0]) for r in results)
    wm = max(len(r[1]) for r in results)
    print("%-*s  %-*s  %-9s  %-10s  %s" % (w, "name", wm, "mode", "expect", "result", "time"))
    for name, mode, expect, status, detail, secs in results:
        print("%-*s  %-*s  %-9s  %-10s  %.1fs" % (w, name, wm, mode, expect, status, secs))
    bad = [r for r in results if r[3] != "same"]
    for name, mode, expect, status, detail, secs in bad:
        print("\n[%s] %s (%s)\n%s" % (status, name, mode, detail))
    print("\n%d cases: %d same, %d diff, %d build_fail" % (
        len(results), len(results) - len(bad),
        sum(1 for r in bad if r[3] == "diff"), sum(1 for r in bad if r[3] == "build_fail")))
    golden_ok = g.finish() if use_golden else True
    return 1 if bad or not golden_ok else 0


if __name__ == "__main__":
    sys.exit(main())
