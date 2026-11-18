"""T4: per-edge LLVM link test for the 7 "G6 shim helper edges".

usage: shim_edges.py [--py PATH] [--rs PATH] [--only-py | --only-rs] [--work DIR]

In the LLVM backend a runtime contract check of an index observer (the caller) also calls a size/capacity
observer (the bound helper), so the Rust shim (`mod pruv_g6_shim` in the --rust-bindings file) must define
`pruv_g6_v1_pruv_g6_<bound>` too (see pruvc/foreign_shim.py). If an edge is missing, the bindings lack that
symbol and linking fails. For each edge and each selected compiler this test:
  1. compiles tests/shim_edges/<caller>.pruv with --no-verify --backend llvm --rust-bindings (the requires
     clause becomes a runtime check) and checks the .ll text calls @pruv_g6_v1_pruv_g6_<bound> (else the
     test is vacuous);
  2. assembles the .ll with `clang -O0` (MUST be -O0: at -O2 the unused internal function is deleted and the
     reference disappears);
  3. links main.rs (`#[path] mod sol;` of the bindings) with the C runtime using
     `rustc -C link-dead-code` (MUST pass it: otherwise ld dead-strips and the missing symbol is not reported).

--py is the path of pruvc.py (same meaning as in spec_trace.py / run_all.py); it is run with the pinned
Python (compiler_driver.pinned_python()). --rs is the Rust pruvc binary.
Last line: `shim_edges: N/M ok`; exit 0 only if all cases pass.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
if HERE not in sys.path:
    sys.path.insert(0, HERE)
import compiler_driver as CD  # noqa: E402

# (caller op, bound helper); order matches pruvc/foreign_shim.py:113-122.
EDGES = [
    ("file_byte", "file_size"),
    ("proc_byte", "proc_size"),
    ("ev_kind", "ev_cap"),
    ("ev_eof", "ev_cap"),
    ("ev_write_open", "ev_cap"),
    ("ev_byte", "ev_size"),
    ("net_byte", "net_size"),
]

RUSTC = shutil.which("rustc") or os.path.expanduser("~/.cargo/bin/rustc")
CLANG = shutil.which("clang") or "/usr/bin/clang"
RUNTIME_C = os.path.join(ROOT, "pruvc", "runtime.c")


def sh(argv, cwd=None):
    """Run argv with a timeout; returns (returncode, stdout, stderr)."""
    try:
        p = subprocess.run(argv, capture_output=True, text=True, encoding="utf-8", errors="replace",
                           timeout=300, cwd=cwd)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return 124, "", "timeout after 300s: %s" % " ".join(map(str, argv))
    except OSError as e:
        return 127, "", "cannot run %s: %r" % (argv[0], e)


def build_runtime(work):
    """Compile pruvc/runtime.c once; returns (rt.o path or None, error text)."""
    rt = os.path.join(work, "rt.o")
    rc, _, err = sh([CLANG, "-O2", "-std=c11", "-DPRUV_RUST_ALLOCATOR", "-c", RUNTIME_C, "-o", rt])
    return (rt, "") if rc == 0 else (None, "runtime.c: " + err[-600:])


def case(drv, op, bound, rt, work):
    """Returns "" on success, else the failure reason."""
    d = os.path.join(work, drv.kind, op)
    os.makedirs(d, exist_ok=True)
    src = os.path.join(ROOT, "tests", "shim_edges", op + ".pruv")
    ll = os.path.join(d, "X.ll")
    bind = os.path.join(d, "X_b.rs")
    o = os.path.join(d, "X.o")
    main_rs = os.path.join(d, "main.rs")
    exe = os.path.join(d, "X_bin")
    c = drv.run(drv.argv(src, ll, ["--no-verify", "--rust-bindings", bind], "llvm"), timeout=300)
    if c.returncode != 0:
        return "compile exit %d: %s" % (c.returncode, (c.stdout + c.stderr)[-600:])
    try:
        with open(ll, encoding="utf-8") as fh:
            text = fh.read()
    except OSError as e:
        return "no .ll output: %r" % e
    if "@pruv_g6_v1_pruv_g6_" + bound not in text:
        return "test is vacuous: no call to %s" % bound
    rc, _, err = sh([CLANG, "-O0", "-Wno-override-module", "-c", ll, "-o", o])
    if rc != 0:
        return "clang: " + err[-600:]
    with open(main_rs, "w", encoding="utf-8") as fh:
        fh.write("#[path = %s] mod sol;\nfn main() {}\n" % json.dumps(os.path.abspath(bind)))
    rc, _, err = sh([RUSTC, "--edition", "2021", "-O", "-C", "link-dead-code", main_rs, "-o", exe,
                     "-C", "link-arg=" + o, "-C", "link-arg=" + rt])
    if rc != 0:
        lines = err.splitlines()   # ld version warnings drown the tail; show the undefined symbol when there is one
        for k, line in enumerate(lines):
            if "Undefined symbols" in line or "undefined reference" in line:
                return "link: " + " ".join(s.strip() for s in lines[k:k + 2])
        return "link: " + err[-600:]
    return ""


def main(argv):
    py_path = rs_path = work = None
    kinds = ["py", "rs"]
    i = 0
    while i < len(argv):
        a = argv[i]
        if a in ("--py", "--rs", "--work") and i + 1 < len(argv):
            if a == "--py":
                py_path = argv[i + 1]
            elif a == "--rs":
                rs_path = argv[i + 1]
            else:
                work = argv[i + 1]
            i += 2
            continue
        if a == "--only-py":
            kinds = ["py"]
        elif a == "--only-rs":
            kinds = ["rs"]
        else:
            sys.stderr.write(__doc__.split("\n\n")[0] + "\n")
            return 2
        i += 1
    if rs_path is None:
        rs_path = os.path.join(ROOT, "pruvc-rs", "target", "release", "pruvc")
    drivers = [CD.make_driver(k, py_path if k == "py" else rs_path) for k in kinds]
    own_work = work is None
    work = os.path.abspath(work or tempfile.mkdtemp(prefix="shim_edges_"))
    os.makedirs(work, exist_ok=True)
    total = ok = 0
    try:
        rt, rt_err = build_runtime(work)
        for drv in drivers:
            for op, bound in EDGES:
                total += 1
                why = rt_err if rt is None else case(drv, op, bound, rt, work)
                if why:
                    print("FAIL %s %s -> %s: %s" % (drv.kind, op, bound, why))
                else:
                    ok += 1
                    print("ok %s %s -> %s" % (drv.kind, op, bound))
    finally:
        if own_work:
            shutil.rmtree(work, ignore_errors=True)
    print("shim_edges: %d/%d ok" % (ok, total))
    return 0 if ok == total else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
