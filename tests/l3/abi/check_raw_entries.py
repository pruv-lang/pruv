"""L3 P09: manifest `raw_entries` ("crate" / "unsafe") on the selected compiler.

usage: check_raw_entries.py WORK [--rs PATH] [--py PATH]
  --rs PATH: the Rust driver (default: the Python compiler); every generated file must also be byte-identical to the
             Python compiler's output
  --py PATH: the pruvc.py to run (default: pruvc/pruvc.py of this repository)
Checks (a) text facts for key x flags x backend (incl. P13a.10: poison flag only under --no-verify or llvm, and for a
par / foreign / printing program), (b) compile-time rejection with rustc, (c) panic injection with a host
binary, (d) a user `struct PruvStore` next to a no-requires entry (names must not collide with pruv_raw's own items).
Prints one FAIL line per failing case; exit 0 only if every case passes. Needs rustc (PATH, else ~/.cargo/bin)."""
from pathlib import Path
import json
import os
import re
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
RUSTC = shutil.which("rustc") or os.path.expanduser("~/.cargo/bin/rustc")

# two host entries: an inout writer and a reader (both have requires, so both are checked wrappers)
SRC_OK = ("pub fn put(inout st: [i64], x: i64) -> ()\n"
          "  requires len(st) >= 1\n"
          "{\n  st[0] = x;\n}\n"
          "\n"
          "pub fn get(st: [i64], i: i64) -> i64\n"
          "  requires 0 <= i && i < len(st)\n"
          "{\n  return st[i];\n}\n")
ENTRIES = ("put", "get")
# a user type named like pruv_raw's store, and an entry without requires (a plain forwarder)
SRC_CLASH = ("pub struct PruvStore { x: i64 }\n"
             "\n"
             "pub fn read(v: PruvStore) -> i64 {\n  return v.x;\n}\n"
             "\n"
             "pub fn get(st: [i64], i: i64) -> i64\n"
             "  requires 0 <= i && i < len(st)\n"
             "{\n  return st[i];\n}\n")
CLASH_USE = "pub fn f() -> i64 { let v: Vec<i64> = vec![4]; unsafe { gen::pruv_raw::read(gen::PruvStore { x: 3 }) + gen::pruv_raw::get(&v, 0) } }\n"
# the panic package: put_far writes st[0] and then st[i]; the verifier rejects it, so it is built with --no-verify
SRC_PANIC = SRC_OK + ("\n"
                      "pub fn put_far(inout st: [i64], i: i64) -> ()\n"
                      "  requires len(st) >= 1\n"
                      "{\n  st[0] = 9;\n  st[i] = 1;\n}\n")

# P13a.10: an entry that reaches foreign code may unwind inside its body (G6 boundary checks, trusted Rust), so the store
# keeps its poison flag even in a verified build
SRC_FOREIGN = ("trusted rust fn rust_min(a: i64, b: i64) -> i64\n"
               "  ensures result <= a && result <= b\n"
               "  ensures result == a || result == b\n"
               "  = \"::std::cmp::min\";\n"
               "\n"
               "pub fn put_min(inout st: [i64], x: i64) -> ()\n"
               "  requires len(st) >= 1\n"
               "{\n  st[0] = rust_min(st[0], x);\n}\n")

# P13a.10 (audit): print lowers to println!, which panics when stdout fails, so a printing program keeps the flag
SRC_PRINT = ("pub fn put_print(inout st: [i64], x: i64) -> () ! io\n"
             "  requires len(st) >= 1\n"
             "{\n  st[0] = x;\n  print(x);\n}\n")

FLAGS = (("default", []), ("trust", ["--trust-callers"]), ("noverify", ["--no-verify"]))
KEYS = (None, "crate", "unsafe")
BACKENDS = ("rust", "llvm")

CONSUMERS = {
    "call_put": "pub fn f() { let mut v: Vec<i64> = vec![0; 2]; gen::put(&mut v, 1); }\n",
    "raw_put_safe": "pub fn f() { let mut v: Vec<i64> = vec![0; 2]; gen::pruv_raw::put(&mut v, 1); }\n",
    "raw_put_unsafe": "pub fn f() { let mut v: Vec<i64> = vec![0; 2]; unsafe { gen::pruv_raw::put(&mut v, 1); } }\n",
    "store_field": "pub fn f() { let s = gen::pruv_raw::PruvStore::<Vec<i64>>::new(vec![1]); let _x = &s.data; }\n",
}
# (consumer, key) -> expected: "ok" or an error code; keys absent from the table are not checked
EXPECT = {
    ("call_put", None): "ok", ("call_put", "crate"): "E0603", ("call_put", "unsafe"): "E0603",
    ("raw_put_safe", "unsafe"): "E0133", ("raw_put_safe", "crate"): "E0425",
    ("raw_put_unsafe", "unsafe"): "ok", ("raw_put_unsafe", "crate"): "E0425",
    ("store_field", "crate"): "E0616", ("store_field", "unsafe"): "E0616",
}


