#!/usr/bin/env python3
"""Harness for the L3-10 M3/M4 milestones.

  python3 bench/l3/budget/m34_test/run.py m3|m4|m4raw|perf|keys|admit [--pruvc PATH] [--seed N] [--count N]
  python3 bench/l3/budget/m34_test/run.py m4 --only m4llvm|m4llvmraw|m4c|m4cb|m4nv|m4llvmnv   (or the lane name as the mode;
                                                  mode `lanes` runs every lane; see M4_LANES for what each builds)

`--pruvc` may be pruvc/pruvc.py (the Python compiler; run under this interpreter).
`m4raw` (P09) builds root_m4 as a manifest package (work/m34/pkg_m4raw, raw_entries "crate") and drives it through
sol::pruv_raw::PruvStore; `m4` drives the same calls through the local no-poison Plain store. Same driver checks.

`admit` (P11) builds root_admit.pruv / driver_admit.rs: bud_admit::admit on a clean store and on one corruption per
check (header, roots, state, owner, key span/padding, blob span, links, free list, slot word, counts), then the clean store
at larger lengths (visit counts must stay linear: entries == top, free steps == free entries, slots == slot count).

`keys` (P03) builds root_keys.pruv / driver_keys.rs and compares every output line with the gen_keys.py oracle.

Builds root_m3.pruv / root_m4.pruv (this directory) with pruvc into bench/l3/budget/work/m34/, compiles the Rust
drivers against the generated modules, runs them on deterministic generated cases and compares with the Python
reference (gen_m3.py / gen_m4.py). `perf` compares the Pruv M4 ops against the TS-style Map/Set loop (perf_ts.mjs), which does the same work
boundary (op words decoded and validated, result words written); 200000 reps, Pruv/TS runs alternated, median of 5.
Exit code: 0 all pass, 1 mismatches / failed verdict, 2 build or tool problem. Python 3.9, standard library only.
"""
import argparse
import os
import re
import shutil
import statistics
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(HERE))))    # m34_test -> repo root
WORK = os.path.join(ROOT, "bench", "l3", "budget", "work", "m34")
PRUVC_DEFAULT = os.path.join(ROOT, "pruvc-rs", "target", "release", "pruvc.exe" if os.name == "nt" else "pruvc")
LIBS = ["--lib", "bench/l3/budget", "--lib", "bench/l3/lib"]

sys.path.insert(0, HERE)
import gen_m3   # noqa: E402
import gen_m4   # noqa: E402
import gen_keys  # noqa: E402


class ToolError(Exception):
    pass


def tail(c):
    return ((c.stdout or "") + (c.stderr or ""))[-1500:]


def run(cmd, cwd=None, inp=None, timeout=600):
    try:
        return subprocess.run(cmd, cwd=cwd, input=inp, capture_output=True, text=True, timeout=timeout)
    except OSError as e:
        raise ToolError("cannot execute %s: %s" % (cmd[0], e))
    except subprocess.TimeoutExpired:
        raise ToolError("timeout: %s" % " ".join(cmd))


def find_rustc():
    return shutil.which("rustc") or (os.path.expanduser("~/.cargo/bin/rustc")
                                     if os.path.exists(os.path.expanduser("~/.cargo/bin/rustc")) else None)


def pruvc_cmd(pruvc):
    """The compiler command prefix: a .py path (the Python compiler) runs under this interpreter."""
    return [sys.executable, pruvc] if pruvc.endswith(".py") else [pruvc]


# m4raw modules (root_m4.pruv's `mod` lines), copied from the --lib dirs into the manifest package directory.
M4RAW_MODS = [("bench/l3/lib", "ent_u16"), ("bench/l3/budget", "bud_keys"), ("bench/l3/budget", "bud_store"),
              ("bench/l3/budget", "bud_map"), ("bench/l3/budget", "bud_world")]


