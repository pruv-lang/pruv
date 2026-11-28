#!/usr/bin/env python3
"""Check narrow host wrappers with a C caller and inspect both x64 targets."""
import argparse
import subprocess
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
INPUT = HERE / "winci" / "src" / "p12d_narrow.pruv"


def check(work, compiler):
    ir = work / "narrow.ll"
    exe = work / "caller"
    emit = [compiler, INPUT, "--backend", "llvm", "--emit", "ll", "-o", ir]
    for target in ("x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"):
        subprocess.run(emit + ["--target", target], check=True)
        lines = ir.read_text().splitlines()
        for name, width, ext in (("u8", "i8", "zeroext"), ("i16", "i16", "signext")):
            ret, param = width, width
            if target.endswith("gnu"):
                ret, param = f"{ext} {width}", f"{width} {ext}"
            assert f"define {ret} @pruv_abi_id_{name}({param} %a.x) {{" in lines, target
    for flags in ([], ["--trust-callers"]):
        subprocess.run(emit + flags, check=True)
        subprocess.run(["clang", "-O2", "-DPRUV_COUNT_ALLOCS", ir, ROOT / "pruvc/runtime.c",
                        HERE / "winci/c_caller_p12d.c", "-o", exe], check=True)
        normal = subprocess.run([exe], capture_output=True, text=True, check=True)
        balance = normal.stderr.split("pruv-alloc-balance: ")[1].split()
        assert int(balance[0]) > 0 and balance[0] == balance[1], normal.stderr
        for n in (-1, 2**40 + 1):
            child = subprocess.run([exe, str(n)], capture_output=True, text=True)
            assert child.returncode == 101 and not child.stdout, child
            assert f"pruv: invalid representation: {n} (array length)" in child.stderr, child
    print("narrow host ABI passed")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pruvc", type=Path, default=ROOT / "pruvc-rs/target/release/pruvc")
    parser.add_argument("--keep", type=Path)
    args = parser.parse_args()
    if args.keep:
        args.keep.mkdir(parents=True, exist_ok=True)
        check(args.keep, args.pruvc.resolve())
    else:
        with tempfile.TemporaryDirectory(prefix="p12d_") as tmp:
            check(Path(tmp), args.pruvc.resolve())
