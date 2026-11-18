"""pruvc: Pruv compiler CLI.

  python pruvc/pruvc.py in.pruv -o OUT [--no-verify] [--trust-callers]
                      [--backend rust|llvm] [--rust-bindings PATH] [--infer [--show-inferred]]

--no-verify      skip the proof; emit safe code (checked indexing/arithmetic)
--infer          STAGE1 B2: infer loop invariants from fixed templates (Houdini) and use the surviving
                 ones as extra invariants in the proof (docs/STAGE1_B_INFER.md). Off by default; without it
                 the proof and its solver queries are unchanged. Generated code is unchanged.
--show-inferred  with --infer only (otherwise a usage error): one JSON line per loop on stderr (fn, loop,
                 line, col, status, invariants, generated, capped, kept, dropped, rounds, queries).
                 With --no-verify the single line {"status": "skipped_no_verify"}.
--trust-callers  public entry points skip the run-time `requires` check (benchmarking)
--backend        rust (default): OUT is Rust source.
                 llvm: OUT is textual LLVM IR (.ll); Rust bindings are written to
                 --rust-bindings PATH (default: OUT with the extension changed to .rs;
                 it is a usage error if that equals OUT).
--rust-bindings  llvm backend only; giving it with the rust backend is a usage error.
--rust-extern NAME=PATH  repeatable (G6): a crate a `trusted rust fn` path may start with (NAME: a Rust
                 identifier, not std, no duplicates). A0: only NAME is used; the file is not opened.
--rust-search PATH  repeatable (G6): rustc `-L dependency=PATH` directory (recorded only in A0).
--lib DIR        repeatable (G4b-0): library directory searched for `mod NAME;` declared in the root
                 file (docs/G4B_LIB_SPEC.md); DIR must be an existing directory.
--foreign-checks requires|all|off  G6 boundary contract checks (default requires); `off` needs verification,
                 so `--no-verify --foreign-checks off` is a usage error.
--contract-budget N  internal/test-only (overrides the per-entry requires iteration
                 budget, default 2^30; not part of the language spec).
--also-llvm LL RS  internal/test-only (used by bench/fuzz/run.py; not part of the language
                 spec). Rust backend only: in the same single parse+check+verify run, also
                 write the LLVM IR to LL and its Rust bindings to RS. OUT, LL, RS must be
                 pairwise distinct. Verification failure -> no file is written.
                 All texts are generated before any file is written, so a generator error
                 (exit 3) leaves no output from this call.

Language errors: JSON lines on stdout, exit 1.  Success: {"ok": true}, exit 0.
Usage errors: usage text, exit 2.
Internal compiler errors (any non-PruvError exception): one JSON line with
stage "internal", code "internal_error" on stdout, exit 3.
"""
import argparse
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from vast import PruvError  # noqa: E402
from vparser import parse, RUST_KEYWORDS  # noqa: E402
from modules import load_program  # noqa: E402
from check import check, opaque_inline  # noqa: E402
from codegen import gen_rust, gen_rust_ex  # noqa: E402
from typeinfo import g2_unimplemented_error, first_g3_use  # noqa: E402
from mono import specialize  # noqa: E402
from foreign import g6_unimplemented_error, g7_target_error, first_async_use  # noqa: E402

USAGE = ("usage: pruvc.py in.pruv -o OUT [--no-verify] [--trust-callers] "
         "[--backend rust|llvm] [--rust-bindings PATH] [--infer [--show-inferred]] "
         "[--rust-extern NAME=PATH] [--rust-search PATH] [--lib DIR] [--proof-cache DIR] [--foreign-checks requires|all|off]")


def default_bindings_path(out):
    return os.path.splitext(out)[0] + ".rs"


