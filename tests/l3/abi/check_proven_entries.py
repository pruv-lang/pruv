"""Check P18b/P18c output and the checked-length and by-value Rust host APIs of --proven-entries.

usage: check_proven_entries.py WORK [--rs PATH] [--py PATH] [--vs-py] [--golden PATH] [--update-golden] [--prune-golden]
  (no option)  the Rust compiler (pruvc-rs/target/release/pruvc); every generated Rust text is checked against
               tests/golden/l3_abi_proven_entries.txt
  --rs PATH    the Rust driver binary
  --py PATH    alone (no --rs, no --vs-py): the Python compiler is the main compiler, no golden check (old run_all call
               form `--py PATH`); with --vs-py: the pruvc.py used as the reference
  --vs-py      opt-in: also compile with the frozen Python compiler and require byte-identical output. Never used in CI.
               With --update-golden it writes the golden from the Rust output and prints py/rs disagreements.

PF: the Python compiler (pruvc/) is frozen since bundle PF; by default this script neither runs it, imports it nor
imports z3. The old rs == py byte equality lives behind --vs-py; the default check is the golden file."""
import argparse
import re
import json
from pathlib import Path
import shutil
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
import compiler_driver as CD
import golden


class Ctx:
    """The drivers of this run (drivers[0] is the main one, drivers[1] the py reference with --vs-py) and the golden."""

    def __init__(self, drivers, g, update):
        self.drivers, self.g, self.update = drivers, g, update


def settle(ctx, key, outs, msg, summary):
    """outs: one output per driver. With two drivers they must be identical (under --update-golden a disagreement is
    printed instead); the main driver's output goes to the golden. Returns the main driver's output."""
    if len(outs) > 1 and outs[0] != outs[1]:
        if ctx.update:
            print("PY/RS DISAGREE %s: %s" % (key, msg))
        else:
            raise AssertionError(msg)
    if ctx.g is not None:
        ctx.g.check(key, outs[0], summary)
    return outs[0]


def flag_tag(flags):
    return " ".join(flags) or "default"


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


# P18d: an internal function whose last parameter is `inout o: [i64]` and whose uses of `o` have a fixed prefix K gets a
# by-value twin `NAME__pruv_fp` (`[i64; K]` in and out, a scalar `i64` when K = 1). `g` and `f` are the positive chain (K = 1); the other internal
# functions must NOT get one: variable index, `inout` passed at a non-last position to a callee with a variable index,
# `len(o)` in arithmetic, recursion (it has a requires, which the plan skips as well; the cycle rule itself is a Rust
# unit test in fixed_prefix.rs), and a needed K of 17.
FP_SOURCE = """\
fn g(x: i64, inout o: [i64]) -> i64 {
  if len(o) < 1 { return 3; }
  o[0] = x;
  return 0;
}

fn f(x: i64, inout o: [i64]) -> i64 {
  let r: i64 = g(x, inout o);
  return r;
}

pub fn run(x: i64, inout out: [i64]) -> i64 {
  let r: i64 = f(x, inout out);
  return r;
}

fn rv(i: i64, inout o: [i64]) -> i64 {
  if i < 0 || i >= len(o) { return 3; }
  o[i] = 1;
  return 0;
}

pub fn run_rv(i: i64, inout out: [i64]) -> i64 {
  let r: i64 = rv(i, inout out);
  return r;
}

fn wr(inout a: [i64], i: i64, v: i64) -> i64 {
  if i < 0 || i >= len(a) { return 3; }
  a[i] = v;
  return 0;
}

fn viaw(v: i64, inout o: [i64]) -> i64 {
  let r: i64 = wr(inout o, 0, v);
  return r;
}

pub fn run_viaw(v: i64, inout out: [i64]) -> i64 {
  let r: i64 = viaw(v, inout out);
  return r;
}

fn la(inout o: [i64]) -> i64 {
  if len(o) - 1 < 0 { return 3; }
  o[0] = 1;
  return 0;
}

pub fn run_la(inout out: [i64]) -> i64 {
  let r: i64 = la(inout out);
  return r;
}

fn rec(n: i64, inout o: [i64]) -> i64
  requires n >= 0
  decreases n
{
  if len(o) < 1 { return 3; }
  o[0] = n;
  if n == 0 { return 0; }
  let r: i64 = rec(n - 1, inout o);
  return r;
}

pub fn run_rec(n: i64, inout out: [i64]) -> i64 {
  if n < 0 { return 2; }
  let r: i64 = rec(n, inout out);
  return r;
}

fn big(inout o: [i64]) -> i64 {
  if len(o) < 17 { return 3; }
  o[16] = 1;
  return 0;
}

pub fn run_big(inout out: [i64]) -> i64 {
  let r: i64 = big(inout out);
  return r;
}
"""

