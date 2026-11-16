#!/usr/bin/env python3
"""G4b-Q: cold vs warm compile time and hit rate of --proof-cache on the four L3 pieces.

Run from anywhere:  python3 bench/g4b/measure_cache.py [--pieces json,mcp,qwen,supervisor]
                    [--compilers rs,py] [--work DIR] [--out FILE]

Prerequisites (this script builds nothing):
  - pruvc-rs/target/release/pruvc must exist (Rust compiler, release build).
  - mcp: run bench/l3/mcp/test_impl.py once first; it builds bench/l3/mcp/work/host/debug/deps/libmcp_host-*.rlib
    and bench/l3/mcp/work/libpruv_str.rlib.
  - qwen: run bench/l3/qwen/test_impl.py once first; it builds bench/l3/qwen/work/libpruv_str.rlib.
  - json and supervisor need nothing extra.

Per piece x compiler, sequentially: off (no cache), cold A (fresh DIR_A), warm A (same DIR_A), cold B (fresh DIR_B).
Every run passes --trust-report. Checks: generated .rs / stdout / stderr byte-identical across the four runs;
key set of DIR_A == key set of DIR_B (determinism across runs); warm stores == 0, warm hits == cold hits + cold
stores and warm misses == cold misses - cold stores (a cold run already hits on repeated queries within the run, and
only sat/unknown answers stay misses);
py and rs key sets are disjoint (informational, expected). Exit code is non-zero if any check or compile fails.
"""
import argparse
import glob
import json
import os
import shutil
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
COMPILERS = {
    "py": [sys.executable, "pruvc/pruvc.py"],
    "rs": ["pruvc-rs/target/release/pruvc"],
}


def need(path):
    if not os.path.isfile(os.path.join(ROOT, path)):
        sys.exit("missing prerequisite: %s (run the piece's own test driver once first)" % path)
    return path


def piece_args(piece):
    """Returns (source, extra flags) with paths relative to the repo root."""
    if piece == "json":
        return "bench/l3/json/a5_root.pruv", ["--manifest", "bench/l3/json/a5_root.pruv.json"]
    if piece == "mcp":
        libs = sorted(glob.glob(os.path.join(ROOT, "bench/l3/mcp/work/host/debug/deps/libmcp_host-*.rlib")),
                      key=os.path.getmtime)
        if not libs:
            sys.exit("missing prerequisite: bench/l3/mcp/work/host/debug/deps/libmcp_host-*.rlib "
                     "(run bench/l3/mcp/test_impl.py once first)")
        need("bench/l3/mcp/work/libpruv_str.rlib")
        return "bench/l3/mcp/root.pruv", [
            "--manifest", "bench/l3/mcp/pruv.json",
            "--rust-extern", "mcp_host=" + os.path.relpath(libs[-1], ROOT),
            "--rust-extern", "pruv_str=bench/l3/mcp/work/libpruv_str.rlib"]
    if piece == "qwen":
        need("bench/l3/qwen/work/libpruv_str.rlib")
        return "bench/l3/qwen/root.pruv", [
            "--manifest", "bench/l3/qwen/root.pruv.json",
            "--rust-extern", "pruv_str=bench/l3/qwen/work/libpruv_str.rlib",
            "--rust-search", "bench/l3/qwen/work"]
    if piece == "supervisor":
        return "bench/l3/supervisor/supervise.pruv", ["--manifest", "bench/l3/supervisor/supervise.pruv.json"]
    sys.exit("unknown piece: " + piece)


def key_set(d):
    """Cache keys as 'xx/key' under d, ignoring temporary files."""
    keys = set()
    if not os.path.isdir(d):
        return keys
    for sub in os.listdir(d):
        p = os.path.join(d, sub)
        if os.path.isdir(p):
            for name in os.listdir(p):
                if not name.startswith(".tmp."):
                    keys.add(sub + "/" + name)
    return keys


def read_bytes(path):
    try:
        with open(path, "rb") as f:
            return f.read()
    except OSError:
        return None


def run_once(comp, src, flags, rundir, cache):
    os.makedirs(rundir, exist_ok=True)
    rs = os.path.join(rundir, "out.rs")
    report = os.path.join(rundir, "trust.json")
    cmd = COMPILERS[comp] + [src, "-o", rs] + flags + ["--trust-report", report]
    if cache:
        cmd += ["--proof-cache", cache]
    t0 = time.perf_counter()
    p = subprocess.run(cmd, cwd=ROOT, capture_output=True, timeout=3600)
    dt = time.perf_counter() - t0
    pc = {"hits": 0, "misses": 0, "stores": 0}
    try:
        with open(report, encoding="utf-8") as f:
            pc = json.load(f)["proof_cache"]
    except (OSError, ValueError, KeyError):
        pass
    return {"rc": p.returncode, "secs": dt, "stdout": p.stdout, "stderr": p.stderr,
            "rs": read_bytes(rs), "pc": pc, "cmd": cmd}


