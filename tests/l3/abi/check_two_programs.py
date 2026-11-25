"""L3 P10H: two LLVM-backend programs with different package names link into ONE binary.

PF: the Python compiler (pruvc/) is frozen since bundle PF; this module never imports it, and runs it only when
check_c_handle.py passes tools["vs_py"] (its opt-in --vs-py). Otherwise the Rust output goes to the golden object that
check_c_handle.py passes as tools["golden"] (keys "c_handle|two_programs/<pkg>|rust-bindings" and "...|llvm-ir").

Called by check_c_handle.py in the same run (`run_case`), so it runs for the selected compiler. Two small
programs, packages `twoa` and `twob`, both define the entry `calc` (same name, different result) and both lower the same
G6 builtins (vec_new / vec_push / vec_get on a Vec<i64>, so the .ll imports the shim adapters pruv_g6_v1_*, drop__vec__i
... and the bridge marker pruv_g6_bridge_v1 from its own Rust module), with raw_entries = "c".

Steps: (1) compile both (--backend llvm --rust-bindings); with --rs every file is also byte-identical to the Python
compiler's; (2) text facts: every program-specific non-internal global symbol of the .ll and of the Rust bindings carries
the package prefix (pruv_abi_*, pruv_g6_v1_*, pruv_g6_bridge_v1), no define of the .ll is a non-internal unprefixed
global; (3) ONE Rust staticlib containing both generated Rust modules (two `mod`s of one crate) -- the allocator hooks
`pruv_host_*` are program-independent unprefixed symbols the runtime calls, so only the first module keeps them (the
second is stripped of them; SPEC.md, LLVM symbol prefix) -- plus both .ll objects plus ONE runtime object; (4) one C
binary (c_two.c) calls twoa__pruv_h_calc and twob__pruv_h_calc and checks that each returns its own program's result.
"""
from pathlib import Path
import json
import platform
import re

HERE = Path(__file__).resolve().parent

PKGS = (("twoa", 1000, 7), ("twob", 100, 9))   # (package, multiplier, constant): calc(st, x) = x * mult + const


def source(mult, const):
    return ("pub fn calc(st: [i64], x: i64) -> i64 ! alloc\n"
            "{\n"
            "  let mut v: Vec<i64> = vec_new::<i64>();\n"
            "  v = vec_push::<i64>(move v, x);\n"
            "  v = vec_push::<i64>(move v, %d);\n"
            "  return vec_get::<i64>(v, 0) * %d + vec_get::<i64>(v, 1);\n"
            "}\n" % (const, mult))


# `#[allow(..)]` + `#[no_mangle] pub unsafe extern "C" fn pruv_host_*(..) { 4-space-indented body }`
HOST_HOOK = re.compile(r'(?:#\[allow\(dead_code, non_snake_case, unused\)\]\n)?#\[no_mangle\]\n'
                       r'pub (?:unsafe )?extern "C" fn pruv_host_[^\n]*\n(?:    [^\n]*\n)*\}\n')