# m4 lanes (all drive root_m4.pruv with driver_m4.rs; the q-mode check is the same for every lane):
#   backend: "rust" (Rust source) or "llvm" (LLVM IR + Rust bindings, linked through clang + runtime.c);
#   raw:     manifest raw_entries (None = no manifest, "crate" = pruv_raw::PruvStore, "c" = pruv_c extern "C" adapter);
#   nv:      root_m4.pruv with the leading `valid(st)` checks of m4_op / m4_observe removed (temp copy);
#   eq:      the package copies of the modules call a verified element loop (EQ_HELPER) instead of eq_range. The LLVM
#            backend rejects eq_range until P17a lowers it, so every LLVM lane needs this; m4eq is the Rust lane with the
#            same substitution, the like-for-like reference for the LLVM lanes. These are preliminary (P10) lanes: P17c
#            measures the real LLVM path with eq_range.
M4_LANES = {
    "m4":        dict(backend="rust", raw=None,    nv=False, eq=False),
    "m4raw":     dict(backend="rust", raw="crate", nv=False, eq=False),
    "m4nv":      dict(backend="rust", raw=None,    nv=True,  eq=False),
    "m4eq":      dict(backend="rust", raw=None,    nv=False, eq=True),
    "m4llvm":    dict(backend="llvm", raw=None,    nv=False, eq=True),
    "m4llvmraw": dict(backend="llvm", raw="crate", nv=False, eq=True),
    "m4c":       dict(backend="llvm", raw="c",     nv=False, eq=True),
    # m4cb: m4c, but the sessions axis does each rep's get + put as ONE call of the batch entry <tag>__pruv_hb_m4_op
    "m4cb":      dict(backend="llvm", raw="c",     nv=False, eq=True),
    "m4llvmnv":  dict(backend="llvm", raw=None,    nv=True,  eq=True),
}
# World-section lanes: driver_<tag>.rs replaces driver_m4.rs's World section (the symbols are <tag>__pruv_*)
WORLD_LANES = ("m4c", "m4cb")
# m4cb: the sessions-axis timed loop of driver_m4.rs perf (one [get, put] pair per rep) calls World::exec2 once per rep
BATCH_LOOP_ANCHOR = "    for _ in 0..nops {\n        let op = &cycle[j];\n"
BATCH_LOOP_PATCH = """    // m4cb (run.py): sessions axis = [get, put] per rep, one batch call per rep; other axes keep one call per op.
    let batched = axis == "sessions" && cycle.len() == 2;
    if batched {
        for _ in 0..nops / 2 {
            match w.exec2(black_box(&cycle[0]), black_box(&cycle[1])) {
                // Same sink as the single-call loop: out[0] + out[1] of each of the two ops.
                R::Ok(_) => {
                    sink = sink.wrapping_add(black_box(w.out[0] + w.out[1]));
                    sink = sink.wrapping_add(black_box(w.out2[0] + w.out2[1]));
                }
                _ => {
                    eprintln!("perf: batch failed in timed loop");
                    std::process::exit(1);
                }
            }
        }
    }
    for _ in 0..(if batched { 0 } else { nops }) {
        let op = &cycle[j];
"""
EQ_CALL = re.compile(r"\beq_range\(")
EQ_HELPER = """
// P10 bench lane (run.py, eq lanes): eq_range as a verified element loop; same five bounds as eq_range's obligations.
fn eqr_p10(a: [i64], i: i64, b: [i64], j: i64, n: i64) -> bool
  requires 0 <= i && 0 <= j && 0 <= n && i + n <= len(a) && j + n <= len(b)
{
  let mut k: i64 = 0;
  while k < n
    invariant 0 <= k && k <= n
    decreases n - k
  {
    if a[i + k] != b[j + k] { return false; }
    k = k + 1;
  }
  return true;
}
"""
NV_CHECK = re.compile(r"^[ \t]*if !crate::bud_store::valid\(st\) \{ return 2; \}[ \t]*\r?\n", re.M)
CLANG = shutil.which("clang") or "/usr/bin/clang"
RUNTIME_C = os.path.join(ROOT, "pruvc", "runtime.c")
WORLD_BEGIN = "// LANE-WORLD-BEGIN"
WORLD_END = "// LANE-WORLD-END"


