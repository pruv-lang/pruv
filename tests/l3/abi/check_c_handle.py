"""L3 P10: the C adapter handle (manifest `raw_entries = "c"`, module `pruv_c` in the LLVM Rust bindings).

usage: check_c_handle.py WORK [--rs PATH] [--py PATH]
  --rs PATH: the Rust driver (default: the Python compiler); every generated file (.rs bindings, .ll) must also be
             byte-identical to the Python compiler's output
  --py PATH: the pruvc.py to run (default: pruvc/pruvc.py of this repository)
Steps: (1) compile a small program with raw_entries = "c" (--backend llvm --rust-bindings), (2) text facts of B.rs
(pruv_c present for "c", absent for "crate"; the entry symbols pruv_h_* / pruv_hr_grow), (2b) the same program compiled
with the Rust backend (no --backend flag, -o out.rs) for raw_entries "crate" and "c": the two outputs are byte-identical
and contain no extern "C", (3) build libpruv_b.a = rustc --crate-type staticlib --cfg pruv_c_probe B.rs, the .ll object
and the runtime object with clang, (4) link tests/l3/abi/c_probe.c with them and run it (prints OK). The probe covers
ABI/lifetime, ret slots between sentinels, positional parameter names (h, ret, st, ro, ro + ro_len), [u8]/str/[bool]
arguments, zero-length NULL, BAD_ARGS (misaligned, overlap, address overflow, length > INT64_MAX/8, overlap with the
handle object, invalid UTF-8, bad bool bytes, rebuild new_len negative/oversized), rebuild observed via stlen, and
poisoning (the probe panic, whose payload Drop panics, returns 3 without aborting; a poisoned handle returns 1 before
any argument check), and the batch entry app__pruv_hb_op (order, stop rule, done, up-front and per-item BAD_ARGS, poison).
All global C symbols carry the manifest package name as prefix ("app__"); text-level check with a second name "app2".
The batch entry refuses up front with *done = 0 when done is usable (n = -1 with done preset to 77, NULL items with n = 1).
Two programs (packages twoa / twob, same entry name, same G6 builtins) are compiled with the LLVM backend and linked into
ONE binary (check_two_programs.py, c_two.c): the pruv_abi_* / pruv_g6_v1_* / pruv_g6_bridge_v1 symbols carry the prefix.
Prints one FAIL line per failing case; exit 0 only if every case passes. Needs rustc and clang (PATH, else ~/.cargo/bin, /usr/bin)."""
from pathlib import Path
import json
import os
import platform
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
RUSTC = shutil.which("rustc") or os.path.expanduser("~/.cargo/bin/rustc")
CLANG = shutil.which("clang") or "/usr/bin/clang"
RUNTIME_C = ROOT / "pruvc" / "runtime.c"

