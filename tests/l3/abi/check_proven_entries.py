"""Check P18b/P18c output parity and the checked-length and by-value Rust host APIs."""
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

pub fn pair(x: i64, inout out: [i64]) -> i64
  requires len(out) >= 2 && x < 9223372036854775807
{
    out[0] = x;
    out[1] = x + 1;
    return 7;
}

pub fn not_last(inout out: [i64], x: i64) -> i64
  requires len(out) >= 1
{
    out[0] = x;
    return x;
}

pub fn bytes_out(inout out: [u8]) -> i64
  requires len(out) >= 1
{
    out[0] = 1;
    return 0;
}

pub fn named_k(K: i64, inout out: [i64]) -> i64
  requires len(out) >= 1
{
    out[0] = K;
    return K;
}

pub fn only_out(inout out: [i64]) -> i64
  requires len(out) >= 1
{
    out[0] = 3;
    return 4;
}

pub fn guarded(x: i64) -> i64
  requires x >= 0
{
    return 7;
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
    assert_eq!(pruv_proven::val::pair::<2>(5), (7, [5, 6]));
    assert_eq!(pruv_proven::val::copy_first::<3>(src_proof), (42, [42, 0, 0]));
    assert_eq!(pruv_proven::val::named_k::<2>(9), (9, [9, 0]));
    assert_eq!(pruv_proven::val::only_out::<1>(), (4, [3]));

    let err = std::panic::catch_unwind(|| {
        let mut dst = [0];
        pruv_proven::copy_first(PruvArr::new(&[]), PruvArr::new_mut(&mut dst));
    }).unwrap_err();
    assert!(err.downcast_ref::<&str>().is_some_and(|s| s.contains("pruv: requires violated"))
         || err.downcast_ref::<String>().is_some_and(|s| s.contains("pruv: requires violated")));
    let err = std::panic::catch_unwind(|| pruv_proven::val::pair::<1>(5)).unwrap_err();
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


# The forwarders shadow host macro_rules! for every macro the generator emits (TIDY 4.13 audit).
WRAP_MACROS = (("assert", "core"), ("eprintln", "std"), ("format_args", "core"), ("matches", "core"),
               ("panic", "core"), ("println", "std"), ("unreachable", "core"), ("vec", "std"))
WRAP_HEAD = b"mod pruv_impl {\n" + b"".join(
    b"#[allow(unused_macros)]\nmacro_rules! %s { ($($t:tt)*) => { %s::%s!($($t)*) } }\n" % (m.encode(), c.encode(), m.encode())
    for m, c in WRAP_MACROS)
WRAP_TAIL = b"}\n#[allow(unused_imports)]\npub use self::pruv_impl::*;\n"


def unwrap(code):
    """The program text inside the private `pruv_impl` module that hides the unchecked bodies (TIDY 4.13)."""
    assert code.startswith(WRAP_HEAD) and code.endswith(WRAP_TAIL), code[:80] + code[-120:]
    return code[len(WRAP_HEAD):-len(WRAP_TAIL)]


def host_rejects(rustc, work, host, codes):
    """Compile a host that must not build; an error with one of codes must point into the host (main.rs)."""
    (work / "main.rs").write_text(host, encoding="utf-8")
    result = run([rustc, "--edition", "2021", "--error-format=json", str(work / "main.rs"), "-o", str(work / "host")])
    assert result.returncode != 0, "host built:\n" + host
    for line in result.stderr.splitlines():
        d = json.loads(line) if line.startswith("{") else {}
        spans = [sp for sp in d.get("spans", ()) if sp["is_primary"]]
        if d.get("level") == "error" and (d.get("code") or {}).get("code") in codes and spans \
                and all(Path(sp["file_name"]).name == "main.rs" for sp in spans):
            return "%s at main.rs:%d: %s" % (d["code"]["code"], spans[0]["line_start"], d["message"])
    raise AssertionError(result.stderr)


# TIDY 4.13: a host names an unchecked inner body (skipping the length and requires checks); it must not compile,
# whether the generated file is a crate-root include! or a #[path] module, with or without --proven-entries.
BODY_REJECTS = (
    ('include!("gen.rs");\nfn main() { let _ = copy_first__pruv_body(&[], &mut []); }\n', ("E0425",)),
    ('include!("gen.rs");\nfn main() { let _ = pruv_impl::copy_first__pruv_body(&[], &mut []); }\n', ("E0603",)),
    ('#[path = "gen.rs"] mod gen;\nfn main() { let _ = gen::copy_first__pruv_body(&[], &mut []); }\n',
     ("E0425", "E0603")),
    ('#[path = "gen.rs"] mod gen;\nfn main() { let _ = gen::pruv_impl::copy_first__pruv_body(&[], &mut []); }\n',
     ("E0603",)),
)

# A host macro_rules! is in textual scope inside the generated file; one that lets a failed requires fall through
# must not take over the check (TIDY 4.13 audit). Both host styles, the requires message must still be raised.
MACRO_HOST = """\
macro_rules! panic { ("{}", $s:expr) => { {} }; ($($t:tt)*) => { ::std::panic!($($t)*) } }
%s

fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    // guarded's body returns 7 safely, so a check that falls through fails unwrap_err() instead of running UB.
    let err = std::panic::catch_unwind(|| %sguarded(-1)).unwrap_err();
    assert!(err.downcast_ref::<&str>().is_some_and(|s| s.contains("pruv: requires violated"))
         || err.downcast_ref::<String>().is_some_and(|s| s.contains("pruv: requires violated")));
}
"""
MACRO_HOSTS = (MACRO_HOST % ('include!("gen.rs");', ""), MACRO_HOST % ('#[path = "gen.rs"] mod gen;', "gen::"))

# The checked entry and the proven twins stay reachable from a #[path] host (the include! host is HOST above).
PATH_HOST = """\
#[path = "gen.rs"] mod gen;

fn main() {
    let (src, mut dst) = ([42, 9], [0]);
    assert_eq!(gen::copy_first(&src, &mut dst), 42);
    assert_eq!(gen::pruv_proven::copy_first(gen::PruvArr::new(&src), gen::PruvArr::new_mut(&mut dst)), 42);
    assert_eq!(gen::pruv_proven::val::pair::<2>(5), (7, [5, 6]));
    let err = std::panic::catch_unwind(|| gen::copy_first(&[], &mut [0])).unwrap_err();
    assert!(err.downcast_ref::<&str>().is_some_and(|s| s.contains("pruv: requires violated"))
         || err.downcast_ref::<String>().is_some_and(|s| s.contains("pruv: requires violated")));
}
"""


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
        plain, proven = [unwrap(code) for code in plain], [unwrap(code) for code in proven]
        assert proven[0].startswith(plain[0]), "guarded output changed"
        assert all(name not in plain[0] for name in (b"PruvArr", b"PruvStr", b"pruv_proven"))
        twins = proven[0].split(b"pub mod pruv_proven {", 1)[1]
        assert b"pruv_len_slow" not in twins and b"fn scalar(" not in twins
        assert b"let dst: &mut [i64] = dst.get_mut();" in twins
        assert b"let text: &str = text.get();" in twins
        assert b"pub mod val {" in twins
        assert b"fn pair<const PRUV__K: usize>(x: i64) -> (i64, [i64; PRUV__K])" in twins
        assert b"fn copy_first<const PRUV__K: usize>(src: &PruvArr<i64>) -> (i64, [i64; PRUV__K])" in twins
        for name in (b"not_last", b"bytes_out", b"scalar", b"array_len"):
            assert b"fn " + name + b"<" not in twins, name
        for name in (b"copy_first", b"text_len", b"array_len"):
            assert inline_attr(plain[0], name) == inline_attr(twins, name), name
        if flags == ("--trust-callers",):
            assert b"pruv: requires violated" not in twins
            continue
        (work / "main.rs").write_text(HOST, encoding="utf-8")
        binary = work / "host"
        # The default build uses rustc's default edition (as README's direct compile does).
        edition = () if not flags else ("--edition", "2021")
        result = run([rustc, *edition, "-C", "opt-level=3", str(work / "main.rs"), "-o", str(binary)])
        assert result.returncode == 0, result.stderr
        result = run([str(binary)])
        assert result.returncode == 0, result.stderr

    # Even an include! at the crate root cannot let safe host code build a PruvArr without the length check.
    (work / "main.rs").write_text(
        'include!("gen.rs");\nfn main() { let _ = &PruvArr::new(&[1]).0; }\n', encoding="utf-8")
    result = run([rustc, "--edition", "2021", str(work / "main.rs"), "-o", str(work / "host")])
    assert result.returncode != 0 and "E0616" in result.stderr, result.stderr

    # The val module's glob import brings the checked entries into scope privately.
    for call, diagnostic in (
        ("pair::<{ (1usize << 40) + 1 }>(1)", "by-value result longer than 2^40"),
        ("pair::<{ (1usize << 40) + 1 }>(1)", "E0080"),
        ("not_last::<1>(1)", "E0603"),
        ("bytes_out::<1>()", "E0603"),
    ):
        (work / "main.rs").write_text(
            f'include!("gen.rs");\nfn main() {{ let _ = pruv_proven::val::{call}; }}\n', encoding="utf-8")
        result = run([rustc, "--edition", "2021", str(work / "main.rs"), "-o", str(work / "host")])
        assert result.returncode != 0, call
        assert diagnostic in result.stderr, result.stderr
        if call.startswith("pair") and diagnostic != "E0080":
            print(next(line for line in result.stderr.splitlines() if diagnostic in line))

    # TIDY 4.13: no host style can name an unchecked body; the checked entries and twins still work.
    rejects = shadows = 0
    for flags in ((), (flag,)):
        for driver in drivers:
            compile_source(driver, source, work / "gen.rs", flags)
            for host, codes in BODY_REJECTS:
                if rejects == 0:
                    print(host_rejects(rustc, work, host, codes))
                else:
                    host_rejects(rustc, work, host, codes)
                rejects += 1
            for host in MACRO_HOSTS:
                (work / "main.rs").write_text(host, encoding="utf-8")
                result = run([rustc, "--edition", "2021", str(work / "main.rs"), "-o", str(work / "host")])
                assert result.returncode == 0, result.stderr
                result = run([str(work / "host")])
                assert result.returncode == 0, host + result.stderr
                shadows += 1
    (work / "main.rs").write_text(PATH_HOST, encoding="utf-8")
    result = run([rustc, "--edition", "2021", "-C", "opt-level=3", str(work / "main.rs"), "-o", str(work / "host")])
    assert result.returncode == 0, result.stderr
    result = run([str(work / "host")])
    assert result.returncode == 0, result.stderr

    manifest = work / "pruv.json"
    for mode in ("crate", "unsafe"):
        manifest.write_text(json.dumps({"name": "app", "version": "1.0.0", "root": source.name,
                                        "raw_entries": mode}), encoding="utf-8")
        flags = ("--manifest", str(manifest))
        plain = [compile_source(driver, source, work / "plain.rs", flags) for driver in drivers]
        proven = [compile_source(driver, source, work / "gen.rs", (*flags, flag)) for driver in drivers]
        assert plain[0] == plain[1] and proven[0] == proven[1], mode
        plain, proven = [unwrap(code) for code in plain], [unwrap(code) for code in proven]
        assert proven[0].startswith(plain[0]), mode
        if mode == "crate":
            assert proven[0] == plain[0], mode
        else:
            twins = proven[0][len(plain[0]):]
            assert b"pub mod pruv_raw {" in plain[0] and b"pub mod pruv_proven {" in twins, mode
            assert b"pub(crate) fn copy_first(" in twins, mode
            assert b"pub(crate) fn pair<const PRUV__K: usize>" in twins, mode

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

    # R-7-3: proven twins retain each source directive.
    attrs = work / "attrs.pruv"
    attrs.write_text(
        "#[cold] fn f(a: [i64]) -> i64 { return 0; }\n"
        "#[inline] fn g(a: [i64]) -> i64 { return 1; }\n"
        "#[inline(always)] fn h(a: [i64]) -> i64 { return 2; }\n", encoding="utf-8")
    proven = [unwrap(compile_source(driver, attrs, work / "attrs.rs", (flag,))) for driver in drivers]
    assert proven[0] == proven[1], "attribute proven output differs"
    twins = proven[0].split(b"pub mod pruv_proven {", 1)[1]
    for name, directive in (("f", "#[cold]\n    #[inline(never)]"), ("g", "#[inline]"), ("h", "#[inline(always)]")):
        assert f"    {directive}\n    pub fn {name}(a: &PruvArr<i64>) -> i64 {{".encode() in twins, twins
    (work / "main.rs").write_text(
        'include!("attrs.rs");\nfn main() {\n'
        '    assert_eq!(pruv_proven::f(PruvArr::new(&[1])), 0);\n'
        '    assert_eq!(pruv_proven::g(PruvArr::new(&[1])), 1);\n'
        '    assert_eq!(pruv_proven::h(PruvArr::new(&[1])), 2);\n'
        '}\n', encoding="utf-8")
    result = run([rustc, "--edition", "2021", str(work / "main.rs"), "-o", str(work / "host")])
    assert result.returncode == 0, result.stderr
    result = run([str(work / "host")])
    assert result.returncode == 0, result.stderr
    print("P18b/P18c: parity in 3 compiler modes and 2 manifest modes, 3 host runs, privacy, %d body rejects, %d macro shadows and diagnostics pass" % (rejects, shadows))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("work")
    parser.add_argument("--rs")
    parser.add_argument("--py")
    args = parser.parse_args()
    main(args.work, args.rs, args.py)