# Appended to FP_SOURCE (must-reject): print can panic after a write, so a function that reaches print, directly (pw)
# or through a callee that does not take `o` (pc), gets no twin; so does one that reaches a G6 catalog operation, whose
# boundary helper panics on a failed check or an exhausted check budget (pk directly, pkc through ap); g and f keep theirs.
FP_PRINT = """
fn say(x: i64) -> i64 ! io {
  print(x);
  return x;
}

fn pw(x: i64, inout o: [i64]) -> i64 ! io {
  if len(o) < 1 { return 3; }
  o[0] = x;
  print(x);
  return 0;
}

fn pc(x: i64, inout o: [i64]) -> i64 ! io {
  if len(o) < 1 { return 3; }
  o[0] = x;
  let r: i64 = say(x);
  return 0;
}

pub fn run_p(x: i64, inout out: [i64]) -> i64 ! io {
  let a: i64 = pw(x, inout out);
  if a != 0 { return a; }
  let b: i64 = pc(x, inout out);
  return b;
}
fn ap(t: str) -> i64 ! alloc
{
  let s: String = string_new();
  let r: String = string_push_str(move s, t);
  return len(r);
}

fn pk(t: str, inout o: [i64]) -> i64 ! alloc
{
  if len(o) < 1 { return 3; }
  o[0] = 7;
  let s: String = string_new();
  let r: String = string_push_str(move s, t);
  return 0;
}

fn pkc(t: str, inout o: [i64]) -> i64 ! alloc
{
  if len(o) < 1 { return 3; }
  o[0] = 7;
  let n: i64 = ap(t);
  return 0;
}

pub fn run_k(t: str, inout out: [i64]) -> i64 ! alloc
{
  let a: i64 = pk(t, inout out);
  if a != 0 { return a; }
  let b: i64 = pkc(t, inout out);
  return b;
}
"""

# Buffers of length 0, 1, 4, 17 and 20 are pre-filled with 9; only the one expected word may change.
FP_HOST = """\
include!("fp_gen.rs");

fn probe(name: &str, f: &dyn Fn(&mut [i64]) -> i64, min: usize, idx: usize, val: i64, ok: i64, bad: i64) {
    for len in [0usize, 1, 4, 17, 20] {
        let mut buf = [9i64; 20];
        let got = f(&mut buf[..len]);
        let mut want = [9i64; 20];
        if len >= min { want[idx] = val; }
        assert_eq!(got, if len >= min { ok } else { bad }, "{name} len {len}");
        assert_eq!(buf, want, "{name} len {len}");
    }
}

fn main() {
    probe("run", &|o| run(5, o), 1, 0, 5, 0, 3);
    probe("run_rv", &|o| run_rv(0, o), 1, 0, 1, 0, 3);
    probe("run_viaw", &|o| run_viaw(7, o), 1, 0, 7, 0, 3);
    probe("run_la", &|o| run_la(o), 1, 0, 1, 0, 3);
    probe("run_rec", &|o| run_rec(3, o), 1, 0, 0, 0, 3);
    probe("run_big", &|o| run_big(o), 17, 16, 1, 0, 3);
    probe("proven run", &|o| pruv_proven::run(5, PruvArr::new_mut(o)), 1, 0, 5, 0, 3);
    probe("proven run_rv", &|o| pruv_proven::run_rv(0, PruvArr::new_mut(o)), 1, 0, 1, 0, 3);
    probe("proven run_viaw", &|o| pruv_proven::run_viaw(7, PruvArr::new_mut(o)), 1, 0, 7, 0, 3);
    probe("proven run_la", &|o| pruv_proven::run_la(PruvArr::new_mut(o)), 1, 0, 1, 0, 3);
    probe("proven run_rec", &|o| pruv_proven::run_rec(3, PruvArr::new_mut(o)), 1, 0, 0, 0, 3);
    probe("proven run_big", &|o| pruv_proven::run_big(PruvArr::new_mut(o)), 17, 16, 1, 0, 3);
    assert_eq!(pruv_proven::val::run::<0>(5), (3, [0i64; 0]));
    assert_eq!(pruv_proven::val::run::<1>(5), (0, [5]));
    assert_eq!(pruv_proven::val::run::<4>(5), (0, [5, 0, 0, 0]));
    assert_eq!(pruv_proven::val::run_rv::<2>(1), (0, [0, 1]));
    assert_eq!(pruv_proven::val::run_viaw::<1>(7), (0, [7]));
    assert_eq!(pruv_proven::val::run_la::<1>(), (0, [1]));
    assert_eq!(pruv_proven::val::run_rec::<1>(3), (0, [0]));
    assert_eq!(pruv_proven::val::run_big::<16>(), (3, [0i64; 16]));
}
"""