def pct(a, b):
    return 100.0 * a / b if b else 0.0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pieces", default="json,mcp,qwen,supervisor")
    ap.add_argument("--compilers", default="rs,py")
    ap.add_argument("--work", default="bench/g4b/.work/measure")
    ap.add_argument("--out", default=None)
    a = ap.parse_args()
    work = a.work if os.path.isabs(a.work) else os.path.join(ROOT, a.work)
    out = a.out or os.path.join(work, "measure.json")
    pieces = [x for x in a.pieces.split(",") if x]
    comps = [x for x in a.compilers.split(",") if x]
    for c in comps:
        if c not in COMPILERS:
            sys.exit("unknown compiler: " + c)
    if "rs" in comps:
        need(COMPILERS["rs"][0])
    if os.path.isdir(work):
        shutil.rmtree(work)
    os.makedirs(work)

    results, failures = [], []
    keysets = {}  # (piece, comp) -> set of keys from DIR_A
    for piece in pieces:
        src, flags = piece_args(piece)
        for comp in comps:
            base = os.path.join(work, piece, comp)
            dir_a, dir_b = os.path.join(base, "cacheA"), os.path.join(base, "cacheB")
            print("measuring %s / %s ..." % (piece, comp), file=sys.stderr, flush=True)
            off = run_once(comp, src, flags, os.path.join(base, "off"), None)
            cold = run_once(comp, src, flags, os.path.join(base, "coldA"), dir_a)
            keys_a = key_set(dir_a)
            warm = run_once(comp, src, flags, os.path.join(base, "warmA"), dir_a)
            cold_b = run_once(comp, src, flags, os.path.join(base, "coldB"), dir_b)
            keys_b = key_set(dir_b)
            runs = {"off": off, "cold": cold, "warm": warm, "coldB": cold_b}
            name = piece + "/" + comp
            row_fail = []
            for k, r in runs.items():
                if r["rc"] != 0:
                    row_fail.append("%s: compile failed in %s run (rc=%s)" % (name, k, r["rc"]))
            outputs_same = all(
                r[f] is not None and r[f] == off[f] for r in runs.values() for f in ("rs", "stdout", "stderr"))
            same_keys = keys_a == keys_b
            cp, wp = cold["pc"], warm["pc"]
            warm_ok = (wp["stores"] == 0 and wp["hits"] == cp["hits"] + cp["stores"]
                       and wp["misses"] == cp["misses"] - cp["stores"])
            if not outputs_same:
                row_fail.append("%s: outputs differ across runs" % name)
            if not same_keys:
                row_fail.append("%s: key sets differ between cold A and cold B" % name)
            if not warm_ok:
                row_fail.append("%s: warm counts do not match the cold run (stores 0, hits = cold hits + stores)" % name)
            keysets[(piece, comp)] = keys_a
            wh, wm = warm["pc"]["hits"], warm["pc"]["misses"]
            results.append({
                "piece": piece, "compiler": comp,
                "off_s": off["secs"], "cold_s": cold["secs"], "warm_s": warm["secs"], "coldB_s": cold_b["secs"],
                "cold_overhead_pct": pct(cold["secs"] - off["secs"], off["secs"]),
                "warm_saving_pct": pct(off["secs"] - warm["secs"], off["secs"]),
                "cold": cold["pc"], "warm": warm["pc"], "coldB": cold_b["pc"],
                "warm_hit_rate_pct": pct(wh, wh + wm),
                "keys_A": len(keys_a), "keys_B": len(keys_b),
                "same_keys_across_runs": same_keys, "outputs_identical": outputs_same,
                "warm_counts_ok": warm_ok, "failures": row_fail,
            })
            failures += row_fail

    disjoint = {}
    if "py" in comps and "rs" in comps:
        for piece in pieces:
            disjoint[piece] = not (keysets[(piece, "py")] & keysets[(piece, "rs")])
            # Disjointness is expected (compiler id is in the key); report only, do not fail.

    print("| piece | compiler | off | cold | warm | cold overhead % | warm saving % | cold h/m/s | warm h/m "
          "| warm hit rate | same keys across runs | outputs identical |")
    print("|---|---|---|---|---|---|---|---|---|---|---|---|")
    for r in results:
        c, w = r["cold"], r["warm"]
        print("| %s | %s | %.2fs | %.2fs | %.2fs | %.1f | %.1f | %d/%d/%d | %d/%d | %.1f%% | %s | %s |" % (
            r["piece"], r["compiler"], r["off_s"], r["cold_s"], r["warm_s"], r["cold_overhead_pct"],
            r["warm_saving_pct"], c["hits"], c["misses"], c["stores"], w["hits"], w["misses"],
            r["warm_hit_rate_pct"], "yes" if r["same_keys_across_runs"] else "NO",
            "yes" if r["outputs_identical"] else "NO"))
    if disjoint:
        print("\npy vs rs key sets disjoint (expected yes): " +
              ", ".join("%s=%s" % (k, "yes" if v else "NO") for k, v in disjoint.items()))
    for f in failures:
        print("FAIL: " + f, file=sys.stderr)

    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    with open(out, "w", encoding="utf-8") as f:
        json.dump({"results": results, "py_rs_keys_disjoint": disjoint, "failures": failures}, f, indent=2)
    print("wrote " + out, file=sys.stderr)
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