# state = 4 words; see c_probe.c for what each entry means
SRC = ("pub fn setup(inout st: [i64]) -> ()\n"
       "  requires len(st) >= 4\n"
       "{\n  st[0] = 10;\n  st[1] = 20;\n  st[2] = 0;\n  st[3] = 0;\n}\n"
       "\n"
       "pub fn op(inout st: [i64], a: [i64], inout out: [i64]) -> i64\n"
       "  requires len(st) >= 4 && len(a) >= 1 && len(out) >= 1\n"
       "{\n  st[2] = st[2] + a[0];\n  out[0] = st[2];\n  return st[2] + st[0];\n}\n"
       "\n"
       "pub fn big(st: [i64], t: i64) -> bool\n"
       "  requires len(st) >= 4\n"
       "{\n  return st[2] > t;\n}\n"
       "\n"
       "pub fn flagset(st: [i64]) -> bool\n"
       "  requires len(st) >= 4\n"
       "{\n  return st[3] == 1;\n}\n"
       "\n"
       "pub fn setflag(inout st: [i64], f: bool) -> ()\n"
       "  requires len(st) >= 4\n"
       "{\n  if f {\n    st[3] = 1;\n  } else {\n    st[3] = 0;\n  }\n}\n"
       "\n"
       "pub fn flag0(st: [i64], b: [bool]) -> i64\n"
       "  requires len(st) >= 1 && len(b) >= 1\n"
       "{\n  if b[0] {\n    return 1;\n  }\n  return 0;\n}\n"
       "\n"
       "pub fn grow(prev: [i64], inout next: [i64]) -> i64\n"
       "{\n"
       "  if len(prev) < 4 || len(next) < len(prev) {\n    return 1;\n  }\n"
       "  next[0] = prev[0];\n  next[1] = prev[1];\n  next[2] = prev[2];\n  next[3] = prev[3];\n"
       "  return 0;\n}\n"
       "\n"
       # parameter names that used to collide with the generated C names (h, ret, st, ro, ro + ro_len)
       "pub fn clash(state: [i64], st: [i64], h: i64, ret: i64, ro: [i64], ro_len: i64) -> i64\n"
       "  requires len(state) >= 4 && len(st) >= 1 && len(ro) >= 1\n"
       "{\n  return st[0] * 10000 + h * 1000 + ret * 100 + ro[0] * 10 + ro_len + state[0] * 100000;\n}\n"
       "\n"
       "pub fn bsum(st: [i64], b: [u8]) -> i64\n"
       "{\n"
       "  let mut i: i64 = 0;\n  let mut s: i64 = 0;\n"
       "  while i < len(b) invariant 0 <= i && i <= len(b) decreases len(b) - i {\n"
       "    s = s + b[i];\n    i = i + 1;\n  }\n"
       "  return s;\n}\n"
       "\n"
       "pub fn slen(st: [i64], s: str) -> i64\n"
       "{\n  return len(s);\n}\n"
       "\n"
       "pub fn bflip(st: [i64], inout b: [bool]) -> ()\n"
       "  requires len(b) >= 1\n"
       "{\n"
       "  let mut i: i64 = 0;\n"
       "  while i < len(b) invariant 0 <= i && i <= len(b) decreases len(b) - i {\n"
       "    b[i] = !b[i];\n    i = i + 1;\n  }\n"
       "}\n"
       "\n"
       "pub fn stlen(st: [i64]) -> i64\n"
       "{\n  return len(st);\n}\n")
FLAGS = ["--no-verify"]