def check_fixed_prefix(work, ctx, rustc):
    """P18d: the by-value twin exists exactly for the proved functions, the text matches the golden (and the Python
    compiler with --vs-py), and the optimized host probes (normal entry, proven twin, val::<K>) see the same results and
    untouched words."""
    drivers = ctx.drivers
    source = work / "fp.pruv"
    source.write_text(FP_SOURCE, encoding="utf-8")
    flag = "--proven-entries"
    for flags in ((), ("--trust-callers",), ("--no-verify",)):
        plain = settle(ctx, "proven_entries|fp %s|rust-plain" % flag_tag(flags),
                       [compile_source(driver, source, work / "fp_plain.rs", flags) for driver in drivers],
                       "fixed-prefix plain output differs", "rust-plain fp")
        proven = settle(ctx, "proven_entries|fp %s|rust-proven" % flag_tag(flags),
                        [compile_source(driver, source, work / "fp_gen.rs", (*flags, flag)) for driver in drivers],
                        "fixed-prefix proven output differs", "rust-proven fp")
        assert b"__pruv_fp" not in plain, "twin without --proven-entries"
        code = unwrap(proven)
        if flags == ("--no-verify",):
            # must-reject: an unverified build may panic inside a body (overflow, index), and the adapter would drop
            # the words written before the panic; the plan needs a verified, unwind-free program
            assert b"__pruv_fp" not in code, "twin in a --no-verify build"
        else:
            check_fp_twins(code)
            # must-reject: print and G6 catalog operations can panic after a write, so a function that reaches one gets
            # no twin (checked in every --foreign-checks mode: requires, all, off)
            printing = work / "fp_print.pruv"
            printing.write_text(FP_SOURCE + FP_PRINT, encoding="utf-8")
            for mode in ("requires", "all", "off"):
                fc = ("--foreign-checks", mode)
                outs = [compile_source(driver, printing, work / "fp_print.rs", (*flags, flag, *fc)) for driver in drivers]
                out = settle(ctx, "proven_entries|fp-print %s %s|rust-proven" % (flag_tag(flags), " ".join(fc)), outs,
                             "fixed-prefix print output differs", "rust-proven fp-print %s" % mode)
                code = unwrap(out)
                check_fp_twins(code)
                for name in (b"say", b"pw", b"pc", b"run_p", b"ap", b"pk", b"pkc", b"run_k"):
                    assert re.search(rb"\nfn \w*" + name + rb"__pruv_fpb?\(", code) is None, (mode, name)
        if flags == ("--trust-callers",):
            continue
        (work / "main.rs").write_text(FP_HOST, encoding="utf-8")
        edition = () if not flags else ("--edition", "2021")
        result = run([rustc, *edition, "-C", "opt-level=3", str(work / "main.rs"), "-o", str(work / "host")])
        assert result.returncode == 0, result.stderr
        result = run([str(work / "host")])
        assert result.returncode == 0, result.stderr


def check_fp_twins(code):
    """twins exactly for g and f"""
    # private functions carry the module prefix (e.g. pruv_m_..._g); take it from g's twin
    found = re.search(rb"\nfn (\w*?)g__pruv_fp\(", code)
    assert found, "no twin for g"
    pre = found.group(1)
    for name in (b"g", b"f"):
        assert b"\nfn " + pre + name + b"__pruv_fp(" in code, name
        assert b"\nfn " + pre + name + b"__pruv_fpb(" in code, name
    # K = 1: the word travels as a scalar (`(i64, i64)` is returned in registers, `(i64, [i64; 1])` through memory)
    assert b"\nfn " + pre + b"g__pruv_fp(x: i64, pruv_f0: i64, pruv_fn: usize) -> (i64, i64) {" in code
    assert b"\nfn " + pre + b"f__pruv_fp(x: i64, pruv_f0: i64, pruv_fn: usize) -> (i64, i64) {" in code
    assert b"let (pruv_fr, pruv_f0) = " + pre + b"g__pruv_fp(x, pruv_fa[0], pruv_fn);" in code
    assert code.count(b"__pruv_fp(") == 4, "twins beyond g and f"
    for name in (b"run", b"rv", b"run_rv", b"wr", b"viaw", b"run_viaw", b"la", b"run_la", b"rec", b"run_rec",
                 b"big", b"run_big"):
        for p in (b"", pre):
            assert b"\nfn " + p + name + b"__pruv_fp(" not in code, name
            assert b"\nfn " + p + name + b"__pruv_fpb(" not in code, name


def inline_attr(code, name):
    lines = code.splitlines()
    index = next(i for i, line in enumerate(lines) if line.lstrip().startswith(b"pub fn " + name + b"("))
    return lines[index - 1].lstrip()


