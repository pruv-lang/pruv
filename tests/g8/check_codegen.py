"""Inspect ordinary bundled Rust bodies on the Rust compiler.

    python tests/g8/check_codegen.py WORK [--rs PATH] [--vs-py] [--py IGNORED]

Default: every compile runs the Rust binary (--rs, default pruvc-rs/target/release/pruvc). The std function names come
from a small regex over stdlib/std.pruv (`fn NAME(` at line start, public or not), so no parser and nothing from pruvc/
is imported.

PF: since bundle PF the Python compiler (pruvc/) is frozen. --vs-py is an opt-in extra that runs the same checks through
the frozen Python compile_file as well (never used by CI); pruvc/ is imported only inside that code path. `--py PATH`
is the old run_all call form: it is accepted only together with --vs-py (PATH itself is not needed).
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_RS = ROOT / "pruvc-rs" / "target" / "release" / ("pruvc.exe" if os.name == "nt" else "pruvc")


def std_names():
    """Function names of the std bundle, in source order."""
    text = (ROOT / "stdlib/std.pruv").read_text()
    names = re.findall(r"^(?:pub\s+)?fn\s+(\w+)", text, re.M)
    assert len(names) == 12 and "reverse_i64" in names and "min_i64" in names, names
    return names


def rs_compiler(binary):
    def compile_(root, out, verified, trust, also_llvm=None):
        flags = ([] if verified else ["--no-verify"]) + (["--trust-callers"] if trust else [])
        argv = [binary, str(root), "-o", str(out), "--backend", "rust"]
        if also_llvm:
            argv += ["--also-llvm", str(also_llvm[0]), str(also_llvm[1])]
        c = subprocess.run(argv + flags, capture_output=True, text=True, timeout=120)
        assert c.returncode == 0, c.stdout + c.stderr
    return compile_


def py_compiler():
    """Frozen Python compiler (--vs-py only)."""
    sys.path.insert(0, str(ROOT / "pruvc"))
    from pruvc import compile_file

    def compile_(root, out, verified, trust, also_llvm=None):
        extra = {"also_llvm": (str(also_llvm[0]), str(also_llvm[1]))} if also_llvm else {}
        assert not compile_file(str(root), str(out), verify=verified, trust_callers=trust, **extra)
    return compile_


def run_checks(work, compile_, names):
    work.mkdir(parents=True)
    root = work / "root.pruv"
    root.write_text("mod std;\npub fn f(x: i64) -> i64 { return crate::std::min_i64(x, 0); }\n")
    for verified, trust in ((True, False), (False, False), (False, True)):
        out = work / "out.rs"
        compile_(root, out, verified, trust)
        text = out.read_text()
        for name in names:
            stem = "pruv_m_f_2_3_std_%d_%s" % (len(name), name)
            assert re.search(r"^fn " + stem + r"(?:__pruv_body)?\(", text, re.M), name
            assert not re.search(r"pub fn " + stem, text), name
            assert not re.search(r"pub use[^;]*\b" + stem + r"\b", text), name
        assert "#[no_mangle]" not in text
        assert "std::cmp::min" not in text and "std::cmp::max" not in text
        assert "ensures violated" not in text
        assert not (work / "out.rs.foreign.json").exists()
        reverse = text.split("fn pruv_m_f_2_3_std_11_reverse_i64(")[1]
        assert reverse.count("pruv_alloc(") + reverse.count("vec![") == 1
        assert ".clone()" not in reverse and ".to_vec()" not in reverse
        if not verified:
            for name in ("clamp_i64", "abs_i64", "sum_i64", "array_max_i64"):
                assert "pruv: requires violated: crate::std::" + name in text
        ll, bindings = work / "out.ll", work / "bindings.rs"
        compile_(root, out, verified, trust, also_llvm=(ll, bindings))
        assert out.read_text() == text
        assert ll.exists() and bindings.exists()
        out.unlink()
    check_try(work, compile_)


TRY_ROOT = """pub struct Fault { code: i64 }
fn check(x: i64) -> Result<(), Fault> {
  if x < 0 { return Err::<(), Fault>(Fault { code: x }); }
  return Ok::<(), Fault>(());
}
fn half(x: i64) -> Result<i64, Fault> {
  if x < 0 { return Err::<i64, Fault>(Fault { code: x }); }
  return Ok::<i64, Fault>(x / 2);
}
pub fn both(a: i64, b: i64) -> Result<i64, Fault> {
  check(a)?;
  let h: i64 = half(b)?;
  let mut t: i64 = 0;
  t = half(a)?;
  return Ok::<i64, Fault>(h + t);
}
"""


def check_try(work, compile_):
    """`f(..)?;` and `let x: T = f(..)?;` (same error type) emit the hand `?`, not a match."""
    root, out = work / "try.pruv", work / "try.rs"
    root.write_text(TRY_ROOT)
    compile_(root, out, False, False)
    text = out.read_text()
    assert re.search(r"^\s*pruv_\w*_5_check\(a\)\?;$", text, re.M), text
    assert re.search(r"^\s*let h: i64 = pruv_\w*_4_half\(b\)\?;$", text, re.M), text
    assert re.search(r"^\s*t = pruv_\w*_4_half\(a\)\?;$", text, re.M), text
    assert "__pruv_try" not in text, text


def main(argv):
    ap = argparse.ArgumentParser(description="G8 codegen inspection on the Rust compiler")
    ap.add_argument("work", help="existing scratch directory")
    ap.add_argument("--rs", default=str(DEFAULT_RS), metavar="PATH", help="Rust compiler binary (default: release build)")
    ap.add_argument("--py", default=None, metavar="PATH", help="old call form; only with --vs-py")
    ap.add_argument("--vs-py", action="store_true",
                    help="opt-in, not CI: also run the checks through the frozen Python compiler")
    try:
        args = ap.parse_args(argv)
    except SystemExit as e:
        return 2 if e.code else 0
    if args.py is not None and not args.vs_py:
        print("--py runs the frozen Python compiler: add --vs-py", file=sys.stderr)
        return 2
    if not os.path.isfile(args.rs):
        print("Rust compiler not found: %s" % args.rs, file=sys.stderr)
        return 2
    names = std_names()
    base = Path(args.work)
    run_checks(base / "g8_codegen", rs_compiler(args.rs), names)
    if args.vs_py:
        run_checks(base / "g8_codegen_py", py_compiler(), names)
    print("12 ordinary bodies x 3 modes; reverse allocation; private std; e? as hand `?`: OK"
          + (" (rs and py)" if args.vs_py else " (rs)"))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
