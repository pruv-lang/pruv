#!/usr/bin/env python3
"""Grok history core perf harness: the generated Pruv read::window (pruv) against hand-written safe Rust (hand).

  python3 bench/l3/grok/perf/run_perf.py [check|all] [--rounds 5] [--time-ms 400] [--rows hist-ord-64k,...] [--pruvc PATH]

Build (everything under perf/work/, scratch, never committed): the grok modules read.pruv reaches (util, packet, path, framing,
entry, read) are copied verbatim next to perf_root.pruv together with a manifest, pruvc compiles it (verified mode, str adapter
rlib), and gen.rs + hand_history.rs + perf_driver.rs become one main.rs built with rustc -C opt-level=3 --edition 2021.

Rows (generated deterministically here, written to work/rows/*.bin only so the driver can read them; nothing is committed):
  hist-ord-64k (S sanity), hist-ord-2m (M), hist-ord-8m (L)   ordinary grok session lines, LF-terminated
  hist-dense-2m (M), hist-dense-8m (L)                         dense tiny lines ("0", "{}", a few small user/assistant objects)
check: the driver runs both rows on every row (omitted=false and true) and compares the packets word for word; a mismatch exits 1.
perf: a fresh process per (row, cell, round), implementation order rotated every round, warm-up call, per process median of the
timed calls; the cell value is the median of the round medians. RSS: /usr/bin/time -l whole-process peak (median of 3) of one call
with the window read first (so RSS includes the input window bytes for both sides).
Writes work/perf_result.json and work/perf_cells.json (schema of tools/perf_gate.py).
"""
import argparse, json, os, random, shutil, statistics, subprocess, sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
GROK = os.path.dirname(HERE)
L3 = os.path.dirname(GROK)
ROOT = os.path.abspath(os.path.join(L3, "..", ".."))
WORK = os.path.join(HERE, "work")
ROWS_DIR = os.path.join(WORK, "rows")
CARGO_BIN = os.path.expanduser("~/.cargo/bin")
ENV = dict(os.environ, PATH=CARGO_BIN + os.pathsep + os.environ.get("PATH", ""))
HEADER = ('#![allow(dead_code, unused_imports, unused_variables, unused_mut, non_snake_case, unused_parens, '
          'unused_assignments, non_camel_case_types, private_interfaces)]\n')
RUSTC_FLAGS = ["-C", "opt-level=3", "--edition", "2021"]
GROK_MODULES = ["util", "packet", "path", "framing", "entry", "read"]
DEPS = ["json", "parse", "access", "jstr", "lines", "bytes", "text", "time", "names", "turns"]
IMPLS = ["pruv", "hand"]
# (name, size class, window bytes, shape)
ROWS = [("hist-ord-64k", "S", 64 * 1024, "ord"), ("hist-ord-2m", "M", 2 << 20, "ord"), ("hist-ord-8m", "L", 8 << 20, "ord"),
        ("hist-dense-2m", "M", 2 << 20, "dense"), ("hist-dense-8m", "L", 8 << 20, "dense")]


def rustc():
    return shutil.which("rustc", path=ENV["PATH"]) or os.path.join(CARGO_BIN, "rustc")


def sh(cmd, cwd=None, what=None):
    r = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, env=ENV)
    if r.returncode != 0:
        sys.exit("%s failed:\n%s%s" % (what or cmd[0], r.stdout[-4000:], r.stderr[-6000:]))
    return r


def find_pruvc(arg):
    cands = [arg, os.environ.get("PRUVC"), os.path.join(ROOT, "pruvc-rs", "target", "release", "pruvc")]
    for c in cands:
        if c and os.path.isfile(c):
            return c
    sys.exit("pruvc not found (tried --pruvc, $PRUVC, %s); build pruvc-rs or pass --pruvc PATH" % cands[-1])


