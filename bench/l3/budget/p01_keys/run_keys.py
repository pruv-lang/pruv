#!/usr/bin/env python3
"""P01 budget key layouts runner (keys.pruv, driver_keys.rs, hand_keys.rs; formats in the keys.pruv header).

  python3 bench/l3/budget/p01_keys/run_keys.py check [--count 300] [--seed 20261117] [--pruvc PATH]
  python3 bench/l3/budget/p01_keys/run_keys.py perf  [--rounds 7] [--sizes 100,500] [--sets sid,uuid,nonascii,surr]
                                                     [--reps N] [--no-instr] [--instr-reps N] [--instr-samples N]
                                                     [--inline-variant] [--pruvc PATH]

check: generates `count` deterministic cases (pairs of UTF-16 unit lists: empty, NUL, lengths 1..17 so every tail
0..7 occurs, ASCII, non-ASCII BMP, units 128..255, astral pairs, lone high/low surrogates; equal pairs, prefix pairs,
differ-in-last-unit pairs, form transitions) plus fixed noncanonical records per layout, runs `driver_keys check`
and compares with a Python oracle: Python's own encoding of every layout, k_check 0 on encodings and 1 on the
negatives, find/insert hit/miss by list equality (both hash kinds), k_cmp by Python list order (= UTF-16 code-unit
order), and the unit hash equal across layouts and equal to a Python port of bud_map::hash_key. Prints the mismatch
count (must be 0).

perf: builds (pruvc, then rustc -C opt-level=3 for the driver and the hand lane; with --inline-variant also a driver
from gen.rs with every `#[inline(always)]` replaced by `#[inline]`), runs every (set, size, layout or hand) in a
separate process with the side order rotated every round, and reports median/min/max ns per op per
(set, L, H, row, N). Instructions per op (`/usr/bin/time -l`, macOS) for the ret and raw rows: one (row, L, H) per
run, retired-instruction difference between --instr-reps and 5x --instr-reps, divided by the extra operations.
Raw JSON goes to bench/l3/budget/work/p01_keys/.
"""
import argparse, json, os, random, shutil, statistics, subprocess, sys, time

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
BUD = os.path.dirname(HERE)
ROOT = os.path.abspath(os.path.join(BUD, "..", "..", ".."))
WORK = os.path.join(BUD, "work", "p01_keys")
PRUVC_DEFAULT = "/Users/spica4345/Projects/pruv/pruvc-rs/target/release/pruvc"
LIBS = ["--lib", "bench/l3/budget", "--lib", "bench/l3/lib"]
ROOT_PRUV = "bench/l3/budget/p01_keys/keys.pruv"
LAYS = ["U", "P", "A8", "F1"]
SETS = ["sid", "uuid", "nonascii", "surr"]
HEADER = ('#![allow(dead_code, unused_imports, unused_variables, unused_mut, non_snake_case, unused_parens, '
          'unused_assignments)]\n#[path = "%s"] mod sol;\n')


# ---- build ----
def rustc():
    return shutil.which("rustc") or os.path.expanduser("~/.cargo/bin/rustc")


def compile_gen(pruvc, gen):
    c = subprocess.run([pruvc, ROOT_PRUV, "-o", gen] + LIBS, cwd=ROOT, capture_output=True, text=True)
    if c.returncode != 0 or not os.path.exists(gen):
        sys.exit("pruvc failed:\n" + c.stdout[-4000:] + c.stderr[-4000:])


def rustc_build(src, exe):
    c = subprocess.run([rustc(), "-C", "opt-level=3", "--edition", "2021", "-o", exe, src], capture_output=True,
                       text=True)
    if c.returncode != 0:
        sys.exit("rustc failed (%s):\n%s" % (os.path.basename(src), c.stderr[-6000:]))


def build_driver(gen_name, exe_name):
    main = os.path.join(WORK, exe_name + ".rs")
    with open(main, "w") as f:
        f.write(HEADER % gen_name)
        f.write(open(os.path.join(HERE, "driver_keys.rs"), encoding="utf-8").read())
    exe = os.path.join(WORK, exe_name)
    rustc_build(main, exe)
    return exe