def root_text(nv):
    with open(os.path.join(HERE, "root_m4.pruv")) as fh:
        text = fh.read()
    if nv:
        text, n = NV_CHECK.subn("", text)
        if n != 2:    # m4_op and m4_observe (m4_grow checks `prev`, which stays)
            raise ToolError("nv: expected 2 `valid(st)` checks in root_m4.pruv, found %d" % n)
    return text


def prepare_m4_pkg(tag, raw, nv, eq=False):
    """work/m34/pkg_<tag>/: root_m4.pruv (nv: checks removed) + its modules (eq: eq_range -> eqr_p10) + pruv.json with
    raw_entries = raw (no key for None). Returns (manifest, root)."""
    pkg = os.path.join(WORK, "pkg_" + tag)
    os.makedirs(pkg, exist_ok=True)
    with open(os.path.join(pkg, "root_m4.pruv"), "w") as fh:
        fh.write(root_text(nv))
    for d, m in M4RAW_MODS:
        src = os.path.join(ROOT, *d.split("/"), m + ".pruv")
        if not os.path.exists(src):
            raise ToolError("missing module %s" % src)
        with open(src) as fh:
            text = fh.read()
        if eq and EQ_CALL.search(text):
            text = EQ_CALL.sub("eqr_p10(", text) + EQ_HELPER
        with open(os.path.join(pkg, m + ".pruv"), "w") as fh:
            fh.write(text)
    manifest = os.path.join(pkg, "pruv.json")
    with open(manifest, "w") as fh:
        fh.write('{"name": "%s", "version": "0.1.0", "root": "root_m4.pruv"%s}\n'
                 % (tag, ', "raw_entries": "%s"' % raw if raw else ""))
    return manifest, os.path.join(pkg, "root_m4.pruv")


def prepare_m4_root_nv(tag):
    """Manifest-less nv lanes: the edited root alone in work/m34/pkg_<tag>/ (modules come from LIBS). Returns its path."""
    pkg = os.path.join(WORK, "pkg_" + tag)
    os.makedirs(pkg, exist_ok=True)
    root = os.path.join(pkg, "root_m4.pruv")
    with open(root, "w") as fh:
        fh.write(root_text(True))
    return root


def build_runtime():
    """runtime.c -> work/m34/runtime.o (LLVM lanes link it; -DPRUV_RUST_ALLOCATOR as bench/g5/perf.py does)."""
    rt = os.path.join(WORK, "runtime.o")
    c = run([CLANG, "-O2", "-std=c11", "-DPRUV_RUST_ALLOCATOR", "-c", RUNTIME_C, "-o", rt], timeout=300)
    if c.returncode != 0:
        raise ToolError("clang (runtime.c) failed:\n%s" % tail(c))
    return rt