def _write_trust_report(prog, path, report_path, verify, trust_callers, budget, infer, foreign_checks,
                        rust_extern_raw, rust_search_raw, graph, cache=None, cache_flag=False):
    """G4b-2 (docs/G4B_SPEC.md 6, rule 8): run mono and the Rust-source generator in memory, keep only the trust data,
    write the report.  Nothing is written when a language error is raised."""
    from trust_report import build_report, write_report
    plan = None
    gprog = prog
    if first_g3_use(prog) is not None or prog.explicit:
        plan = specialize(prog)          # PruvError specialization_limit propagates
        gprog = plan.program
    g2err = g2_unimplemented_error(gprog, "codegen", skip=("array_u8", "array_bool", "str"))
    if g2err is not None:
        raise g2err
    specs = []
    for spec in rust_extern_raw:
        n, _, p = spec.partition("=")
        specs.append((n, p))
    trust = {}
    gen_rust_ex(prog, verified=verify, trust_callers=trust_callers, budget=budget, plan=plan,
                foreign_checks=foreign_checks, externs=specs, trust_out=trust)
    text = build_report(prog, trust, graph, path, verify, trust_callers, foreign_checks, infer,
                        rust_extern_raw, rust_search_raw, cache=cache, cache_flag=cache_flag)
    write_report(report_path, text)


def compile_file(path, out, verify=True, trust_callers=False, backend="rust", rust_bindings=None,
                 budget=None, also_llvm=None, infer=False, show_inferred=False, externs=(),
                 foreign_checks="requires", extern_specs=(), emit=None, libs=(), manifest=None,
                 trust_report=None, rust_extern_raw=(), rust_search_raw=(), proof_cache=None):
    """extern_specs: [(name, path)] of --rust-extern (recorded in the manifest); externs: the names only.
    trust_report: PATH of --trust-report (G4b-2); rust_extern_raw / rust_search_raw: the raw flag values for it."""
    if budget is None:
        budget = 1 << 30
    assert 1 <= budget <= 2 ** 62
    if also_llvm and backend != "rust":
        raise ValueError("also_llvm requires backend='rust'")
    prog = load_program(path, libs=libs, manifest=manifest)   # G4: the root file plus its declared module files; a legacy root is plain parse(src)
    check(prog, externs=externs)
    # G9 A3: both back ends lower G9 programs; the G9 staging guard is gone from the driver.
    # G6: the Rust source backend lowers foreign calls and collections (A2); the LLVM backend (and --also-llvm)
    # lowers the G6_SPEC 13.1 subset (A3) and rejects everything else with codegen/unsupported_type: that decision
    # is made inside llvmgen.gen_llvm (after verification, so verification errors win).
    cache = None
    if verify and proof_cache is not None:
        from proof_cache import ProofCache
        cache = ProofCache(proof_cache)     # G4b-3: only built when verification runs
    if verify:
        from verify import verify_program
        if infer:
            report = [] if show_inferred else None
            errs = verify_program(prog, infer=True, report=report, cache=cache)
            for rec in report or []:
                print(json.dumps(rec, sort_keys=True, ensure_ascii=False), file=sys.stderr)
        else:
            errs = verify_program(prog, cache=cache)
    elif infer and show_inferred:
        print(json.dumps({"status": "skipped_no_verify"}, sort_keys=True, ensure_ascii=False), file=sys.stderr)
        errs = []
    else:
        errs = []
    opaque_inline(prog)     # P13o: after verification, contract applications of opaque functions become their bodies
    if trust_report is not None:
        # G4b-2: after verification (proved or not); a language error raised here is the compile's result
        _write_trust_report(prog, path, trust_report, verify, trust_callers, budget, infer, foreign_checks,
                            rust_extern_raw, rust_search_raw, manifest, cache=cache,
                            cache_flag=proof_cache is not None)
    if errs:
        return errs
    g7err = g7_target_error(prog) if emit in ("exe", "obj") else None
    if g7err is not None:
        return [g7err]
    if emit in ("exe", "obj") and first_async_use(prog) is not None:
        return [PruvError("codegen", 0, 0, "foreign_target",
                          "a G6 program on the LLVM backend needs the Rust host build (generated bindings compiled by rustc)",
                          "emit LLVM IR and Rust bindings (--backend llvm --rust-bindings PATH) and compile the bindings with rustc")]
    if emit is not None:
        raise NativeEmitUnavailable()
    # G3 A3/A4: both back ends (the Rust source backend, the LLVM backend, also --also-llvm) consume the same
    # shared specialization plan (pruvc/mono.py), also with --no-verify. G4 A2: explicit mode always goes through the
    # plan (it names every function and type by its Rust stem). G4 A3: the LLVM backend generates explicit programs
    # too (no staging guard); the executable shim of the Rust-implemented `--emit exe` targets only the root `main`.
    plan = None
    gprog = prog
    if first_g3_use(prog) is not None or prog.explicit:
        try:
            plan = specialize(prog)   # does not mutate prog; PruvError specialization_limit -> no output at all
        except PruvError as e:
            return [e]
        gprog = plan.program
    # G2: runs after verify too; array types that have no code-generation layer yet are rejected.
    # [u8] (A3·A4), [bool] (A5) and str (B2) are implemented by every back end.
    g2err = g2_unimplemented_error(gprog, "codegen", skip=("array_u8", "array_bool", "str"))  # G2 B2
    if g2err is not None:
        return [g2err]
    if backend == "llvm":
        from llvmgen import gen_llvm  # lazy: only the llvm backend needs it
        try:
            ir, bindings = gen_llvm(prog, verified=verify, trust_callers=trust_callers, budget=budget, plan=plan,
                                    foreign_checks=foreign_checks)
        except PruvError as e:   # G6 A3: codegen/unsupported_type outside G6_SPEC 13.1
            return [e]
        with open(out, "w") as fh:
            fh.write(ir)
        with open(rust_bindings or default_bindings_path(out), "w") as fh:
            fh.write(bindings)
    else:
        # generate every text first; write files only after all generators succeeded
        # (gen_rust / gen_llvm do not mutate prog, so they share it)
        specs = list(extern_specs) if extern_specs else [(n, "") for n in externs]
        rs_text, manifest = gen_rust_ex(prog, verified=verify, trust_callers=trust_callers, budget=budget,
                                        plan=plan, foreign_checks=foreign_checks, externs=specs)
        llvm_texts = None
        if also_llvm:
            from llvmgen import gen_llvm
            try:
                llvm_texts = gen_llvm(prog, verified=verify, trust_callers=trust_callers, budget=budget, plan=plan,
                                      foreign_checks=foreign_checks)
            except PruvError as e:   # G6 A3: nothing is written when the LLVM side rejects
                return [e]
        with open(out, "w") as fh:
            fh.write(rs_text)
        if manifest is not None:   # G6_SPEC 12.4
            with open(out + ".foreign.json", "w", encoding="utf-8") as fh:
                fh.write(manifest)
        if llvm_texts is not None:
            with open(also_llvm[0], "w") as fh:
                fh.write(llvm_texts[0])
            with open(also_llvm[1], "w") as fh:
                fh.write(llvm_texts[1])
    return []