def build(args, hand=False, inline=False):
    os.makedirs(WORK, exist_ok=True)
    gen = os.path.join(WORK, "gen.rs")
    compile_gen(args.pruvc, gen)
    exes = {"drv": build_driver("gen.rs", "driver_keys")}
    if inline:
        txt = open(gen, encoding="utf-8").read()
        n_always = txt.count("#[inline(always)]")
        with open(os.path.join(WORK, "gen_inl.rs"), "w", encoding="utf-8") as f:
            f.write(txt.replace("#[inline(always)]", "#[inline]"))
        exes["inl"] = build_driver("gen_inl.rs", "driver_keys_inl")
        exes["inline_always_replaced"] = n_always
    if hand:
        exe = os.path.join(WORK, "hand_keys")
        rustc_build(os.path.join(HERE, "hand_keys.rs"), exe)
        exes["hand"] = exe
    return exes


# ---- Python model of the layouts ----
M64 = (1 << 64) - 1


def s64(x):
    x &= M64
    return x - (1 << 64) if x >= (1 << 63) else x


def pack(vals, bits, per):
    """Big-endian lanes of `bits` bits, `per` lanes per word, zero tail padding."""
    out = []
    for i in range(0, len(vals), per):
        w = 0
        for j, v in enumerate(vals[i:i + per]):
            w |= v << ((per - 1 - j) * bits)
        out.append(s64(w))
    return out


def py_enc(L, u):
    n = len(u)
    asc = all(x < 128 for x in u)
    if L == "U":
        return [n] + list(u)
    if L == "P":
        return [n] + pack(u, 16, 4)
    if L == "A8":
        return ([2 * n] + pack(u, 8, 8)) if asc else ([2 * n + 1] + pack(u, 16, 4))
    if L == "F1":
        if asc:
            return [2 * n, n] + pack(u, 8, 8)
        b = [0xFF]
        for x in u:
            b += [x & 0xFF, x >> 8]
        return [2 * len(b) + 1, n] + pack(b, 8, 8)
    raise ValueError(L)


def mixu(h, v):
    return ((h ^ v) * 0x9E3779B97F4A7C15) & M64


def finu(h):
    a = h ^ (h >> 33)
    b = (a * 0xFF51AFD7ED558CCD) & M64
    return b ^ (b >> 33)


def py_hash_u(u):
    """bud_map::hash_key(owner 0, generation 0, units)."""
    h = mixu(1469598103934665603, 0)
    n = len(u)
    for j in range(0, n, 4):
        g = list(u[j:j + 4]) + [0, 0, 0]
        h = mixu(h, (g[0] + (g[1] << 16) + (g[2] << 32) + (g[3] << 48)) & M64)
    return s64(finu(h ^ n))


# ---- cases ----
PROFILES = ["ascii", "nul", "bmp", "mixed", "astral", "lone_hi", "lone_lo", "hi128", "edge"]
MODES = ["equal", "prefix", "extend", "last", "random", "cross", "empty", "transition"]
EDGE = [0, 0x7F, 0x80, 0xFF, 0x100, 0x7FFF, 0x8000, 0xFF00, 0xFF7F, 0xFFFF, 0xD800, 0xDBFF, 0xDC00, 0xDFFF]