def build(pruvc, tag):
    """tag m3|keys|<m4 lane, see M4_LANES>: pruvc root -> gen_<tag>.rs (llvm lanes: gen_<tag>.ll + bindings gen_<tag>.rs,
    clang -> .o, linked with runtime.o), then rustc main_<tag>.rs + driver. m4raw/m4llvmraw: manifest raw_entries "crate"
    (driver over sol::pruv_raw::PruvStore); m4c: raw_entries "c" (driver_m4c.rs world calls the sol::pruv_c extern "C"
    symbols, prefixed m4c__); m4cb: the same with driver_m4cb.rs (m4cb__ symbols) and one batch call (hb) per sessions rep;
    m4/m4nv/m4llvm/m4llvmnv use the local no-poison Plain store. Returns the exe."""
    lane = M4_LANES.get(tag)
    base = "m4" if lane else tag
    root_rel = "bench/l3/budget/m34_test/root_%s.pruv" % base
    if not os.path.exists(os.path.join(ROOT, root_rel)):
        raise ToolError("missing %s (written by the Pruv-side workers)" % root_rel)
    if not os.path.exists(pruvc):
        raise ToolError("compiler not found: %s (cd pruvc-rs && cargo build --release --locked)" % pruvc)
    rustc = find_rustc()
    if not rustc:
        raise ToolError("rustc not found")
    os.makedirs(WORK, exist_ok=True)
    gen = os.path.join(WORK, "gen_%s.rs" % tag)
    llvm = bool(lane) and lane["backend"] == "llvm"
    ll = os.path.join(WORK, "gen_%s.ll" % tag)
    if lane and (lane["raw"] or lane["eq"]):
        manifest, root = prepare_m4_pkg(tag, lane["raw"], lane["nv"], lane["eq"])
        args = [root, "-o", ll if llvm else gen, "--manifest", manifest]
    elif lane and lane["nv"]:
        args = [prepare_m4_root_nv(tag), "-o", ll if llvm else gen] + LIBS
    else:
        args = [root_rel, "-o", ll if llvm else gen] + LIBS
    if llvm:
        args += ["--backend", "llvm", "--rust-bindings", gen]
    c = run(pruvc_cmd(pruvc) + args, cwd=ROOT)
    if c.returncode != 0:
        raise ToolError("pruvc failed on %s (%s, exit %d):\n%s" % (root_rel, tag, c.returncode, tail(c)))
    link = []
    if llvm:
        obj = os.path.join(WORK, "gen_%s.o" % tag)
        c = run([CLANG, "-O2", "-Wno-override-module", "-c", ll, "-o", obj], timeout=300)
        if c.returncode != 0:
            raise ToolError("clang (%s .ll) failed:\n%s" % (tag, tail(c)))
        link = ["-C", "link-arg=" + obj, "-C", "link-arg=" + build_runtime()]
    with open(os.path.join(HERE, "driver_%s.rs" % base)) as d:
        driver = d.read()
    if tag in WORLD_LANES:
        b, e = driver.find(WORLD_BEGIN), driver.find(WORLD_END)
        if b < 0 or e < b:
            raise ToolError("driver_m4.rs: LANE-WORLD markers missing")
        with open(os.path.join(HERE, "driver_%s.rs" % tag)) as w:
            driver = driver[:b] + w.read() + driver[e:]
    if tag == "m4cb":
        if driver.count(BATCH_LOOP_ANCHOR) != 1:
            raise ToolError("driver_m4.rs: the timed loop of perf() changed, cannot patch the m4cb batch loop")
        driver = driver.replace(BATCH_LOOP_ANCHOR, BATCH_LOOP_PATCH)
    main = os.path.join(WORK, "main_%s.rs" % tag)
    with open(main, "w") as fh:
        fh.write('#![allow(dead_code, unused_imports)]\n#[path = "%s"] mod sol;\n' % gen.replace("\\", "/"))
        if lane and tag not in WORLD_LANES:
            fh.write("type Store<T> = %s;\n" % ("sol::pruv_raw::PruvStore<T>" if lane["raw"] == "crate" else "Plain<T>"))
        fh.write(driver)
    exe = os.path.join(WORK, "driver_%s_bin" % tag + (".exe" if os.name == "nt" else ""))
    c = run([rustc, "-C", "opt-level=3", "--edition", "2021", "-o", exe, main] + link)
    if c.returncode != 0:
        raise ToolError("rustc failed (%s):\n%s" % (tag, tail(c)))
    return exe


def show(words, n=40):
    s = " ".join(str(x) for x in words[:n])
    return s + (" ... (%d words)" % len(words) if len(words) > n else "")


# ---- m3 -------------------------------------------------------------------------------------------------------------