# ---------------------------------------------------------------- build
def build(pruvc):
    os.makedirs(WORK, exist_ok=True)
    shutil.copy(os.path.join(HERE, "perf_root.pruv"), os.path.join(WORK, "perf_root.pruv"))
    for m in GROK_MODULES:
        shutil.copy(os.path.join(GROK, m + ".pruv"), os.path.join(WORK, m + ".pruv"))  # copies, the originals stay untouched
    man = {"name": "l3_grok_perf", "version": "0.1.0", "root": "perf_root.pruv",
           "dependencies": {k: {"path": "../../../pkgs/" + k, "version": "0.1"} for k in DEPS}}
    json.dump(man, open(os.path.join(WORK, "perf_root.pruv.json"), "w"))
    rlib = os.path.join(WORK, "libpruv_str.rlib")
    sh([rustc(), "--crate-name", "pruv_str", "--crate-type", "rlib", os.path.join(GROK, "str_adapter.rs"), "-o", rlib],
       what="rustc str_adapter")
    gen = os.path.join(WORK, "gen.rs")
    if os.path.exists(gen):
        os.remove(gen)  # a stale gen.rs must never pass for a fresh compile
    r = subprocess.run([pruvc, "perf_root.pruv", "-o", "gen.rs", "--manifest", "perf_root.pruv.json",
                        "--rust-extern", "pruv_str=libpruv_str.rlib"], cwd=WORK, capture_output=True, text=True, env=ENV)
    if r.returncode != 0 or not os.path.exists(gen):
        sys.exit("pruvc failed:\n" + r.stdout[-4000:] + r.stderr[-4000:])
    shutil.copy(os.path.join(HERE, "hand_history.rs"), os.path.join(WORK, "hand_history.rs"))
    drv = open(os.path.join(HERE, "perf_driver.rs"), encoding="utf-8").read()
    main = os.path.join(WORK, "main.rs")
    with open(main, "w", encoding="utf-8") as f:
        f.write(HEADER + 'include!("gen.rs");\nmod hand { include!("hand_history.rs"); }\n' + drv)
    exe = os.path.join(WORK, "perf_driver")
    sh([rustc()] + RUSTC_FLAGS + ["-o", exe, main, "--extern", "pruv_str=" + rlib], what="rustc driver")
    return exe


# ---------------------------------------------------------------- rows
WORDS = ("alpha beta gamma delta retry sync flaky config parser stream buffer window cursor ring slot packet merge branch commit "
         "build test fix lint cache index tokens naïve café 日本語 한국어 résumé 🙂 error timeout queue worker handler").split()