def main(work, rs=None, py=None):
    work = Path(work) / "l3_abi_raw"
    work.mkdir(parents=True)
    pyc = str(Path(py).resolve()) if py else str(ROOT / "pruvc" / "pruvc.py")
    py_cmd = [sys.executable, pyc]
    sel_cmd = [rs] if rs else py_cmd
    fails = []
    counter = [0]

    def check(name, ok, detail=""):
        if not ok:
            fails.append("FAIL %s: %s" % (name, str(detail).strip()[-400:]))

    def compile_with(cmd, pdir, out, flags, backend):
        args = list(cmd) + [str(pdir / "app.pruv"), "-o", str(out)] + list(flags) + ["--manifest", str(pdir / "pruv.json")]
        if backend == "llvm":
            args += ["--backend", "llvm"]
        c = subprocess.run(args, capture_output=True, text=True, timeout=300)
        return c.returncode, c.stdout + c.stderr

    def build(label, src, key, flags, backend):
        """compile one package; returns (generated Rust path or None, message). With --rs the Python output must match."""
        counter[0] += 1
        base = work / ("b%d" % counter[0])
        pdir = base / "app"
        pdir.mkdir(parents=True)
        man = {"name": "app", "version": "0.1.0", "root": "app.pruv"}
        if key is not None:
            man["raw_entries"] = key
        (pdir / "pruv.json").write_text(json.dumps(man))
        (pdir / "app.pruv").write_text(src)
        exts = (".rs", ".ll") if backend == "llvm" else (".rs",)
        outs = {}
        runs = [("sel", sel_cmd)] + ([("py", py_cmd)] if rs else [])
        for tag, cmd in runs:
            d = base / tag
            d.mkdir()
            rc, txt = compile_with(cmd, pdir, d / ("out.ll" if backend == "llvm" else "out.rs"), flags, backend)
            if rc != 0:
                check("%s: %s compile" % (label, tag), False, "exit %d %s" % (rc, txt))
                return None, txt
            outs[tag] = {e: (d / ("out" + e)) for e in exts}
        if rs:
            for e in exts:
                a, b = outs["sel"][e].read_bytes(), outs["py"][e].read_bytes()
                check("%s: rs output identical to python (%s)" % (label, e), a == b, "differs")
        return outs["sel"][".rs"], ""

    # ---- (a) text facts ---------------------------------------------------------------------------------------------
    for key in KEYS:
        for fname, flags in FLAGS:
            for backend in BACKENDS:
                label = "text %s/%s/%s" % (key or "nokey", fname, backend)
                path, _ = build(label, SRC_OK, key, flags, backend)
                if path is None:
                    continue
                t = path.read_text()
                top_pub = re.findall(r"^pub fn \w+", t, re.M)
                if backend == "rust":
                    # T0 (SPEC 7, host length bound): only manifest "crate" leaves the 2^40 length check to the
                    # same-crate adapter; every other mode keeps it in each entry, also under --trust-callers
                    want_guard = key != "crate"
                    check(label + ": length check " + ("kept" if want_guard else "left to the adapter"),
                          t.count("fn pruv_len_slow(") == (len(ENTRIES) if want_guard else 0), t.count("fn pruv_len_slow("))
                if key is None:
                    for n in ENTRIES:
                        check(label + ": pub fn " + n, re.search(r"^pub fn %s\(" % n, t, re.M) is not None)
                    check(label + ": no pruv_raw", "pruv_raw" not in t)
                    continue
                check(label + ": no top-level pub fn", not top_pub, top_pub)
                for n in ENTRIES:
                    check(label + ": pub(crate) fn " + n, re.search(r"^pub\(crate\) fn %s\(" % n, t, re.M) is not None)
                check(label + ": pub mod pruv_raw", re.search(r"^pub mod pruv_raw \{", t, re.M) is not None)
                check(label + ": PruvStore", "pub struct PruvStore<T>" in t and "pub struct PruvPoisoned" in t)
                # P13a.10: a verified Rust-backend program with no foreign or runtime code cannot unwind inside an entry
                # body, so its store has no poison flag; --no-verify and the LLVM backend keep the flag
                want_flag = fname == "noverify" or backend == "llvm"
                check(label + ": poison flag " + ("kept" if want_flag else "dropped"),
                      ("poisoned: bool" in t) == want_flag and ("Ok(f(&mut self.data))" in t) != want_flag)
                unsafe_fns = re.findall(r"^        pub unsafe fn (\w+)\(", t, re.M)
                safety = t.count("/// # Safety")
                if key == "unsafe":
                    check(label + ": pub unsafe fn per entry", unsafe_fns == list(ENTRIES), unsafe_fns)
                    check(label + ": # Safety per entry", safety == len(ENTRIES), safety)
                else:
                    check(label + ": no unsafe forwarders", not unsafe_fns and safety == 0, unsafe_fns)

    # T0: "c" shares the "crate" Rust output except the length checks, which stay in every entry
    for fname, flags in FLAGS:
        label = "text c/%s/rust" % fname
        path, _ = build(label, SRC_OK, "c", flags, "rust")
        if path is not None:
            t = path.read_text()
            check(label + ": length check kept", t.count("fn pruv_len_slow(") == len(ENTRIES), t.count("fn pruv_len_slow("))
            check(label + ": pub(crate) entries", all(re.search(r"^pub\(crate\) fn %s\(" % n, t, re.M) for n in ENTRIES))

    for key in ("crate", "unsafe"):
        for what, src in (("foreign", SRC_FOREIGN), ("print", SRC_PRINT)):
            label = "text %s %s" % (what, key)
            path, _ = build(label, src, key, [], "rust")
            if path is not None:
                check(label + ": poison flag kept", "poisoned: bool" in path.read_text())

    # ---- (b) compile-time rejection ---------------------------------------------------------------------------------
    def rustc(args):
        try:
            c = subprocess.run([RUSTC] + args, capture_output=True, text=True, timeout=300)
        except OSError as e:
            return 127, repr(e)
        return c.returncode, c.stdout + c.stderr

    for key in KEYS:
        for backend in BACKENDS:
            label = "rustc %s/%s" % (key or "nokey", backend)
            path, _ = build(label, SRC_OK, key, [], backend)
            if path is None:
                continue
            d = path.parent
            gen = d / "gen.rs"
            shutil.copyfile(path, gen)
            rlib = d / "libgen.rlib"
            rc, txt = rustc(["--crate-type", "rlib", "--crate-name", "gen", "--edition", "2021", "-A", "warnings",
                             str(gen), "-o", str(rlib)])
            check(label + ": generated code compiles as an rlib", rc == 0, txt)
            if rc != 0:
                continue
            for cname, ctext in CONSUMERS.items():
                want = EXPECT.get((cname, key))
                if want is None:
                    continue
                cfile = d / (cname + ".rs")
                cfile.write_text(ctext)
                rc, txt = rustc(["--crate-type", "lib", "--crate-name", "cons", "--edition", "2021", "-A", "warnings",
                                 "--extern", "gen=" + str(rlib), "--emit=metadata", "--out-dir", str(d / ("o_" + cname)),
                                 str(cfile)])
                if want == "ok":
                    check("%s: %s compiles" % (label, cname), rc == 0, txt)
                else:
                    check("%s: %s fails with %s" % (label, cname, want), rc != 0 and ("error[%s]" % want) in txt, txt)

    # ---- (d) name clash with pruv_raw items, no-requires entry ------------------------------------------------------
    for key in ("crate", "unsafe"):
        for backend in BACKENDS:
            label = "clash %s/%s" % (key, backend)
            path, _ = build(label, SRC_CLASH, key, [], backend)
            if path is None:
                continue
            t = path.read_text()
            fwd = re.findall(r"^        pub unsafe fn (\w+)\(", t, re.M)
            check(label + ": forwarders", fwd == (["read", "get"] if key == "unsafe" else []), fwd)
            d = path.parent
            shutil.copyfile(path, d / "gen.rs")
            rlib = d / "libgen.rlib"
            rc, txt = rustc(["--crate-type", "rlib", "--crate-name", "gen", "--edition", "2021", "-A", "warnings",
                             str(d / "gen.rs"), "-o", str(rlib)])
            check(label + ": generated code compiles as an rlib", rc == 0, txt)
            if rc != 0 or key != "unsafe":
                continue
            (d / "use.rs").write_text(CLASH_USE)
            rc, txt = rustc(["--crate-type", "lib", "--crate-name", "cons", "--edition", "2021", "-A", "warnings",
                             "--extern", "gen=" + str(rlib), "--emit=metadata", "--out-dir", str(d / "o_use"),
                             str(d / "use.rs")])
            check(label + ": consumer calls both forwarders", rc == 0, txt)

    # ---- (c) panic injection (Rust backend, crate mode, --no-verify) -------------------------------------------------
    label = "panic host"
    path, _ = build(label, SRC_PANIC, "crate", ["--no-verify"], "rust")
    if path is not None:
        d = work / "host"
        d.mkdir()
        shutil.copyfile(path, d / "gen.rs")
        shutil.copyfile(HERE / "raw_host.rs", d / "main.rs")
        rc, txt = rustc(["--edition", "2021", "-A", "warnings", str(d / "main.rs"), "-o", str(d / "host")])
        check(label + ": host builds", rc == 0, txt)
        if rc == 0:
            c = subprocess.run([str(d / "host")], capture_output=True, text=True, timeout=120)
            want = ("partial_write: poisoned, refused, dropped\n"
                    "alloc_failure: poisoned, refused, dropped\n"
                    "host_panic: poisoned, refused, dropped\n"
                    "normal: ok\n"
                    "summary: 3 poisoned, 1 clear\n")
            check(label + ": exit 0", c.returncode == 0, "exit %d %s" % (c.returncode, c.stderr))
            check(label + ": output", c.stdout == want, c.stdout)

    for line in fails:
        print(line)
    if fails:
        return 1
    print("raw_entries: text facts, rustc visibility/unsafe/privacy, name clash, panic poisoning%s: OK" % (", rs == py" if rs else ""))
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