def main(work, rs=None, py=None):
    work = Path(work) / "l3_abi_chandle"
    work.mkdir(parents=True)
    pyc = str(Path(py).resolve()) if py else str(ROOT / "pruvc" / "pruvc.py")
    py_cmd = [sys.executable, pyc]
    sel_cmd = [rs] if rs else py_cmd
    fails = []

    def check(name, ok, detail=""):
        if not ok:
            fails.append("FAIL %s: %s" % (name, str(detail).strip()[-600:]))
        return ok

    def run(cmd, timeout=600):
        try:
            c = subprocess.run([str(x) for x in cmd], capture_output=True, text=True, timeout=timeout)
        except (OSError, subprocess.TimeoutExpired) as e:
            return 127, repr(e)
        return c.returncode, c.stdout + c.stderr

    def build(key, name="app"):
        """compile the program with raw_entries = key and manifest package name `name`;
        returns (B.rs, out.ll) of the selected compiler or None"""
        base = work / ("pkg_" + (key or "nokey") + ("" if name == "app" else "_" + name))
        pdir = base / "app"
        pdir.mkdir(parents=True)
        man = {"name": name, "version": "0.1.0", "root": "app.pruv"}
        if key is not None:
            man["raw_entries"] = key
        (pdir / "pruv.json").write_text(json.dumps(man))
        (pdir / "app.pruv").write_text(SRC)
        outs = {}
        for tag, cmd in [("sel", sel_cmd)] + ([("py", py_cmd)] if rs else []):
            d = base / tag
            d.mkdir()
            args = list(cmd) + [str(pdir / "app.pruv"), "-o", str(d / "out.ll")] + FLAGS + [
                "--manifest", str(pdir / "pruv.json"), "--backend", "llvm", "--rust-bindings", str(d / "B.rs")]
            rc, txt = run(args, 300)
            if not check("raw_entries=%s: %s compile" % (key, tag), rc == 0, "exit %d %s" % (rc, txt)):
                return None
            outs[tag] = (d / "B.rs", d / "out.ll")
        if rs:
            for i, what in enumerate(("B.rs", "out.ll")):
                check("raw_entries=%s: rs %s identical to python" % (key, what),
                      outs["sel"][i].read_bytes() == outs["py"][i].read_bytes(), "differs")
        return outs["sel"]

    # ---- text facts: the C module only for "c" ----------------------------------------------------------------------
    other = build("crate")
    if other:
        t = other[0].read_text()
        check("crate: no pruv_c", "pruv_c" not in t and "extern \"C\" fn pruv_h_" not in t)
    built = build("c")
    if built is None:
        for line in fails:
            print(line)
        return 1
    b_rs, ll = built
    t = b_rs.read_text()
    # every global C symbol carries the manifest package name ("app") as a prefix; hb (batch) exists only for entries whose
    # state is inout [i64] and whose return is i64: here only `op`
    for sym in ("pub mod pruv_c", "app__pruv_handle_new", "app__pruv_handle_free", "app__pruv_handle_poisoned",
                "app__pruv_handle_probe_panic", "app__pruv_h_setup", "app__pruv_h_op", "app__pruv_h_big",
                "app__pruv_h_flagset", "app__pruv_h_setflag", "app__pruv_h_flag0", "app__pruv_hr_grow",
                "app__pruv_h_clash", "app__pruv_h_bsum", "app__pruv_h_slen", "app__pruv_h_bflip", "app__pruv_h_stlen",
                "app__pruv_hb_op"):
        check("c: B.rs has " + sym, sym in t)
    for sym in ("app__pruv_hb_setup", "app__pruv_hb_big", "app__pruv_hb_flagset", "app__pruv_hb_setflag",
                "app__pruv_hb_flag0", "app__pruv_hb_clash", "app__pruv_hb_bsum", "app__pruv_hb_slen",
                "app__pruv_hb_bflip", "app__pruv_hb_stlen", "app__pruv_hb_grow", "app__pruv_hr_op"):
        check("c: B.rs has no " + sym, sym not in t)
    # no unprefixed definition of a pruv_c global is left
    import re
    unpref = re.findall(r"\bfn\s+(pruv_(?:handle|h|hr|hb)_\w*)", t)
    check("c: no unprefixed pruv_c function definitions", not unpref, ", ".join(sorted(set(unpref))))
    check("c: the pruv_raw module is still there", "pub mod pruv_raw" in t)
    check("c: the batch entry writes *done = 0 on an up-front refusal", "done.write(0);" in t)
    # the same program under another package name uses only that name's prefix (disjoint global C symbols); linking two
    # programs into one binary is tested below (check_two_programs.py: ONE staticlib with both Rust modules)
    built2 = build("c", "app2")
    if built2:
        t2 = built2[0].read_text()
        for sym in ("app2__pruv_handle_new", "app2__pruv_handle_free", "app2__pruv_h_op", "app2__pruv_hr_grow",
                    "app2__pruv_hb_op"):
            check("c: app2 B.rs has " + sym, sym in t2)
        check("c: app2 B.rs has no app__ symbol", "app__pruv_" not in t2)
        check("c: app B.rs has no app2__ symbol", "app2__pruv_" not in t)

    # ---- Rust backend: "c" is "crate" (the C adapter is LLVM-only, no extern "C" in Rust-backend output) --------------
    rust_out = {}
    for key in ("crate", "c"):
        base = work / ("rustbe_" + key)
        pdir = base / "app"
        pdir.mkdir(parents=True)
        (pdir / "pruv.json").write_text(json.dumps({"name": "app", "version": "0.1.0", "root": "app.pruv",
                                                    "raw_entries": key}))
        (pdir / "app.pruv").write_text(SRC)
        out = base / "out.rs"
        # same flags as check_raw_entries.py for the Rust backend: no --backend flag (rust is the default), -o out.rs
        rc, txt = run(list(sel_cmd) + [str(pdir / "app.pruv"), "-o", str(out)] + FLAGS +
                      ["--manifest", str(pdir / "pruv.json")], 300)
        if check("rust backend raw_entries=%s: compile" % key, rc == 0, "exit %d %s" % (rc, txt)):
            rust_out[key] = out.read_bytes()
    if len(rust_out) == 2:
        check("rust backend: raw_entries c output == crate output", rust_out["c"] == rust_out["crate"], "differs")
        check("rust backend: no extern \"C\"", all(b'extern "C"' not in v for v in rust_out.values()),
              "extern \"C\" in Rust-backend output")

    # ---- build: staticlib (probe cfg) + .ll object + runtime object, then the C probe ---------------------------------
    d = work / "link"
    d.mkdir()
    lib = d / "libpruv_b.a"
    rc, txt = run([RUSTC, "--edition", "2021", "-O", "--crate-type", "staticlib", "--crate-name", "pruv_b",
                   "--cfg", "pruv_c_probe", "-A", "warnings", "--print", "native-static-libs", str(b_rs), "-o", str(lib)])
    # the system libraries the Rust staticlib needs on this target, as rustc reports them (Linux: -lgcc_s -lc ...)
    syslibs = []
    for line in txt.splitlines():
        if "native-static-libs:" in line:
            syslibs = line.split("native-static-libs:", 1)[1].split()
    if check("rustc staticlib (--cfg pruv_c_probe)", rc == 0, txt):
        cpu = "-mcpu=native" if platform.machine().lower() in ("arm64", "aarch64") else "-march=native"
        gen_o, rt_o, probe = d / "gen.o", d / "runtime.o", d / "c_probe"
        rc1, t1 = run([CLANG, "-O2", cpu, "-Wno-override-module", "-c", str(ll), "-o", str(gen_o)])
        check("clang: .ll object", rc1 == 0, t1)
        rc2, t2 = run([CLANG, "-O2", cpu, "-std=c11", "-DPRUV_RUST_ALLOCATOR", "-c", str(RUNTIME_C), "-o", str(rt_o)])
        check("clang: runtime object", rc2 == 0, t2)
        if rc1 == 0 and rc2 == 0:
            rc, txt = run([CLANG, "-O1", "-Wall", "-std=c11", str(HERE / "c_probe.c"), str(lib), str(gen_o), str(rt_o),
                           "-o", str(probe)] + syslibs)
            if check("clang: link c_probe", rc == 0, txt):
                rc, txt = run([probe], 120)
                out = txt
                check("c_probe exit 0", rc == 0, "exit %d %s" % (rc, out))
                # the deliberate probe panic writes its message to stderr; "OK" is the probe's own last stdout line
                check("c_probe prints OK", "OK" in out.splitlines(), out)
                if rc != 0:
                    for line in out.splitlines():
                        if line.startswith("FAIL"):
                            fails.append("FAIL c_probe: " + line[5:])

    # ---- two programs with different package names (same entry name, same G6 builtins) in ONE binary -------------------
    import importlib.util
    spec = importlib.util.spec_from_file_location("check_two_programs", str(HERE / "check_two_programs.py"))
    two = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(two)
    two.run_case(work, rs, py, check, run, {"rustc": RUSTC, "clang": CLANG, "runtime_c": RUNTIME_C,
                                            "sel_cmd": sel_cmd, "py_cmd": py_cmd})

    for line in fails:
        print(line)
    if fails:
        return 1
    print("c_handle: two programs (packages twoa, twob; same entry name, same G6 builtins) in one binary, pruv_c text facts, Rust backend c == crate without extern \"C\", staticlib + C probe (ABI, provenance, "
          "lifetime, positional parameter names, [u8]/str/[bool] args, zero-length NULL, BAD_ARGS incl. address overflow, "
          "length limit, handle overlap, rebuild new_len, poison-before-argument-checks, panic with panicking payload "
          "Drop)%s: OK" % (", rs == py" if rs else ""))
    return 0


if __name__ == "__main__":
    args = sys.argv[1:]
    opts = {}
    for flag in ("--rs", "--py"):
        if flag in args:
            i = args.index(flag)
            if i + 1 >= len(args):
                print(__doc__)
                sys.exit(2)
            opts[flag] = args[i + 1]
            del args[i:i + 2]
    if len(args) != 1:
        print(__doc__)
        sys.exit(2)
    sys.exit(main(args[0], opts.get("--rs"), opts.get("--py")))
