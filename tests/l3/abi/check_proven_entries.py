"""Check P18b output parity and the checked-length Rust host API."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
import compiler_driver as CD


SOURCE = """\
pub fn copy_first(src: [i64], inout dst: [i64]) -> i64
  requires len(src) >= 1 && len(dst) >= 1
{
    dst[0] = src[0];
    return src[0];
}

pub fn text_len(text: str) -> i64 {
    return len(text);
}

pub fn array_len(src: [i64]) -> i64 {
    return len(src);
}

pub fn scalar(x: i64) -> i64 {
    return x;
}
"""

HOST = """\
include!("gen.rs");

fn main() {
    let src = [42, 9];
    let (mut guarded, mut proven) = ([1, 2], [1, 2]);
    let got = copy_first(&src, &mut guarded);
    let src_proof = PruvArr::new(&src);
    let dst_proof = PruvArr::new_mut(&mut proven);
    assert_eq!(got, pruv_proven::copy_first(src_proof, dst_proof));
    assert_eq!(guarded, proven);
    assert_eq!(array_len(&src), pruv_proven::array_len(src_proof));
    let text = "hé";
    assert_eq!(text_len(text), pruv_proven::text_len(PruvStr::new(text)));

    let err = std::panic::catch_unwind(|| {
        let mut dst = [0];
        pruv_proven::copy_first(PruvArr::new(&[]), PruvArr::new_mut(&mut dst));
    }).unwrap_err();
    assert!(err.downcast_ref::<&str>().is_some_and(|s| s.contains("pruv: requires violated"))
         || err.downcast_ref::<String>().is_some_and(|s| s.contains("pruv: requires violated")));

    #[cfg(target_pointer_width = "64")]
    {
        // A zero-sized slice can exercise the boundary without a large allocation.
        let slice = |n| unsafe { std::slice::from_raw_parts(std::ptr::NonNull::<()>::dangling().as_ptr(), n) };
        assert_eq!(PruvArr::new(slice(1usize << 40)).get().len(), 1usize << 40);
        let err = std::panic::catch_unwind(|| {
            let _ = PruvArr::new(slice((1usize << 40) + 1));
        }).unwrap_err();
        assert_eq!(err.downcast_ref::<String>().unwrap(),
                   "pruv: invalid representation: 1099511627777 (array length)");
    }
}
"""


def run(argv):
    return subprocess.run(argv, capture_output=True, text=True, timeout=300)


def compile_source(driver, source, out, flags=(), backend="rust"):
    result = run(driver.argv(str(source), str(out), list(flags), backend))
    assert result.returncode == 0, result.stdout + result.stderr
    return out.read_bytes()


def inline_attr(code, name):
    lines = code.splitlines()
    index = next(i for i, line in enumerate(lines) if line.lstrip().startswith(b"pub fn " + name + b"("))
    return lines[index - 1].lstrip()


def main(work, rs, py):
    work = Path(work).resolve() / "l3_abi_proven"
    work.mkdir(parents=True, exist_ok=True)
    drivers = (CD.make_driver("py", py), CD.make_driver("rs", rs))
    source = work / "app.pruv"
    source.write_text(SOURCE, encoding="utf-8")
    flag = "--proven-entries"
    rustc = shutil.which("rustc") or str(Path.home() / ".cargo/bin/rustc")
    for flags in ((), ("--trust-callers",), ("--no-verify",)):
        plain, proven = [], []
        for driver in drivers:
            plain.append(compile_source(driver, source, work / "plain.rs", flags))
            proven.append(compile_source(driver, source, work / "gen.rs", (*flags, flag)))
        assert plain[0] == plain[1], "plain output differs"
        assert proven[0] == proven[1], "proven output differs"
        assert proven[0].startswith(plain[0]), "guarded output changed"
        assert all(name not in plain[0] for name in (b"PruvArr", b"PruvStr", b"pruv_proven"))
        twins = proven[0].split(b"pub mod pruv_proven {", 1)[1]
        assert b"pruv_len_slow" not in twins and b"fn scalar(" not in twins
        assert b"let dst: &mut [i64] = dst.get_mut();" in twins
        assert b"let text: &str = text.get();" in twins
        for name in (b"copy_first", b"text_len", b"array_len"):
            assert inline_attr(plain[0], name) == inline_attr(twins, name), name
        if flags == ("--trust-callers",):
            assert b"pruv: requires violated" not in twins
            continue
        (work / "main.rs").write_text(HOST, encoding="utf-8")
        binary = work / "host"
        result = run([rustc, "--edition", "2021", "-C", "opt-level=3", str(work / "main.rs"), "-o", str(binary)])
        assert result.returncode == 0, result.stderr
        result = run([str(binary)])
        assert result.returncode == 0, result.stderr

    # Even an include! at the crate root cannot let safe host code bypass the constructor.
    (work / "main.rs").write_text(
        'include!("gen.rs");\nfn main() { let _ = &PruvArr::new(&[1]).0; }\n', encoding="utf-8")
    result = run([rustc, "--edition", "2021", str(work / "main.rs"), "-o", str(work / "host")])
    assert result.returncode != 0 and "E0616" in result.stderr, result.stderr

    manifest = work / "pruv.json"
    for mode in ("crate", "unsafe"):
        manifest.write_text(json.dumps({"name": "app", "version": "1.0.0", "root": source.name,
                                        "raw_entries": mode}), encoding="utf-8")
        flags = ("--manifest", str(manifest))
        plain = [compile_source(driver, source, work / "plain.rs", flags) for driver in drivers]
        proven = [compile_source(driver, source, work / "gen.rs", (*flags, flag)) for driver in drivers]
        assert plain[0] == plain[1] and proven[0] == proven[1], mode
        assert proven[0].startswith(plain[0]), mode
        if mode == "crate":
            assert proven[0] == plain[0], mode
        else:
            twins = proven[0][len(plain[0]):]
            assert b"pub mod pruv_raw {" in plain[0] and b"pub mod pruv_proven {" in twins, mode
            assert b"pub(crate) fn copy_first(" in twins, mode

    clash = work / "clash.pruv"
    clash.write_text("pub struct PruvArr { x: i64 }\n" + SOURCE, encoding="utf-8")
    diagnostics = []
    for driver in drivers:
        compile_source(driver, clash, work / "plain.rs")
        result = run(driver.argv(str(clash), str(work / "clash.rs"), [flag], "rust"))
        assert result.returncode == 1 and "conflicts with Rust name PruvArr" in result.stdout, result.stdout + result.stderr
        diagnostics.append(result.stdout)
        result = run(driver.argv(str(source), str(work / "app.ll"), [flag], "llvm"))
        assert result.returncode == 2 and result.stderr.strip().endswith(
            "--proven-entries requires --backend rust"), result.stdout + result.stderr
    assert diagnostics[0] == diagnostics[1], "clash diagnostics differ"
    print("P18b: parity in 3 compiler modes and 2 manifest modes, 2 host runs, privacy and diagnostics pass")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("work")
    parser.add_argument("--rs")
    parser.add_argument("--py")
    args = parser.parse_args()
    main(args.work, args.rs, args.py)