def run_case(work, rs, py, check, run, tools):
    """work: scratch directory (created here); rs/py: as in check_c_handle.main; check(name, ok, detail) records a
    failure; run(cmd, timeout) -> (rc, output); tools: dict with rustc, clang, runtime_c, sel_cmd, py_cmd."""
    work = Path(work) / "two_programs"
    work.mkdir(parents=True)
    sel_cmd, py_cmd = tools["sel_cmd"], tools.get("py_cmd")
    vs_py = bool(rs and tools.get("vs_py") and py_cmd)   # opt-in frozen-reference comparison
    g = tools.get("golden")                              # tests/golden.py object, or None
    built = {}
    for pkg, mult, const in PKGS:
        base = work / pkg
        pdir = base / "app"
        pdir.mkdir(parents=True)
        (pdir / "pruv.json").write_text(json.dumps({"name": pkg, "version": "0.1.0", "root": "app.pruv",
                                                     "raw_entries": "c"}))
        (pdir / "app.pruv").write_text(source(mult, const))
        outs = {}
        for tag, cmd in [("sel", sel_cmd)] + ([("py", py_cmd)] if vs_py else []):
            d = base / tag
            d.mkdir()
            rc, txt = run(list(cmd) + [str(pdir / "app.pruv"), "-o", str(d / "out.ll"), "--no-verify", "--manifest",
                                       str(pdir / "pruv.json"), "--backend", "llvm", "--rust-bindings", str(d / "B.rs")], 300)
            if not check("two programs %s: %s compile" % (pkg, tag), rc == 0, "exit %d %s" % (rc, txt)):
                return
            outs[tag] = (d / "B.rs", d / "out.ll")
        if g is not None:
            for i, what in enumerate(("rust-bindings", "llvm-ir")):
                g.check("c_handle|two_programs/%s|%s" % (pkg, what), outs["sel"][i].read_text(), "%s %s" % (what, pkg))
        if vs_py:
            for i, what in enumerate(("B.rs", "out.ll")):
                same = outs["sel"][i].read_bytes() == outs["py"][i].read_bytes()
                if not same and tools.get("update_golden"):
                    print("PY/RS DISAGREE two programs %s %s" % (pkg, what))
                check("two programs %s: rs %s identical to python" % (pkg, what), same, "differs")
        built[pkg] = outs["sel"]

    # ---- text facts: the prefix on every program-specific global symbol, both sides ---------------------------------------
    for pkg, _, _ in PKGS:
        pre = pkg + "__"
        rs_text = built[pkg][0].read_text()
        ll_text = built[pkg][1].read_text()
        ll_syms = set(re.findall(r"@([A-Za-z0-9_.$]*(?:pruv_abi_|pruv_g6_v1_|pruv_g6_bridge_v1)[A-Za-z0-9_.$]*)", ll_text))
        rs_syms = set(re.findall(r"\b(?:fn|static)\s+([A-Za-z0-9_]*(?:pruv_abi_|pruv_g6_v1_|pruv_g6_bridge_v1)[A-Za-z0-9_]*)",
                                 rs_text))
        check("%s: the .ll uses the G6 bridge (test is not vacuous)" % pkg,
              any("pruv_g6_v1_" in s for s in ll_syms) and any("pruv_g6_bridge_v1" in s for s in ll_syms), sorted(ll_syms))
        check("%s: the .ll has pruv_abi_calc" % pkg, pre + "pruv_abi_calc" in ll_syms, sorted(ll_syms))
        check("%s: every .ll pruv_abi_/pruv_g6_ symbol is prefixed" % pkg,
              all(s.startswith(pre) for s in ll_syms), sorted(s for s in ll_syms if not s.startswith(pre)))
        check("%s: every Rust pruv_abi_/pruv_g6_ symbol is prefixed" % pkg,
              all(s.startswith(pre) for s in rs_syms), sorted(s for s in rs_syms if not s.startswith(pre)))
        check("%s: every symbol the .ll names is named by the Rust module too (defined or declared there)" % pkg,
              ll_syms <= rs_syms, sorted(ll_syms - rs_syms))
        # every non-internal define of the .ll is a prefixed program symbol
        for line in ll_text.splitlines():
            if line.startswith("define ") and " internal " not in line and " private " not in line:
                m = re.search(r"@([A-Za-z0-9_.$]+)\(", line)
                check("%s: non-internal define is prefixed: %s" % (pkg, line[:80]),
                      m is not None and m.group(1).startswith(pre), line)
        # no unprefixed #[no_mangle] program symbol in the Rust module except the program-independent hooks
        for name in re.findall(r'#\[no_mangle\]\n\s*pub (?:unsafe )?(?:extern "C" fn|static) ([A-Za-z0-9_]+)', rs_text):
            check("%s: #[no_mangle] %s is prefixed or a shared hook" % (pkg, name),
                  name.startswith(pre) or name.startswith("pruv_host_"), name)

    # ---- one Rust crate with both modules, one runtime, one C binary --------------------------------------------------------
    d = work / "link"
    d.mkdir()
    a_rs = built["twoa"][0]
    b_text = built["twob"][0].read_text()
    b_stripped, n_hooks = HOST_HOOK.subn("", b_text)
    check("twob: the shared pruv_host_* hooks were found", n_hooks > 0 and "fn pruv_host_" not in b_stripped,
          "%d removed" % n_hooks)
    # the hooks are program-independent (they pair with the one shared runtime): dropping twob's copy is sound only if
    # both modules emit byte-identical hook text
    a_hooks = HOST_HOOK.findall(a_rs.read_text())
    b_hooks = HOST_HOOK.findall(b_text)
    check("the pruv_host_* hooks of twoa and twob are byte-identical", a_hooks == b_hooks and len(a_hooks) == n_hooks,
          "%d vs %d" % (len(a_hooks), len(b_hooks)))
    b_rs = d / "B_without_hooks.rs"
    b_rs.write_text(b_stripped)
    crate = d / "two.rs"
    crate.write_text('#[path = "%s"]\npub mod pa;\n#[path = "%s"]\npub mod pb;\n' % (a_rs.resolve().as_posix(), b_rs.resolve().as_posix()))
    lib = d / "libpruv_two.a"
    rc, txt = run([tools["rustc"], "--edition", "2021", "-O", "--crate-type", "staticlib", "--crate-name", "pruv_two",
                   "-A", "warnings", "--print", "native-static-libs", str(crate), "-o", str(lib)])
    syslibs = []
    for line in txt.splitlines():
        if "native-static-libs:" in line:
            syslibs = line.split("native-static-libs:", 1)[1].split()
    if not check("two programs: one rustc staticlib with both modules", rc == 0, txt):
        return
    cpu = "-mcpu=native" if platform.machine().lower() in ("arm64", "aarch64") else "-march=native"
    objs = []
    for pkg, _, _ in PKGS:
        o = d / (pkg + ".o")
        rc1, t1 = run([tools["clang"], "-O2", cpu, "-Wno-override-module", "-c", str(built[pkg][1]), "-o", str(o)])
        if not check("two programs: clang .ll object %s" % pkg, rc1 == 0, t1):
            return
        objs.append(o)
    rt_o = d / "runtime.o"
    rc2, t2 = run([tools["clang"], "-O2", cpu, "-std=c11", "-DPRUV_RUST_ALLOCATOR", "-c", str(tools["runtime_c"]),
                   "-o", str(rt_o)])
    if not check("two programs: clang runtime object (one)", rc2 == 0, t2):
        return
    exe = d / "c_two"
    rc, txt = run([tools["clang"], "-O1", "-Wall", "-std=c11", str(HERE / "c_two.c"), str(lib)] + [str(o) for o in objs] +
                  [str(rt_o), "-o", str(exe)] + syslibs)
    if not check("two programs: link ONE binary (no duplicate symbol)", rc == 0, txt):
        return
    rc, txt = run([exe], 120)
    check("two programs: c_two exit 0", rc == 0, "exit %d %s" % (rc, txt))
    check("two programs: c_two prints OK", "OK" in txt.splitlines(), txt)
    if rc != 0:
        for line in txt.splitlines():
            if line.startswith("FAIL"):
                check("two programs: c_two " + line[5:], False, "")