def gen_cases(count, seed):
    rng = random.Random(seed)

    def unit(p):
        if p == "ascii":
            return rng.randrange(32, 127)
        if p == "nul":
            return rng.choice([0, 0, rng.randrange(1, 128)])
        if p == "bmp":
            return rng.choice([rng.randrange(0x80, 0x800), rng.randrange(0x800, 0xD800), rng.randrange(0xE000, 0x10000)])
        if p == "hi128":
            return rng.randrange(0x80, 0x100)
        if p == "edge":
            return rng.choice(EDGE)
        raise ValueError(p)

    def seq(p, n):
        out = []
        while len(out) < n:
            if p == "astral":
                cp = rng.randrange(0x10000, 0x110000) - 0x10000
                out += [0xD800 + (cp >> 10), 0xDC00 + (cp & 0x3FF)]
            elif p == "lone_hi":
                out.append(rng.choice([0xD800 + rng.randrange(0x400), rng.randrange(32, 127)]))
            elif p == "lone_lo":
                out.append(rng.choice([0xDC00 + rng.randrange(0x400), rng.randrange(32, 127)]))
            elif p == "mixed":
                out.append(unit(rng.choice(["ascii", "bmp", "nul", "hi128"])))
            else:
                out.append(unit(p))
        return out[:n]

    cases = []
    for i in range(count):
        n = 1 + i % 17
        p = PROFILES[(i // 17) % len(PROFILES)]
        mode = MODES[i % len(MODES)]
        a = seq(p, n)
        if mode == "equal":
            b = list(a)
        elif mode == "prefix":
            b = a[:rng.randrange(0, n)]
        elif mode == "extend":
            b = a + seq(p, rng.randrange(1, 10))
        elif mode == "last":
            b = list(a)
            b[-1] = (b[-1] + rng.randrange(1, 65536)) % 65536
        elif mode == "random":
            b = seq(p, rng.randrange(0, 18))
        elif mode == "cross":
            b = seq(rng.choice(PROFILES), n)
        elif mode == "empty":
            if i % 3 == 0:
                a, b = [], []
            elif i % 3 == 1:
                a, b = [], a
            else:
                b = []
        else:  # transition: one unit of an otherwise equal key crosses the ASCII / wide boundary
            b = list(a)
            b[rng.randrange(n)] = rng.choice([0x7F, 0x80, 0xFF, 0x100, 0xD800, 0xDC00, 0xFFFF, 0])
        if rng.random() < 0.5:
            a, b = b, a
        cases.append({"id": "c%d" % i, "a": a, "b": b, "mode": mode, "profile": p})
    return cases


def negatives():
    W = s64
    neg = []

    def add(L, why, rec):
        neg.append({"id": "neg-%s-%d" % (L, len(neg)), "neg": L, "why": why, "rec": rec})

    add("U", "empty record", [])
    add("U", "negative length", [-1])
    add("U", "length > words", [2, 65])
    add("U", "length < words", [1, 65, 66])
    add("U", "unit > 65535", [1, 65536])
    add("U", "negative unit", [2, 65, -1])
    add("P", "empty record", [])
    add("P", "nonzero padding n=1", [1, W((65 << 48) | 1)])
    add("P", "nonzero padding n=3", [3, W((65 << 48) | (66 << 32) | (67 << 16) | 9)])
    add("P", "too few words", [5, W(65 << 48)])
    add("P", "too many words", [1, W(65 << 48), 0])
    add("P", "empty key with a word", [0, 0])
    add("P", "negative length", [-4])
    add("A8", "empty record", [])
    add("A8", "ASCII key in fallback form", [3] + pack([65], 16, 4))
    add("A8", "ASCII key with NUL in fallback form", [5] + pack([0, 65], 16, 4))
    add("A8", "empty key in fallback form", [1])
    add("A8", "tag0 nonzero padding", [2, W((65 << 56) | 1)])
    add("A8", "tag0 byte >= 128", [2, W(0x80 << 56)])
    add("A8", "tag0 too few words", [2 * 9] + pack([65] * 8, 8, 8))
    add("A8", "tag0 extra word", [2] + pack([65], 8, 8) + [0])
    add("A8", "tag1 nonzero padding", [3, W((0x100 << 48) | 5)])
    add("A8", "tag1 too few words", [2 * 5 + 1] + pack([0x100] * 4, 16, 4))
    add("A8", "negative header", [-2])
    add("F1", "empty record", [])
    add("F1", "missing unit length word", [0])
    add("F1", "tag0 byte >= 128", [2, 1, W(0x80 << 56)])
    add("F1", "tag0 byte 0xFF", [2, 1, W(0xFF << 56)])
    add("F1", "tag0 unit length mismatch", [2 * 2, 3] + pack([65, 66], 8, 8))
    add("F1", "tag0 nonzero padding", [2, 1, W((65 << 56) | 1)])
    add("F1", "tag0 too few words", [2 * 9, 9] + pack([65] * 8, 8, 8))
    add("F1", "tag1 even byte length", [2 * 2 + 1, 1] + pack([0xFF, 0x00], 8, 8))
    add("F1", "tag1 byte length != 1+2n", [2 * 5 + 1, 1] + pack([0xFF, 0, 1, 0, 1], 8, 8))
    add("F1", "tag1 first byte not FF", [2 * 3 + 1, 1] + pack([0xFE, 0x00, 0x01], 8, 8))
    add("F1", "ASCII key in fallback form", [2 * 3 + 1, 1] + pack([0xFF, 65, 0], 8, 8))
    add("F1", "tag1 nonzero padding", [2 * 3 + 1, 1, W((0xFF << 56) | (0x01 << 40) | 7)])
    add("F1", "tag1 empty key", [1, 0])
    add("F1", "tag1 extra word", [2 * 3 + 1, 1] + pack([0xFF, 0x00, 0x01], 8, 8) + [0])
    add("F1", "negative header", [-2, 0])
    # the positive twins of some negatives must pass (guards against a check that rejects everything)
    pos = [
        ("U", [1, 65535]), ("P", [3, W((65 << 48) | (66 << 32) | (67 << 16))]), ("A8", [2, W(65 << 56)]),
        ("A8", [3, W(0x100 << 48)]), ("F1", [2 * 3 + 1, 1] + pack([0xFF, 0x00, 0x01], 8, 8)),
        ("F1", [2 * 3 + 1, 1] + pack([0xFF, 0x80, 0x00], 8, 8)), ("F1", [0, 0]), ("U", [0]), ("P", [0]), ("A8", [0]),
    ]
    for k, (L, rec) in enumerate(pos):
        neg.append({"id": "pos-%s-%d" % (L, k), "neg": L, "why": "canonical control", "rec": rec, "want": 0})
    return neg


def compare_case(c, g, mism):
    a, b = c["a"], c["b"]
    eq = a == b
    want_cmp = (a > b) - (a < b)
    hu = py_hash_u(a), py_hash_u(b)

    def bad(L, what, got, want):
        mism.append("%s [%s/%s] %s %s: got %r want %r (a=%r b=%r)" % (c["id"], c["mode"], c["profile"], L, what, got,
                                                                     want, a, b))

    for L in LAYS:
        x = g[L]
        for what, got, want in (
            ("rec_a", x["rec_a"], py_enc(L, a)), ("rec_b", x["rec_b"], py_enc(L, b)),
            ("chk_a", x["chk_a"], 0), ("chk_b", x["chk_b"], 0),
            ("hu_a", x["hu_a"], hu[0]), ("hu_b", x["hu_b"], hu[1]),
            ("cmp_ab", x["cmp_ab"], want_cmp), ("cmp_ba", x["cmp_ba"], -want_cmp), ("cmp_aa", x["cmp_aa"], 0),
        ):
            if got != want:
                bad(L, what, got, want)
        for H in ("u", "w"):
            exp = {"ins_a": 0, "find_a": 0, "find_b": 0 if eq else -1, "ins_b": 0 if eq else 1,
                   "find_b2": 0 if eq else 1, "find_a2": 0}
            for k, v in exp.items():
                if x[H][k] != v:
                    bad(L, "%s.%s" % (H, k), x[H][k], v)


def cmd_check(args):
    exes = build(args)
    cases = gen_cases(args.count, args.seed)
    negs = negatives()
    lines = [json.dumps({"id": c["id"], "a": c["a"], "b": c["b"]}) for c in cases]
    lines += [json.dumps({"id": n["id"], "neg": n["neg"], "rec": n["rec"]}) for n in negs]
    r = subprocess.run([exes["drv"], "check"], input="\n".join(lines) + "\n", capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit("driver check failed:\n" + r.stderr[-3000:])
    got = {}
    for l in r.stdout.splitlines():
        if l.strip():
            o = json.loads(l)
            got[o["id"]] = o
    mism = []
    for c in cases:
        if c["id"] not in got:
            mism.append("%s: no driver output" % c["id"])
            continue
        compare_case(c, got[c["id"]], mism)
    for n in negs:
        want = n.get("want", 1)
        g = got.get(n["id"])
        if g is None or g["chk"] != want:
            mism.append("%s (%s, %s): k_check got %r want %d, rec=%r" % (n["id"], n["neg"], n["why"],
                                                                         None if g is None else g["chk"], want,
                                                                         n["rec"]))
    tails = sorted({len(c["a"]) % 8 for c in cases} | {len(c["b"]) % 8 for c in cases})
    summary = {"cases": len(cases), "negatives": sum(1 for n in negs if "want" not in n),
               "controls": sum(1 for n in negs if "want" in n), "seed": args.seed, "tails_mod8": tails,
               "mismatches": len(mism)}
    os.makedirs(WORK, exist_ok=True)
    with open(os.path.join(WORK, "check.json"), "w") as f:
        json.dump({"summary": summary, "mismatches": mism[:200]}, f, indent=1, ensure_ascii=False)
    print(json.dumps(summary))
    for m in mism[:20]:
        print("MISMATCH", m[:600])
    print("mismatches: %d" % len(mism))
    return 1 if mism else 0


# ---- perf ----
def run_proc(cmd):
    r = subprocess.run(cmd, capture_output=True, text=True, cwd=HERE)
    if r.returncode != 0:
        sys.exit("%s failed:\n%s" % (" ".join(cmd), r.stderr[-2000:]))
    return r.stdout


def instr_retired(cmd):
    """Instructions retired by one run of cmd (`/usr/bin/time -l`, macOS)."""
    r = subprocess.run(["/usr/bin/time", "-l"] + cmd, capture_output=True, text=True, cwd=HERE)
    if r.returncode != 0:
        sys.exit("%s failed:\n%s" % (cmd[0], r.stderr[-2000:]))
    for l in r.stderr.splitlines():
        if "instructions retired" in l:
            return int(l.split()[0])
    sys.exit("no instruction count from /usr/bin/time -l")


def cmd_perf(args):
    exes = build(args, hand=True, inline=args.inline_variant)
    sizes = [int(x) for x in args.sizes.split(",")]
    sets = args.sets.split(",")
    # side: (label, command prefix, layout or None for hand)
    sides = [(L, [exes["drv"], "perf"], L) for L in LAYS] + [("hand", [exes["hand"]], None)]
    if args.inline_variant:
        sides += [(L + "+inl", [exes["inl"], "perf"], L) for L in LAYS]

    def cmd_for(side, s, N, reps, row="all", H="all"):
        label, base, L = side
        if L is None:
            return base + [s, str(N), str(reps), row]
        return base + [s, str(N), str(reps), row, L, H]

    res, cks, words, runs = {}, {}, {}, []
    t0 = time.time()
    for rd in range(args.rounds):
        k = rd % len(sides)
        order = sides[k:] + sides[:k]
        for s in sets:
            for N in sizes:
                for side in order:
                    out = run_proc(cmd_for(side, s, N, args.reps))
                    runs.append({"round": rd, "set": s, "N": N, "side": side[0], "stdout": out})
                    for line in out.splitlines():
                        f = line.split()
                        if len(f) != 9:
                            continue
                        st, _L, H, row, n, ns, ck, wpk, bpk = f
                        key = (st, side[0], H, row, int(n))
                        res.setdefault(key, []).append(float(ns))
                        cks.setdefault(key, set()).add(int(ck))
                        words[key] = (float(wpk), float(bpk))
        print("round %d done (%.0fs)" % (rd + 1, time.time() - t0), file=sys.stderr)
    ins = {}
    if not args.no_instr:
        r1 = args.instr_reps
        r2 = 5 * r1
        for s in sets:
            for N in sizes:
                for side in sides:
                    for row in ("ret", "raw"):
                        for H in (["sip"] if side[2] is None else ["u", "w"]):
                            vals = []
                            for _ in range(args.instr_samples):
                                lo = instr_retired(cmd_for(side, s, N, r1, row, H))
                                hi = instr_retired(cmd_for(side, s, N, r2, row, H))
                                vals.append((hi - lo) / (r2 - r1))
                            ins[(s, side[0], H, row, N)] = statistics.median(vals)
    # checksum agreement: every side must fold the same results for ret/miss/raw (ids) and cmp
    cs_groups = {}
    for (s, side, H, row, N), v in cks.items():
        if row in ("ret", "miss", "raw", "cmp"):
            cs_groups.setdefault((s, N, row if row != "raw" else "ret"), set()).update(v)
    cs_bad = sorted("%s/%d/%s" % g for g, v in cs_groups.items() if len(v) != 1)
    summary = []
    print("set | N | L | H | row | median ns | min | max | instr/op | words/key | bytes/key | /hand | /no-inl")
    for key in sorted(res, key=lambda k: (k[0], k[4], k[3], k[1], k[2])):
        s, L, H, row, N = key
        v = res[key]
        med = statistics.median(v)
        hk = (s, "hand", "sip", row, N)
        vs_hand = med / statistics.median(res[hk]) if hk in res and L != "hand" else None
        base = (s, L[:-4], H, row, N) if L.endswith("+inl") else None
        vs_base = med / statistics.median(res[base]) if base and base in res else None
        i = ins.get(key)
        wpk, bpk = words[key]
        summary.append({"set": s, "L": L, "H": H, "row": row, "N": N, "median": med, "min": min(v), "max": max(v),
                        "samples": v, "instr": i, "words_per_key": wpk, "bytes_per_key": bpk, "vs_hand": vs_hand,
                        "vs_always_inline": vs_base, "checksums": sorted(cks[key])})
        print("%s | %d | %s | %s | %s | %.2f | %.2f | %.2f | %s | %.2f | %.2f | %s | %s" % (
            s, N, L, H, row, med, min(v), max(v), "-" if i is None else "%.0f" % i, wpk, bpk,
            "-" if vs_hand is None else "%.2f" % vs_hand, "-" if vs_base is None else "%.2f" % vs_base))
    print("checksum groups disagreeing: %s" % (", ".join(cs_bad) if cs_bad else "none"))
    os.makedirs(WORK, exist_ok=True)
    meta = {"rounds": args.rounds, "sizes": sizes, "sets": sets, "reps": args.reps,
            "instr_reps": None if args.no_instr else args.instr_reps, "instr_samples": args.instr_samples,
            "inline_variant": args.inline_variant, "inline_always_replaced": exes.get("inline_always_replaced"),
            "pruvc": args.pruvc, "time": time.strftime("%Y-%m-%dT%H:%M:%S"), "checksum_disagreements": cs_bad}
    out = {"meta": meta, "summary": summary, "runs": runs}
    stamp = time.strftime("%Y%m%d-%H%M%S")
    for name in ("perf_%s.json" % stamp, "perf_latest.json"):
        with open(os.path.join(WORK, name), "w") as f:
            json.dump(out, f, indent=1, ensure_ascii=False)
    print("raw JSON: %s" % os.path.join(WORK, "perf_latest.json"))
    return 1 if cs_bad else 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["check", "perf"])
    ap.add_argument("--pruvc", default=PRUVC_DEFAULT)
    ap.add_argument("--count", type=int, default=300)
    ap.add_argument("--seed", type=int, default=20261117)
    ap.add_argument("--rounds", type=int, default=7)
    ap.add_argument("--sizes", default="100,500")
    ap.add_argument("--sets", default=",".join(SETS))
    ap.add_argument("--reps", type=int, default=2000000)
    ap.add_argument("--no-instr", action="store_true")
    ap.add_argument("--instr-reps", type=int, default=1000000)
    ap.add_argument("--instr-samples", type=int, default=1)
    ap.add_argument("--inline-variant", action="store_true")
    a = ap.parse_args()
    sys.exit({"check": cmd_check, "perf": cmd_perf}[a.cmd](a))


if __name__ == "__main__":
    main()