def cmd_m3(args):
    exe = build(args.pruvc, "m3")
    cases = gen_m3.make_cases(args.seed, args.count)
    # The generator's own size formula check (valid cases).
    size_bad = [c for c in cases if c["expect"][0] == "ok" and c["size"] != len(c["words"])]
    c = run([exe], inp="\n".join(gen_m3.input_line(x) for x in cases) + "\n")
    if c.returncode != 0:
        raise ToolError("driver_m3 exit %d: %s" % (c.returncode, tail(c)))
    lines = c.stdout.splitlines()
    if len(lines) != len(cases):
        raise ToolError("driver_m3 printed %d lines for %d cases" % (len(lines), len(cases)))
    bad = []
    percat = {}
    for i, (case, line) in enumerate(zip(cases, lines)):
        got = [int(t) for t in line.split()]
        exp = gen_m3.expected_words(case)
        ok = got == exp
        pc = percat.setdefault(case["cat"].split("-")[0] if case["cat"].startswith("valid") else case["cat"], [0, 0])
        pc[1] += 1
        if ok:
            pc[0] += 1
        else:
            bad.append((i, case, exp, got))
    passed = len(cases) - len(bad)
    print("m3: %d/%d pass (seed %d)" % (passed, len(cases), args.seed))
    print("m3: size formula for valid cases: %s" % ("ok" if not size_bad else "%d generator mismatches" % len(size_bad)))
    print("m3: by category: " + ", ".join("%s %d/%d" % (k, v[0], v[1]) for k, v in sorted(percat.items())))
    for i, case, exp, got in bad[:5]:
        print("MISMATCH case %d kind=%s cat=%s" % (i, case["kind"], case["cat"]))
        print("  input   : %s" % show(case["words"]))
        print("  expected: %s" % show(exp))
        print("  got     : %s" % show(got))
    return 0 if not bad and not size_bad else 1


# ---- m4 -------------------------------------------------------------------------------------------------------------

