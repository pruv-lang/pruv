"""Inspect std linkage/checks of every positive G8 root's LLVM artifacts on the Rust compiler.

    python tests/g8/check_llvm.py WORK [--rs PATH] [--vs-py] [--golden PATH] [--update-golden [--prune-golden]]

Default: for every root and mode the Rust compiler writes IR + bindings; the structural checks (std linkage, exports,
safe-mode checks, no foreign leftovers) run on that Rust output, and a digest of the IR and of the bindings is compared
with tests/golden/g8_llvm.txt (tests/golden.py). Keys: "g8_llvm|<root path>|ll-<mode>" and "...|bindings-<mode>".
Std names and the public names of a root come from regexes over the .pruv text (no parser; nothing from pruvc/ is imported).

PF: since bundle PF the Python compiler (pruvc/) is frozen. --vs-py is an opt-in extra (never used by CI): it also compiles
every case with the frozen Python compiler, runs the same structural checks on that output and requires byte-equal IR and
bindings (the old behaviour). Plain --vs-py does not touch the golden. --vs-py --update-golden is the capture path: it writes
the golden from the Rust output and prints every py/rs disagreement instead of stopping (exit 1 if there was one).
`--py PATH` is the old run_all call form: accepted only together with --vs-py.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tests"))
import golden  # noqa: E402   (no pruvc / z3 import at module level)

DEFAULT_RS = ROOT / "pruvc-rs" / "target" / "release" / ("pruvc.exe" if os.name == "nt" else "pruvc")
MODES = (("verified", []), ("safe", ["--no-verify"]), ("trust", ["--no-verify", "--trust-callers"]))


def std_names():
    text = (ROOT / "stdlib/std.pruv").read_text()
    names = re.findall(r"^(?:pub\s+)?fn\s+(\w+)", text, re.M)
    assert len(names) == 12, names
    return names


def public_names(src):
    """Top-level `pub fn` names of one root file (all fixtures declare them at column 0)."""
    return re.findall(r"^pub fn (\w+)", (ROOT / src).read_text(), re.M)


def compile_rs(binary, src, flags, ll, bindings):
    c = subprocess.run([binary, str(ROOT / src), "--backend", "llvm", "-o", str(ll),
                        "--rust-bindings", str(bindings), *flags],
                       capture_output=True, text=True, timeout=120)
    assert c.returncode == 0, (src, flags, c.stdout, c.stderr)


def compile_py(src, flags, ll, bindings):
    """Frozen Python compiler (--vs-py only)."""
    sys.path.insert(0, str(ROOT / "pruvc"))
    from pruvc import compile_file
    errors = compile_file(str(ROOT / src), str(ll), backend="llvm", rust_bindings=str(bindings),
                          verify=not flags, trust_callers="--trust-callers" in flags)
    assert not errors, (src, flags, [vars(e) for e in errors])


def structural(src, mode, flags, text, bind, names):
    definitions = re.findall(r"^define ([^\n]+)", text, re.M)
    for name in names:
        stem = "pruv_m_f_2_3_std_%d_%s" % (len(name), name)
        own = [line for line in definitions if "@" + stem + "__" in line]
        assert own and all(line.startswith(("internal ", "private ")) for line in own), (src, name)
        symbols = [re.search(r"@([^ (]+)\(", line).group(1) for line in own]
        assert len(symbols) == len(set(symbols)), (src, mode, name, "duplicate definitions", symbols)
        calls = set(re.findall(r"\bcall\b[^\n@]*@(" + re.escape(stem) + r"__[^ (]+)\(", text))
        assert calls <= set(symbols), (src, mode, name, "undefined calls", sorted(calls - set(symbols)))
        assert stem not in bind, (src, name)
        if flags and name in ("clamp_i64", "abs_i64", "sum_i64", "array_max_i64"):
            assert any("__pruv_entry(" in line for line in own), name
    exported = re.findall(r"^define (?!internal\b|private\b)[^\n@]*@([^ (]+)\(", text, re.M)
    assert sorted(exported) == sorted("pruv_abi_" + n for n in public_names(src)), (src, exported)
    assert sorted(re.findall(r'    fn (pruv_abi_\w+)\(', bind)) == sorted(exported), src
    if flags:
        assert "call void @pruv_panic_oob(" in text
        assert "call void @pruv_panic_arith(" in text
        assert "call { i64, i1 } @llvm.sadd.with.overflow.i64" in text
    # The catalog coexistence fixture intentionally has a G6 foreign operation.
    if "/catalog/" not in src:
        assert "pruv_foreign" not in text and "pruv_foreign" not in bind, src


def main(argv):
    ap = argparse.ArgumentParser(description="G8 LLVM artifact inspection on the Rust compiler")
    ap.add_argument("work", help="existing scratch directory")
    ap.add_argument("--rs", default=str(DEFAULT_RS), metavar="PATH", help="Rust compiler binary (default: release build)")
    ap.add_argument("--py", default=None, metavar="PATH", help="old call form; only with --vs-py")
    ap.add_argument("--vs-py", action="store_true",
                    help="opt-in, not CI: also compile with the frozen Python compiler and compare byte for byte")
    golden.add_args(ap)
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

    work = Path(args.work) / "g8_llvm"
    work.mkdir()
    names = std_names()
    inventory = json.loads((ROOT / "tests/g8/fixtures.json").read_text())
    roots = [e["root"] for key in ("frontend", "proof") for e in inventory[key]]
    roots += inventory["runtime"] + inventory["examples"]
    g = golden.Golden.from_args(args, "g8_llvm.txt", "check_llvm: LLVM IR and Rust bindings of every positive G8 root and mode",
                                tmp_dirs=[work])
    use_golden = not args.vs_py or args.update_golden   # plain --vs-py never reads or writes the golden
    capture = args.vs_py and args.update_golden
    disagreements = []
    byte_equal = 0
    for src in roots:
        for mode, flags in MODES:
            # The unsafe-input runtime root deliberately cannot verify.
            if "g8_r16_" in src and mode == "verified":
                continue
            rsll, rsbind = work / "rs.ll", work / "rs.rs"
            compile_rs(args.rs, src, flags, rsll, rsbind)
            text, bind = rsll.read_bytes().decode("utf-8"), rsbind.read_bytes().decode("utf-8")  # keep line endings
            structural(src, mode, flags, text, bind, names)
            if use_golden:
                g.check("g8_llvm|%s|ll-%s" % (src, mode), text, "ll")
                g.check("g8_llvm|%s|bindings-%s" % (src, mode), bind, "bindings")
            if args.vs_py:
                pyll, pybind = work / "py.ll", work / "py.rs"
                compile_py(src, flags, pyll, pybind)
                structural(src, mode, flags, pyll.read_text(), pybind.read_text(), names)
                same = pyll.read_bytes() == rsll.read_bytes() and pybind.read_bytes() == rsbind.read_bytes()
                if same:
                    byte_equal += 1
                elif capture:
                    disagreements.append("%s [%s]: IR %s, bindings %s" % (
                        src, mode, "equal" if pyll.read_bytes() == rsll.read_bytes() else "DIFFERS",
                        "equal" if pybind.read_bytes() == rsbind.read_bytes() else "DIFFERS"))
                else:
                    assert pyll.read_bytes() == rsll.read_bytes(), (src, mode, "IR mismatch")
                    assert pybind.read_bytes() == rsbind.read_bytes(), (src, mode, "bindings mismatch")
    ok = g.finish() if use_golden else True
    for d in disagreements:
        print("PY/RS DISAGREE " + d)
    if args.vs_py:
        print("%d roots: linkage, exports, safe checks on both compilers; %d IR/binding pairs byte-equal"
              % (len(roots), byte_equal))
    elif ok:
        print("%d roots: linkage, exports, safe checks on Rust output; IR/bindings match the golden" % len(roots))
    else:
        print("%d roots: structural checks passed on Rust output; GOLDEN CHECK FAILED" % len(roots))
    return 0 if ok and not disagreements else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
