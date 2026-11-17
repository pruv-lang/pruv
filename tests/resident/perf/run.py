#!/usr/bin/env python3
"""Resident-state performance harness (docs/RESIDENT_STATE_SPEC.md section 8).

Compiles state.pruv with the Rust compiler for the Rust source backend and the LLVM backend, builds driver.rs
against each output the way tests/run_all.py builds its runtime drivers, runs it N times and prints the median
ns_per_call per (build, S, kind) plus the gate verdicts:

  step ratio  : ns_per_call(step, S=20000) / ns_per_call(step, S=2000) < 2.0
  noop cost   : ns_per_call(noop) <= 2000 at both sizes (llvm builds only)

The rust-backend build compiles generated code and driver in one crate with #[inline] wrappers, so its noop is not
a real call boundary; its rows are labelled "integrated (inlinable)", measured but not gated on noop. The llvm build
links a separate object (real external call), so the boundary gate applies there. The step ratio gate applies to both.

Gates apply to the --trust-callers builds (the intended deployment, spec section 4.2/4.5). Checked builds (no
--trust-callers) are measured too and shown, but are informational unless --gate-checked is given.

Exit codes: 0 gates pass, 1 a gate fails, 2 a tool is missing or a build/run step failed (nothing to judge).
Python 3.9, standard library only.
"""
import argparse
import os
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))   # tests/resident/perf -> repo root
STATE = os.path.join(HERE, "state.pruv")
DRIVER = os.path.join(HERE, "driver.rs")
RUNTIME_C = os.path.join(ROOT, "pruvc", "runtime.c")
PRUVC_DEFAULT = os.path.join(ROOT, "pruvc-rs", "target", "release",
                             "pruvc.exe" if os.name == "nt" else "pruvc")
CPU_FLAG = "-mcpu=native" if platform.machine().lower() in ("arm64", "aarch64") else "-march=native"

SIZES = (2000, 20000)
KINDS = ("step", "noop", "copy")
RATIO_LIMIT = 2.0
NOOP_LIMIT_NS = 2000.0
LINE_RE = re.compile(r"backend=(\S+) S=(\d+) kind=(\w+) calls=(\d+) ns_per_call=([0-9.]+)")


class ToolError(Exception):
    """A tool is missing or a build/run step failed: the harness cannot judge the gates."""


def find_tool(name, extra=()):
    found = shutil.which(name)
    if found:
        return found
    for p in extra:
        p = os.path.expanduser(p)
        if os.path.exists(p):
            return p
    return None


def run(cmd, env=None, timeout=300):
    try:
        return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, env=env)
    except OSError as e:
        raise ToolError("cannot execute %s: %s" % (cmd[0], e))
    except subprocess.TimeoutExpired:
        raise ToolError("timeout: %s" % " ".join(cmd))


def tail(c):
    return ((c.stdout or "") + (c.stderr or ""))[-800:]


def build(label, backend, trust, pruvc, rustc, clang, work):
    """Returns the path of the driver executable for one build variant."""
    stem = label.replace("-", "_")
    gen = os.path.join(work, stem + ("_gen.rs" if backend == "rust" else ".ll"))
    bindings = os.path.join(work, stem + "_bind.rs")
    cmd = [pruvc, STATE, "-o", gen, "--backend", backend]
    if trust:
        cmd.append("--trust-callers")
    if backend == "llvm":
        cmd += ["--rust-bindings", bindings]
    c = run(cmd)
    if c.returncode != 0:
        raise ToolError("pruvc failed (%s, exit %d): %s" % (label, c.returncode, tail(c)))

    link = []
    if backend == "llvm":
        # Same steps as run_all.llvm_objects / runtime_object: clang -O2 on the IR and on runtime.c.
        gen_o = os.path.join(work, stem + "_gen.o")
        c = run([clang, "-O2", CPU_FLAG, "-Wno-override-module", "-c", gen, "-o", gen_o])
        if c.returncode != 0:
            raise ToolError("clang (gen.ll, %s): %s" % (label, tail(c)))
        rt_o = os.path.join(work, "runtime.o")
        if not os.path.exists(rt_o):
            c = run([clang, "-O2", CPU_FLAG, "-std=c11", "-DPRUV_RUST_ALLOCATOR", "-c", RUNTIME_C, "-o", rt_o])
            if c.returncode != 0:
                raise ToolError("clang (runtime.c): " + tail(c))
        link = ["-C", "link-arg=" + gen_o, "-C", "link-arg=" + rt_o]
        sol = bindings
    else:
        sol = gen

    # Same main-file shape as run_all: the generated module, then the driver text.
    main = os.path.join(work, stem + "_main.rs")
    with open(main, "w") as fh:
        fh.write('#![allow(dead_code, unused_imports)]\n#[path = "%s"] mod sol;\n' % sol.replace("\\", "/"))
        with open(DRIVER) as d:
            fh.write(d.read())
    exe = os.path.join(work, stem + "_bin" + (".exe" if os.name == "nt" else ""))
    env = dict(os.environ, PERF_BACKEND=label)
    c = run([rustc, "-C", "opt-level=3", "--edition", "2021", main, "-o", exe] + link, env=env)
    if c.returncode != 0:
        raise ToolError("rustc (%s): %s" % (label, tail(c)))
    return exe


