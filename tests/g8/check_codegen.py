"""Inspect ordinary bundled Rust bodies on the selected compiler."""
from pathlib import Path
import re
import sys
import subprocess

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "pruvc"))
from pruvc import compile_file
from vparser import parse


def main(work, binary=None):
    work = Path(work) / "g8_codegen"
    work.mkdir()
    root = work / "root.pruv"
    root.write_text("mod std;\npub fn f(x: i64) -> i64 { return crate::std::min_i64(x, 0); }\n")
    names = [f.name for f in parse((ROOT / "stdlib/std.pruv").read_text()).functions]
    for verified, trust in ((True, False), (False, False), (False, True)):
        out = work / "out.rs"
        flags = ([] if verified else ["--no-verify"]) + (["--trust-callers"] if trust else [])
        if binary:
            c = subprocess.run([binary, str(root), "-o", str(out), "--backend", "rust", *flags],
                               capture_output=True, text=True, timeout=120)
            assert c.returncode == 0, c.stdout + c.stderr
        else:
            assert not compile_file(str(root), str(out), verify=verified, trust_callers=trust)
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
        if binary:
            c = subprocess.run([binary, str(root), "-o", str(out), "--backend", "rust",
                                "--also-llvm", str(ll), str(bindings), *flags],
                               capture_output=True, text=True, timeout=120)
            assert c.returncode == 0, c.stdout + c.stderr
        else:
            assert not compile_file(str(root), str(out), verify=verified, trust_callers=trust,
                                    also_llvm=(str(ll), str(bindings)))
        assert out.read_text() == text
        assert ll.exists() and bindings.exists()
        out.unlink()
    check_try(work, binary)
    print("12 ordinary bodies x 3 modes; reverse allocation; private std; e? as hand `?`: OK")


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


def check_try(work, binary):
    """`f(..)?;` and `let x: T = f(..)?;` (same error type) emit the hand `?`, not a match."""
    root, out = work / "try.pruv", work / "try.rs"
    root.write_text(TRY_ROOT)
    if binary:
        c = subprocess.run([binary, str(root), "-o", str(out), "--backend", "rust", "--no-verify"],
                           capture_output=True, text=True, timeout=120)
        assert c.returncode == 0, c.stdout + c.stderr
    else:
        assert not compile_file(str(root), str(out), verify=False)
    text = out.read_text()
    assert re.search(r"^\s*pruv_\w*_5_check\(a\)\?;$", text, re.M), text
    assert re.search(r"^\s*let h: i64 = pruv_\w*_4_half\(b\)\?;$", text, re.M), text
    assert re.search(r"^\s*t = pruv_\w*_4_half\(a\)\?;$", text, re.M), text
    assert "__pruv_try" not in text, text


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[3] if len(sys.argv) == 4 and sys.argv[2] == "--rs" else None)