class NativeEmitUnavailable(Exception):
    """The Python driver only diagnoses G7 native targets; native builds use the Rust driver."""


class _Parser(argparse.ArgumentParser):
    def error(self, message):
        print(USAGE)
        print("pruvc.py: error: " + message, file=sys.stderr)
        sys.exit(2)


def main(argv):
    ap = _Parser(prog="pruvc.py", usage=USAGE, add_help=False)
    ap.add_argument("input")
    ap.add_argument("-o", dest="out", required=True)
    ap.add_argument("--no-verify", action="store_true")
    ap.add_argument("--trust-callers", action="store_true")
    ap.add_argument("--backend", choices=["rust", "llvm"], default="rust")
    ap.add_argument("--rust-bindings", default=None)
    ap.add_argument("--emit", choices=["exe", "obj"], default=None)
    ap.add_argument("--infer", action="store_true")
    ap.add_argument("--show-inferred", dest="show_inferred", action="store_true")
    ap.add_argument("--rust-extern", dest="rust_extern", action="append", default=[], metavar="NAME=PATH")
    ap.add_argument("--rust-search", dest="rust_search", action="append", default=[], metavar="PATH")
    ap.add_argument("--lib", dest="libs", action="append", default=[], metavar="DIR")
    ap.add_argument("--manifest", dest="manifest", default=None, metavar="PATH")
    ap.add_argument("--trust-report", dest="trust_report", default=None, metavar="PATH")
    ap.add_argument("--proof-cache", dest="proof_cache", default=None, metavar="DIR")
    ap.add_argument("--foreign-checks", dest="foreign_checks", choices=["requires", "all", "off"],
                    default="requires")
    ap.add_argument("--contract-budget", dest="budget", type=int, default=None,
                    help=argparse.SUPPRESS)
    ap.add_argument("--also-llvm", dest="also_llvm", nargs=2, metavar=("LL", "RS"), default=None,
                    help=argparse.SUPPRESS)
    a = ap.parse_args(argv[1:])
    if a.show_inferred and not a.infer:
        ap.error("--show-inferred requires --infer")
    if a.also_llvm is not None:
        if a.backend != "rust":
            ap.error("--also-llvm requires --backend rust")
        paths = [os.path.abspath(p) for p in [a.out] + a.also_llvm]
        if len(set(paths)) != 3:
            ap.error("-o, --also-llvm LL and RS paths must be pairwise distinct")
    if a.budget is not None and not 1 <= a.budget <= 2 ** 62:
        ap.error("--contract-budget must be in 1..2**62")
    if a.rust_bindings is not None and a.backend != "llvm":
        ap.error("--rust-bindings requires --backend llvm")
    if a.foreign_checks == "off" and a.no_verify:
        ap.error("--foreign-checks off requires verification (not allowed with --no-verify)")
    externs = []
    extern_specs = []
    for spec in a.rust_extern:
        name, sep, xpath = spec.partition("=")
        if not sep or not xpath or not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", name) \
                or name == "std" or name in RUST_KEYWORDS:
            ap.error("invalid --rust-extern %r: expected NAME=PATH with a Rust identifier NAME other than std" % spec)
        if name in externs:
            ap.error("duplicate --rust-extern name %s" % name)
        externs.append(name)
        extern_specs.append((name, xpath))
    if any(not p for p in a.rust_search):
        ap.error("--rust-search needs a non-empty PATH")
    if any(not p or not os.path.isdir(p) for p in a.libs):
        ap.error("--lib needs an existing directory")
    graph = None
    if a.manifest is not None:
        if a.libs:
            ap.error("--manifest and --lib cannot be used together")
        if not os.path.isfile(a.manifest):
            ap.error("--manifest needs an existing file")
    bindings = a.rust_bindings
    if a.backend == "llvm":
        if not bindings:  # same default as the write in compile_file (an empty value means the default)
            bindings = default_bindings_path(a.out)
        if os.path.abspath(bindings) == os.path.abspath(a.out):
            ap.error("--rust-bindings path must differ from -o path")
    if a.trust_report is not None:
        # G4b-2: the report must not overwrite the input or be overwritten by an output written after it
        taken = [a.input, a.out, a.out + ".foreign.json"] + ([bindings] if a.backend == "llvm" else []) + \
            (list(a.also_llvm) if a.also_llvm else [])
        if any(os.path.realpath(p) == os.path.realpath(a.trust_report) for p in taken):
            ap.error("--trust-report path must differ from the input and output paths")
    if a.manifest is not None:
        from manifest import load_graph
        try:
            graph = load_graph(a.manifest)
        except PruvError as e:          # manifest diagnostics come before the root match check
            print(e.to_json())
            return 1
        prog_pkg = graph.program
        rootf = os.path.join(prog_pkg.dir, *prog_pkg.root.split("/"))
        if not (os.path.exists(a.input) and os.path.samefile(rootf, a.input)):
            ap.error("--manifest root does not match the input file")
    try:
        errs = compile_file(a.input, a.out, not a.no_verify, a.trust_callers, a.backend, bindings,
                          a.budget, tuple(a.also_llvm) if a.also_llvm else None,
                          infer=a.infer, show_inferred=a.show_inferred, externs=tuple(externs),
                          foreign_checks=a.foreign_checks, extern_specs=tuple(extern_specs), emit=a.emit,
                          libs=tuple(a.libs), manifest=graph, trust_report=a.trust_report,
                          rust_extern_raw=tuple(a.rust_extern), rust_search_raw=tuple(a.rust_search),
                          proof_cache=a.proof_cache)
    except NativeEmitUnavailable:
        ap.error("unrecognized arguments: --emit " + a.emit)
    except PruvError as e:
        errs = [e]
    except Exception as e:  # compiler-internal failure, not a language error
        print(json.dumps({"stage": "internal", "line": 0, "col": 0,
                          "code": "internal_error", "message": str(e), "fix": ""}))
        return 3
    if errs:
        for e in errs:
            print(e.to_json())
        return 1
    print(json.dumps({"ok": True}))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