def measure(exe, label, runs):
    """Runs the driver `runs` times; returns {(S, kind): median ns_per_call}."""
    samples = {}
    for _ in range(runs):
        c = run([exe], timeout=600)
        if c.returncode != 0:
            raise ToolError("driver %s exit %d: %s" % (label, c.returncode, tail(c)))
        for m in LINE_RE.finditer(c.stdout):
            samples.setdefault((int(m.group(2)), m.group(3)), []).append(float(m.group(5)))
    for s in SIZES:
        for k in KINDS:
            if len(samples.get((s, k), [])) != runs:
                raise ToolError("driver %s: missing measurement S=%d kind=%s" % (label, s, k))
    return dict((key, statistics.median(v)) for key, v in samples.items())


def gates(res):
    """res: {(S, kind): ns}. Returns (ratio, noop_max, ratio_ok, noop_ok)."""
    ratio = res[(SIZES[1], "step")] / res[(SIZES[0], "step")]
    noop_max = max(res[(s, "noop")] for s in SIZES)
    return ratio, noop_max, ratio < RATIO_LIMIT, noop_max <= NOOP_LIMIT_NS


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--pruvc", default=PRUVC_DEFAULT, help="Rust compiler binary (default: %(default)s)")
    ap.add_argument("--runs", type=int, default=3, help="driver runs per build; the median is used (default 3)")
    ap.add_argument("--out", help="also write the table and verdicts as markdown to this file")
    ap.add_argument("--no-checked", action="store_true", help="skip the checked builds (without --trust-callers)")
    ap.add_argument("--gate-checked", action="store_true", help="apply the gates to the checked builds too")
    ap.add_argument("--keep", action="store_true", help="keep the temporary build directory")
    args = ap.parse_args()

    rustc = find_tool("rustc", ["~/.cargo/bin/rustc"])
    clang = find_tool("clang", ["/usr/bin/clang"])
    problems = []
    if not os.path.exists(args.pruvc):
        problems.append("compiler not found: %s (build it: cd pruvc-rs && cargo build --release --locked)" % args.pruvc)
    if not rustc:
        problems.append("rustc not found")
    if not clang:
        problems.append("clang not found (needed for the LLVM backend)")
    if not os.path.exists(RUNTIME_C):
        problems.append("runtime.c not found: %s" % RUNTIME_C)
    if problems:
        for p in problems:
            print("error: " + p, file=sys.stderr)
        return 2

    variants = [("rust", "rust", True), ("llvm", "llvm", True)]
    if not args.no_checked:
        variants += [("rust-checked", "rust", False), ("llvm-checked", "llvm", False)]

    work = tempfile.mkdtemp(prefix="resident_perf_")
    results = {}
    try:
        for label, backend, trust in variants:
            print("building %s ..." % label, file=sys.stderr)
            exe = build(label, backend, trust, args.pruvc, rustc, clang, work)
            print("running %s x%d ..." % (label, args.runs), file=sys.stderr)
            results[label] = measure(exe, label, args.runs)
    except ToolError as e:
        print("error: %s" % e, file=sys.stderr)
        return 2
    finally:
        if args.keep:
            print("build directory kept: %s" % work, file=sys.stderr)
        else:
            shutil.rmtree(work, ignore_errors=True)

    # The rust-backend build compiles generated code and driver in one crate with #[inline] wrappers, so its noop is
    # not a real call boundary. The llvm build links a separate object: a real external call.
    def shown(label, backend):
        return label + " [integrated (inlinable)]" if backend == "rust" else label

    lines = []
    lines.append("| build | S | step ns | noop ns | copy ns |")
    lines.append("|---|---:|---:|---:|---:|")
    for label, backend, _ in variants:
        for s in SIZES:
            r = results[label]
            lines.append("| %s | %d | %.1f | %.1f | %.1f |" % (shown(label, backend), s, r[(s, "step")], r[(s, "noop")], r[(s, "copy")]))
    lines.append("")
    lines.append("Median of %d runs, 10,000 calls per measurement. Gates: step ratio S=%d/S=%d < %.1f (all gated builds); "
                 "noop <= %d ns (llvm builds only: real external call boundary)."
                 % (args.runs, SIZES[1], SIZES[0], RATIO_LIMIT, int(NOOP_LIMIT_NS)))
    lines.append("rust-backend rows are integrated (inlinable): generated code and driver share one crate, so their noop "
                 "is measured but is not a call-boundary cost and is not gated.")
    lines.append("")

    failed = False
    for label, backend, trust in variants:
        ratio, noop_max, ratio_ok, noop_ok = gates(results[label])
        gated = trust or args.gate_checked
        tag = "" if gated else " (informational)"
        name = shown(label, backend)
        lines.append("- %s step ratio %.2f (< %.1f): %s%s" % (name, ratio, RATIO_LIMIT, "PASS" if ratio_ok else "FAIL", tag))
        if backend == "llvm":
            lines.append("- %s noop %.1f ns (<= %d): %s%s" % (name, noop_max, int(NOOP_LIMIT_NS), "PASS" if noop_ok else "FAIL", tag))
        else:
            lines.append("- %s noop %.1f ns (not gated: no real call boundary)" % (name, noop_max))
        if gated and not (ratio_ok and (noop_ok or backend != "llvm")):
            failed = True
    lines.append("")
    lines.append("verdict: %s" % ("FAIL" if failed else "PASS"))

    text = "\n".join(lines)
    print(text)
    if args.out:
        with open(args.out, "w") as fh:
            fh.write(text + "\n")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