def layout_slots(L):
    """Largest power-of-two slots >= 8 with 16 + slots + (slots/2)*20 + (slots/2)*32 <= L (CONTRACT.md); None if none."""
    def need(s):
        return 16 + s + (s // 2) * 20 + (s // 2) * 32
    if need(8) > L:
        return None
    s = 8
    while need(s * 2) <= L:
        s *= 2
    return s


def check_stats(exp_line, act_line, prev_len):
    """exp `stats <live non-root>`; act `stats live used arena len`. Returns (error or None, len)."""
    p = act_line.split()
    if p[0] != "stats" or len(p) != 5:
        return "bad stats line (len(st) reported by the entry differs from the host buffer?)", prev_len
    live, used, arena, L = [int(x) for x in p[1:]]
    ew = exp_line.split()
    want, ghosts = int(ew[1]), int(ew[2])
    # header word 12 counts exactly the three roots and the live entries: remove frees the children at once, so the
    # model's ghost count (an upper bound kept for reporting) is not allowed for
    if live != want + 3:
        return "allocated entries %d, model %d non-root + 3 roots (%d children cut off so far)" % (live, want, ghosts), L
    slots = layout_slots(L)
    if slots is None:
        return "len(st)=%d is below the minimum layout" % L, L
    if live > slots // 2:
        return "live entries %d exceed capacity %d (layout %d)" % (live, slots // 2, L), L
    if used > slots * 3 // 4 or used < want:
        return "used slots %d outside [%d, %d]" % (used, want, slots * 3 // 4), L
    if arena > L - 16 - slots - (slots // 2) * 20:
        return "live arena words %d exceed the arena of layout(%d)" % (arena, L), L
    if prev_len is not None and L < prev_len:
        return "len(st) shrank %d -> %d" % (prev_len, L), L
    return None, L


def cmd_m4raw(args):
    return cmd_m4(args, "m4raw")


def cmd_m4(args, tag="m4"):
    exe = build(args.pruvc, tag)
    progs = gen_m4.gen_programs(args.seed, args.count)
    bad = []
    grows_total = 0
    with_grow = 0
    stats_checks = 0
    stats_bad = 0
    for pi, pr in enumerate(progs):
        c = run([exe, "q"], inp="\n".join(pr["lines"]) + "\n", timeout=300)
        if c.returncode != 0:
            bad.append((pi, 0, "(driver)", "exit 0", "exit %d: %s" % (c.returncode, tail(c))))
            continue
        out = c.stdout.splitlines()
        g = 0
        while out and (out[-1].startswith("grows ") or out[-1].startswith("final_len ")):
            if out[-1].startswith("grows "):
                g = int(out[-1].split()[1])
            out.pop()
        grows_total += g
        with_grow += 1 if g else 0
        if len(out) != len(pr["expected"]):
            bad.append((pi, -1, "(line count)", "%d lines" % len(pr["expected"]), "%d lines" % len(out)))
            continue
        prev_len = None
        for li, (inp, e, a) in enumerate(zip(pr["lines"], pr["expected"], out)):
            if e.startswith("stats "):
                stats_checks += 1
                err, prev_len = check_stats(e, a, prev_len)
                if err:
                    stats_bad += 1
                    bad.append((pi, li, inp, e + "   [" + err + "]", a))
                    break
            elif e != a:
                bad.append((pi, li, inp, e, a))
                break
    ok_progs = len(progs) - len(set(b[0] for b in bad))
    print("%s: %d/%d pass (seed %d)" % (tag, ok_progs, len(progs), args.seed))
    print("%s: grows total %d across %d/%d programs" % (tag, grows_total, with_grow, len(progs)))
    print("%s: size formula / header-bound checks on stats lines: %d/%d ok" % (tag, stats_checks - stats_bad, stats_checks))
    if grows_total == 0:
        print("%s: WARNING no grow was exercised" % tag)
    for pi, li, inp, e, a in bad[:5]:
        print("MISMATCH program %d line %d: %s" % (pi, li, inp[:200]))
        print("  expected: %s" % e[:600])
        print("  got     : %s" % a[:600])
    return 0 if not bad else 1


# ---- keys -----------------------------------------------------------------------------------------------------------

def cmd_keys(args):
    exe = build(args.pruvc, "keys")
    cases = gen_keys.make_cases(args.seed, args.count)
    c = run([exe], inp="\n".join(x[0] for x in cases) + "\n", timeout=900)
    if c.returncode != 0:
        raise ToolError("driver_keys exit %d: %s" % (c.returncode, tail(c)))
    lines = c.stdout.splitlines()
    if len(lines) != len(cases):
        raise ToolError("driver_keys printed %d lines for %d cases" % (len(lines), len(cases)))
    bad = []
    percat = {}
    for i, ((line, exp, cat), got) in enumerate(zip(cases, lines)):
        pc = percat.setdefault(cat, [0, 0])
        pc[1] += 1
        if got.split() == exp.split():
            pc[0] += 1
        else:
            bad.append((i, cat, line, exp, got))
    print("keys: %d/%d pass (seed %d)" % (len(cases) - len(bad), len(cases), args.seed))
    print("keys: by category: " + ", ".join("%s %d/%d" % (k, v[0], v[1]) for k, v in sorted(percat.items())))
    for i, cat, line, exp, got in bad[:5]:
        print("MISMATCH case %d cat=%s: %s" % (i, cat, line[:300]))
        print("  expected: %s" % exp[:300])
        print("  got     : %s" % got[:300])
    return 0 if not bad else 1


# Expected admit result per case of root_admit.pruv (14, 19, 20 are valid stores: stale tag, slots left by removal,
# reused entry under a stale slot), the exact (case, len) rows driver_admit.rs prints, and the slot count per length.
ADMIT_EXPECT = [10, -1, -2, -3, -4, -5, -5, -5, -6, -7, -8, -9, -10, -10, 10, -10, -4, -8, -7, 7, 8]
ADMIT_ROWS = [(c, 1024) for c in range(21)] + [(0, 4096), (0, 65536)]
ADMIT_SLOTS = {1024: 32, 4096: 128, 65536: 2048}


def cmd_admit(args):
    exe = build(args.pruvc, "admit")
    c = run([exe])
    if c.returncode != 0:
        raise ToolError("driver_admit exit %d: %s" % (c.returncode, tail(c)))
    rows = [[int(x) for x in l.split()] for l in c.stdout.split("\n") if l.strip()]
    bad = []
    if [(r[0], r[1]) for r in rows] != ADMIT_ROWS:
        bad.append("rows: got %s, expected %s" % ([(r[0], r[1]) for r in rows], ADMIT_ROWS))
    for case, n, r, ce, cf, cs in rows:
        exp = ADMIT_EXPECT[case] if 0 <= case < len(ADMIT_EXPECT) else None
        if r != exp:
            bad.append("case %d len %d: result %d, expected %s" % (case, n, r, exp))
        if case == 0 and (ce, cf, cs) != (11, 1, ADMIT_SLOTS.get(n)):
            bad.append("case 0 len %d: visited %d entries / %d free steps / %d slots, expected 11 / 1 / %s"
                       % (n, ce, cf, cs, ADMIT_SLOTS.get(n)))
        print("admit case %2d len %6d -> %4d  (entries %d, free %d, slots %d)" % (case, n, r, ce, cf, cs))
    for b in bad:
        print("MISMATCH " + b)
    npass = len(rows) - len(set(b.split(":")[0] for b in bad if b.startswith("case")))
    print("admit: %d/%d pass" % (npass if len(rows) == len(ADMIT_ROWS) else 0, len(ADMIT_ROWS)))
    return 0 if not bad else 1


# ---- perf -----------------------------------------------------------------------------------------------------------

KV =re.compile(r"(\w+)=(\S+)")


def parse_kv(text):
    return dict((m.group(1), m.group(2)) for m in KV.finditer(text))


def median_run(cmd, key, runs):
    vals = []
    last = None
    for _ in range(runs):
        c = run(cmd, timeout=900)
        if c.returncode != 0:
            raise ToolError("%s exit %d: %s" % (" ".join(cmd), c.returncode, tail(c)))
        last = parse_kv(c.stdout.strip().splitlines()[-1])
        vals.append(float(last[key]))
    return statistics.median(vals), last


def median_alternating(cmd_a, key_a, cmd_b, key_b, runs):
    """Runs A,B,A,B,... (so load drifts hit both alike); returns (median A, last A output, median B)."""
    va, vb = [], []
    last_a = None
    for _ in range(runs):
        ca = run(cmd_a, timeout=900)
        if ca.returncode != 0:
            raise ToolError("%s exit %d: %s" % (" ".join(cmd_a), ca.returncode, tail(ca)))
        last_a = parse_kv(ca.stdout.strip().splitlines()[-1])
        va.append(float(last_a[key_a]))
        cb = run(cmd_b, timeout=900)
        if cb.returncode != 0:
            raise ToolError("%s exit %d: %s" % (" ".join(cmd_b), cb.returncode, tail(cb)))
        vb.append(float(parse_kv(cb.stdout.strip().splitlines()[-1])[key_b]))
    return statistics.median(va), last_a, statistics.median(vb)


def cmd_perf(args):
    exe4 = build(args.pruvc, "m4")
    exe3 = None
    try:
        exe3 = build(args.pruvc, "m3")
    except ToolError as e:
        print("note: m3 codec line skipped: %s" % str(e).splitlines()[0], file=sys.stderr)
    node = shutil.which("node")
    if not node:
        print("note: node not found, TS column is n/a", file=sys.stderr)
    sizes = [100, 500, 1000]
    reps = 200000
    rows = []
    for axis in args.axes.split(","):
        for n in sizes:
            pruv_cmd = [exe4, "perf", axis, str(n), str(reps)]
            if node:
                core, last, ts = median_alternating(
                    pruv_cmd, "core_ns_per_op",
                    [node, os.path.join(HERE, "perf_ts.mjs"), axis, str(n), str(reps)], "ts_ns_per_op", args.runs)
            else:
                core, last = median_run(pruv_cmd, "core_ns_per_op", args.runs)
                ts = None
            enc = float(last["encode_ns_per_op"])
            rows.append({"axis": axis, "n": n, "ts": ts, "core": core, "enc": enc, "grows": last.get("grows"),
                         "grow_ns": last.get("grow_ns_total")})
    print("| axis | n | TS ns/op | Pruv core ns/op | Pruv encode ns/op | ratio (core/TS) | grows | grow ns total |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|")
    for r in rows:
        print("| %s | %d | %s | %.1f | %.1f | %s | %s | %s |" % (
            r["axis"], r["n"], "n/a" if r["ts"] is None else "%.1f" % r["ts"], r["core"], r["enc"],
            "n/a" if r["ts"] is None else "%.2f" % (r["core"] / r["ts"]), r["grows"], r["grow_ns"]))
    print("")
    if exe3:
        v, last = median_run([exe3, "perf", "200000"], "m3_call_ns", args.runs)
        print("M3 codec: %.1f ns per m3_call (typical 3-argument call, %s words), counted as encode time" % (v, last.get("words")))
        print("")
    flat_all = True
    slower_all = True
    for axis in args.axes.split(","):
        ar = [r for r in rows if r["axis"] == axis]
        lo = [r for r in ar if r["n"] == 100][0]["core"]
        hi = [r for r in ar if r["n"] == 1000][0]["core"]
        flat = hi <= 1.5 * lo
        not_slower = None if not node else all(r["core"] <= r["ts"] for r in ar)
        flat_all = flat_all and flat
        slower_all = slower_all and (not_slower is not False)
        print("axis %s: core n=1000/n=100 = %.2f -> %s; %s" % (
            axis, hi / lo if lo else float("inf"), "FLAT" if flat else "NOT FLAT",
            "Pruv core <= TS at every size -> NOT SLOWER" if not_slower else
            ("n/a (no node)" if not_slower is None else "Pruv core > TS at some size -> SLOWER")))
    print("verdict: %s, %s" % ("FLAT" if flat_all else "NOT FLAT", "NOT SLOWER" if slower_all and node else
                               ("SLOWER" if node else "SLOWER-unknown (no node)")))
    return 0 if flat_all and slower_all and node else 1


def main():
    ap = argparse.ArgumentParser(description="L3-10 M3/M4 test harness")
    ap.add_argument("mode", choices=["m3", "perf", "keys", "admit", "lanes"] + sorted(M4_LANES))
    ap.add_argument("--only", choices=sorted(M4_LANES), default=None,
                    help="m4 mode: run only this lane (same as giving the lane name as the mode); mode `lanes` runs all")
    ap.add_argument("--pruvc", default=PRUVC_DEFAULT)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--count", type=int, default=300)
    ap.add_argument("--runs", type=int, default=5, help="perf: runs per measurement, median is used")
    ap.add_argument("--axes", default="sessions,tools", help="perf: comma list of sessions,tools,clocks")
    args = ap.parse_args()
    try:
        if args.mode in M4_LANES:
            return cmd_m4(args, args.only or args.mode)
        if args.mode == "lanes":
            rc = 0
            for tag in ([args.only] if args.only else list(M4_LANES)):
                rc = max(rc, cmd_m4(args, tag))
            return rc
        return {"m3": cmd_m3, "perf": cmd_perf, "keys": cmd_keys, "admit": cmd_admit}[args.mode](args)
    except ToolError as e:
        print("error: %s" % e, file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