def ts(i):
    return "2025-03-%02dT%02d:%02d:%02d.%03dZ" % (1 + (i // 86400) % 28, (i // 3600) % 24, (i // 60) % 60, i % 60, (i * 7) % 1000)


def text(rng, n):
    out, size = [], 0
    while size < n:
        w = rng.choice(WORDS)
        if rng.random() < 0.04:
            w = '"%s"' % w
        out.append(w)
        size += len(w.encode("utf-8")) + 1
        if rng.random() < 0.02:
            out.append("\n")
    return " ".join(out).strip() or "x"


def dump(o):
    return json.dumps(o, ensure_ascii=False, separators=(",", ":")).encode("utf-8") + b"\n"


def gen_ord(size, seed):
    rng = random.Random(seed)
    out, total, i, k = [], 0, 0, 0
    while True:
        batch = []
        t = text(rng, rng.randint(200, 4000))
        v = rng.random()
        if v < 0.6:
            u = {"type": "user", "content": t, "timestamp": ts(i), "prompt_index": k}
        elif v < 0.8:
            u = {"type": "user", "content": "<user_query>\n" + t + "\n</user_query>", "timestamp": ts(i)}
        else:
            u = {"type": "user", "content": [{"type": "text", "text": t}, {"type": "text", "text": text(rng, rng.randint(200, 1500))}],
                 "created_at": ts(i)}
        batch.append(dump(u))
        k += 1
        for _ in range(rng.choice([1, 2, 3, 5, 8, 20])):
            i += 1
            a = {"type": "assistant", "content": text(rng, rng.randint(200, 4000))}
            r = rng.random()
            if r < 0.85:
                a["timestamp"] = ts(i)
            elif r < 0.95:
                a["timestamp"] = None
                a["created_at"] = ts(i)
            if rng.random() < 0.4:
                a["tool_calls"] = [{"id": "call_%d_%d" % (i, j), "type": "function",
                                    "function": {"name": "read_file", "arguments": "{\"path\":\"src/a%d.rs\"}" % j}}
                                   for j in range(rng.randint(1, 3))]
            batch.append(dump(a))
            if rng.random() < 0.15:
                batch.append(dump({"type": "tool", "content": text(rng, rng.randint(200, 1000)), "timestamp": ts(i),
                                   "meta": {"a": [1, 2.5, None, True], "b": "x"}}))
        i += 1
        n = sum(len(b) for b in batch)
        if total + n > size:
            for b in batch:  # fill the tail line by line
                if total + len(b) > size:
                    return b"".join(out)
                out.append(b)
                total += len(b)
            return b"".join(out)
        out.extend(batch)
        total += n


def gen_dense(size, seed):
    out, total, i = [], 0, 0
    while True:
        c = i % 400
        if c == 0:
            b = dump({"type": "user", "content": "q %d" % i, "timestamp": ts(i)})
        elif c == 1:
            b = dump({"type": "assistant", "content": "a %d" % i, "timestamp": ts(i), "tool_calls": [{}, {}]})
        else:
            b = b"0\n" if c % 2 else b"{}\n"
        if total + len(b) > size:
            return b"".join(out)
        out.append(b)
        total += len(b)
        i += 1


def make_rows(only):
    os.makedirs(ROWS_DIR, exist_ok=True)
    rows = []
    for name, size_class, size, shape in ROWS:
        if only and name not in only:
            continue
        data = (gen_ord if shape == "ord" else gen_dense)(size, 20240601 + len(name) + size)
        assert len(data) <= size and data.endswith(b"\n")
        path = os.path.join(ROWS_DIR, name + ".bin")
        with open(path, "wb") as f:
            f.write(data)
        rows.append((name, size_class, path, len(data)))
    if not rows:
        sys.exit("no rows selected")
    return rows


# ---------------------------------------------------------------- run
def max_rss(cmd):
    """Peak resident set size in bytes of one process (/usr/bin/time -l, macOS)."""
    r = subprocess.run(["/usr/bin/time", "-l"] + cmd, capture_output=True, text=True, env=ENV)
    if r.returncode != 0:
        sys.exit("%s failed:\n%s" % (cmd[0], r.stderr[-2000:]))
    for l in r.stderr.splitlines():
        if "maximum resident set size" in l:
            return int(l.split()[0])
    sys.exit("no 'maximum resident set size' in /usr/bin/time -l output")


def run_check(exe, rows):
    r = subprocess.run([exe, "check"] + [p for _, _, p, _ in rows], capture_output=True, text=True, env=ENV)
    if r.stderr.strip():
        sys.stderr.write(r.stderr[-6000:])
    if r.returncode != 0:
        print("check: MISMATCH between pruv and hand (see above)")
        return 1
    for ln in r.stdout.splitlines():
        t = ln.split()
        print("check %-14s packet words %-9s fold %s  pruv == hand" % (os.path.basename(t[0])[:-4], t[1], t[2]))
    return 0


def run_perf(exe, rows, args):
    res = {}
    for rd in range(args.rounds):
        for name, _, path, _ in rows:
            order = IMPLS[rd % 2:] + IMPLS[:rd % 2]  # rotate the order every round
            for impl in order:
                r = subprocess.run([exe, "time", impl, path, str(args.time_ms)], capture_output=True, text=True, env=ENV)
                if r.returncode != 0:
                    sys.exit("time %s %s failed:\n%s" % (impl, name, r.stderr[-2000:]))
                med, mn, mx, n, ck = r.stdout.split()
                e = res.setdefault((name, impl), {"med": [], "min": [], "max": [], "calls": 0, "ck": set()})
                e["med"].append(int(med)); e["min"].append(int(mn)); e["max"].append(int(mx))
                e["calls"] += int(n); e["ck"].add(int(ck))
        print("round %d/%d done" % (rd + 1, args.rounds), file=sys.stderr, flush=True)
    out = []
    bad = 0
    for name, size_class, path, nbytes in rows:
        cks = res[(name, "pruv")]["ck"] | res[(name, "hand")]["ck"]
        if len(cks) != 1:
            print("ERROR: checksums disagree on %s: %s" % (name, sorted(cks)), file=sys.stderr)
            bad += 1
        for impl in IMPLS:
            e = res[(name, impl)]
            rss = statistics.median([max_rss([exe, "rss", impl, path]) for _ in range(3)])
            out.append({"row": name, "size": size_class, "impl": impl, "window_bytes": nbytes, "median_ns": statistics.median(e["med"]),
                        "min_ns": min(e["min"]), "max_ns": max(e["max"]), "rounds_median_ns": e["med"], "calls": e["calls"],
                        "max_rss_bytes": rss})
    return out, bad


def report(out, rows, args):
    by = {(o["row"], o["impl"]): o for o in out}
    print("\n| row | size | impl | median ns | min | max | max RSS | vs hand (ns) | vs hand (RSS) |")
    print("|---|---|---|---:|---:|---:|---:|---:|---:|")
    cells = []
    for name, size_class, _, _ in rows:
        h = by[(name, "hand")]
        for impl in IMPLS:
            o = by[(name, impl)]
            print("| %s | %s | %s | %d | %d | %d | %d | %.2fx | %.2fx |" % (
                name, size_class, impl, o["median_ns"], o["min_ns"], o["max_ns"], o["max_rss_bytes"],
                o["median_ns"] / max(h["median_ns"], 1), o["max_rss_bytes"] / max(h["max_rss_bytes"], 1)))
        p = by[(name, "pruv")]
        cells.append({"name": "grok history %s time" % name, "axis": "time", "size": size_class, "kind": "proportional",
                      "pruv": p["median_ns"], "hand": h["median_ns"], "unit": "ns", "runs": args.rounds})
        cells.append({"name": "grok history %s rss" % name, "axis": "rss", "size": size_class, "kind": "proportional",
                      "pruv": p["max_rss_bytes"], "hand": h["max_rss_bytes"], "unit": "bytes", "runs": 3})
    json.dump({"rounds": args.rounds, "time_ms": args.time_ms, "results": out}, open(os.path.join(WORK, "perf_result.json"), "w"), indent=1)
    json.dump({"source": "bench/l3/grok/perf/run_perf.py", "cells": cells}, open(os.path.join(WORK, "perf_cells.json"), "w"), indent=1)
    print("\nJSON: " + os.path.join(WORK, "perf_result.json"))
    print("perf_cells: " + os.path.join(WORK, "perf_cells.json"))
    for c in cells:
        print("%-36s %-4s %-4s pruv %14.0f hand %14.0f %-5s %.3fx" % (
            c["name"], c["axis"], c["size"], c["pruv"], c["hand"], c["unit"], c["pruv"] / max(c["hand"], 1)))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("mode", nargs="?", default="all", choices=["check", "all"])
    ap.add_argument("--rounds", type=int, default=5)
    ap.add_argument("--time-ms", type=int, default=400)
    ap.add_argument("--rows", default="", help="comma separated row names (default: all)")
    ap.add_argument("--pruvc", default=None)
    args = ap.parse_args()
    only = [r for r in args.rows.split(",") if r]
    rows = make_rows(only)
    exe = build(find_pruvc(args.pruvc))
    bad = run_check(exe, rows)
    if args.mode == "all" and bad == 0:
        out, b = run_perf(exe, rows, args)
        bad += b
        report(out, rows, args)
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