def main(work, rs, py, vs_py=False, gargs=None):
    work = Path(work).resolve() / "l3_abi_proven"
    work.mkdir(parents=True, exist_ok=True)
    # --py alone keeps the old run_all call form: the Python compiler is then the main one (no golden). Otherwise the
    # Rust compiler is main and the Python compiler is only created for --vs-py.
    py_main = bool(py) and not rs and not vs_py
    if py_main:
        drivers = (CD.make_driver("py", py),)
    else:
        drivers = (CD.make_driver("rs", rs),) + ((CD.make_driver("py", py),) if vs_py else ())
    g = None
    if not py_main and gargs is not None:
        g = golden.Golden.from_args(gargs, "l3_abi_proven_entries.txt",
                                    "check_proven_entries: generated Rust text per case", tmp_dirs=[str(work)])
    ctx = Ctx(drivers, g, bool(gargs is not None and gargs.update_golden))
    source = work / "app.pruv"
    source.write_text(SOURCE, encoding="utf-8")
    flag = "--proven-entries"
    rustc = shutil.which("rustc") or str(Path.home() / ".cargo/bin/rustc")
    for flags in ((), ("--trust-callers",), ("--no-verify",)):
        plain, proven = [], []
        for driver in drivers:
            plain.append(compile_source(driver, source, work / "plain.rs", flags))
            proven.append(compile_source(driver, source, work / "gen.rs", (*flags, flag)))
        plain = unwrap(settle(ctx, "proven_entries|app %s|rust-plain" % flag_tag(flags), plain,
                              "plain output differs", "rust-plain app"))
        proven = unwrap(settle(ctx, "proven_entries|app %s|rust-proven" % flag_tag(flags), proven,
                               "proven output differs", "rust-proven app"))
        plain, proven = [plain], [proven]
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
        plain = unwrap(settle(ctx, "proven_entries|manifest %s|rust-plain" % mode, plain, "plain output differs: " + mode,
                              "rust-plain manifest %s" % mode))
        proven = unwrap(settle(ctx, "proven_entries|manifest %s|rust-proven" % mode, proven,
                               "proven output differs: " + mode, "rust-proven manifest %s" % mode))
        plain, proven = [plain], [proven]
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
    clash_plain = []
    for driver in drivers:
        clash_plain.append(compile_source(driver, clash, work / "plain.rs"))
        result = run(driver.argv(str(clash), str(work / "clash.rs"), [flag], "rust"))
        assert result.returncode == 1 and "conflicts with Rust name PruvArr" in result.stdout, result.stdout + result.stderr
        diagnostics.append(result.stdout)
        result = run(driver.argv(str(source), str(work / "app.ll"), [flag], "llvm"))
        assert result.returncode == 2 and result.stderr.strip().endswith(
            "--proven-entries requires --backend rust"), result.stdout + result.stderr
    settle(ctx, "proven_entries|clash|rust-plain", clash_plain, "clash plain output differs", "rust-plain clash")
    settle(ctx, "proven_entries|clash|diagnostic", diagnostics, "clash diagnostics differ", "diagnostic clash")

    # R-7-3: proven twins retain each source directive.
    attrs = work / "attrs.pruv"
    attrs.write_text(
        "#[cold] fn f(a: [i64]) -> i64 { return 0; }\n"
        "#[inline] fn g(a: [i64]) -> i64 { return 1; }\n"
        "#[inline(always)] fn h(a: [i64]) -> i64 { return 2; }\n", encoding="utf-8")
    proven = unwrap(settle(ctx, "proven_entries|attrs|rust-proven",
                           [compile_source(driver, attrs, work / "attrs.rs", (flag,)) for driver in drivers],
                           "attribute proven output differs", "rust-proven attrs"))
    twins = proven.split(b"pub mod pruv_proven {", 1)[1]
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
    check_fixed_prefix(work, ctx, rustc)
    if g is not None and not g.finish():
        return 1
    print("P18d: by-value fixed-prefix twins in 3 compiler modes, 5 must-reject shapes, no twin under --no-verify or reaching print or a G6 catalog operation, 2 optimized host runs")
    print("P18b/P18c: %s in 3 compiler modes and 2 manifest modes, 3 host runs, privacy, %d body rejects, %d macro shadows and diagnostics pass"
          % ("golden" + (" + rs == py" if vs_py else "") if g is not None else "single compiler", rejects, shadows))
    return 0


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("work")
    parser.add_argument("--rs")
    parser.add_argument("--py")
    parser.add_argument("--vs-py", action="store_true", dest="vs_py")
    golden.add_args(parser)
    args = parser.parse_args()
    sys.exit(main(args.work, args.rs, args.py, args.vs_py, args))
