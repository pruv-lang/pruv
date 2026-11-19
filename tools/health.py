#!/usr/bin/env python3
"""Health check numbers for docs/notes/health_log.md (TIDY_SPEC T6).

  python3 tools/health.py                 one markdown row (fast: line counts, PERF_HEADROOM, CI, work/, disk)
  python3 tools/health.py --full          + budget m4: pruvc proof time/peak memory, rustc time/peak memory, binary
                                            size, and M4 generated program vs hand Rust (time, peak RSS, allocations)
  python3 tools/health.py --compare       print only the lines past their thresholds against the last saved state
  python3 tools/health.py --save          append this run to the state file (work/health_state.json, main checkout)
  python3 tools/health.py --header        print the table header first
  python3 tools/health.py --branchy       print the 15 functions with the most branches (both compilers)

Thresholds (the light check run at every merge):
  - compiler lines: more than +1,000 lines in pruvc (Python) or pruvc-rs (Rust) since the last state;
  - m4 proof time, rustc time, binary size, compiler peak memory, rustc peak memory: more than 20% over the last state;
  - generated program peak RSS or allocation count (M4 vs hand Rust, per axis): more than 20% over the last state;
  - every PERF_HEADROOM summary item above 1.0x hand Rust is listed (there is no tolerance band; only repeated
    interleaved runs with overlapping distributions make it noise, and that is judged by a person, not here);
  - any of the last 3 CI runs on main that failed; work/ changed after the last backup; disk free below 20 GB.

The compiler (pruvc-rs) links z3 in-process, so the pruvc peak RSS is the compiler and z3 together.
Allocation counts come from the counting global allocator already in driver_m4.rs and m4_hand.rs (allocs=N on
their perf line); the m4 build reuses bench/l3/budget/m34_test/run.py build() into a temp directory.
Exit code: 0, or 1 when --compare found a breach. Standard library only.
"""
import argparse
import ast
import datetime
import json
import os
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PERF_HEADROOM = os.path.join(ROOT, "bench", "l3", "budget", "PERF_HEADROOM.md")
M34 = os.path.join(ROOT, "bench", "l3", "budget", "m34_test")
HAND_DIR = os.path.join(ROOT, "bench", "l3", "budget", "perf_headroom", "host")
COMPILERS = (("pruvc", "pruvc/*.py"), ("pruvc-rs", "pruvc-rs/*.rs"))

LINES_JUMP = 1000          # lines added to one compiler since the last state
GROWTH = 0.20              # time / size / memory growth over the last state
DISK_MIN = 20 * 10 ** 9    # bytes free on /
AXES = ("sessions", "tools", "clocks")
M4_N = 1000
M4_REPS = 200000


# ---- small helpers ----------------------------------------------------------------------------------------------

def sh(cmd, cwd=ROOT, timeout=120):
    """stdout of cmd, or None when it cannot run or fails."""
    try:
        c = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return c.stdout if c.returncode == 0 else None


def main_checkout():
    """The main checkout (owner of work/ and docs/notes/), also when run from a linked worktree."""
    common = sh(["git", "rev-parse", "--path-format=absolute", "--git-common-dir"])
    return os.path.dirname(common.strip()) if common else ROOT


def fmt_int(v):
    return "{:,}".format(v)


def fmt_delta(v):
    return "+" + fmt_int(v) if v > 0 else ("0" if v == 0 else "-" + fmt_int(-v))


def human(nbytes):
    for unit, size in (("G", 1 << 30), ("M", 1 << 20), ("K", 1 << 10)):
        if nbytes >= size:
            return "%.1f%s" % (nbytes / size, unit) if nbytes < 10 * size else "%d%s" % (nbytes // size, unit)
    return "%dB" % nbytes


# ---- fast measurements ------------------------------------------------------------------------------------------

def line_counts():
    """{compiler: total lines} and the per-file counts of tracked sources (same as `git ls-files PAT | xargs wc -l`)."""
    totals, files = {}, {}
    for name, pat in COMPILERS:
        out = sh(["git", "ls-files", "-z", pat]) or ""
        total = 0
        for rel in filter(None, out.split("\0")):
            try:
                with open(os.path.join(ROOT, rel), "rb") as fh:
                    n = fh.read().count(b"\n")
            except OSError:
                continue
            files[rel] = n
            total += n
        totals[name] = total
    return totals, files


def top_files(files, k=5):
    return sorted(files.items(), key=lambda kv: (-kv[1], kv[0]))[:k]


RATIO = re.compile(r"\d+\.\d+")


def parse_headroom(text):
    """Items of the PERF_HEADROOM summary table whose Pruv/hand cell has a ratio above 1.0.

    The cell is free text; ratios are read from the part before the first '(' (the parenthesis holds history such as
    'was 1.40'). Returns [(item, max ratio, status)] in table order."""
    lines = text.splitlines()
    try:
        start = next(i for i, l in enumerate(lines) if l.startswith("## Summary"))
    except StopIteration:
        return []
    header = None
    out = []
    for l in lines[start + 1:]:
        if l.startswith("## "):
            break
        if not l.startswith("|"):
            continue
        cells = [c.strip() for c in l.strip().strip("|").split("|")]
        if header is None:
            header = cells
            continue
        if set(l.replace("|", "").strip()) <= set("-: "):
            continue
        row = dict(zip(header, cells))
        cell = row.get("Pruv/hand", "")
        ratios = [float(x) for x in RATIO.findall(cell.split("(")[0])]
        if ratios and max(ratios) > 1.0:
            out.append((row.get("item", cells[0]), max(ratios), row.get("status", "")))
    return out


def ci_runs():
    """Last 3 CI runs on main: [(id, conclusion or status, short sha)], or None when gh is unavailable."""
    out = sh(["gh", "run", "list", "--branch", "main", "--limit", "3", "--json",
              "databaseId,status,conclusion,headSha"], timeout=30)
    if out is None:
        return None
    try:
        runs = json.loads(out)
    except ValueError:
        return None
    return [(r["databaseId"], r.get("conclusion") or r.get("status") or "?", r.get("headSha", "")[:7]) for r in runs]


def du_bytes(path):
    out = sh(["du", "-sk", path], cwd=None, timeout=60)
    try:
        return int(out.split()[0]) * 1024
    except (AttributeError, IndexError, ValueError):
        return None


def newest_mtime(path):
    newest = 0.0
    for d, dirs, files in os.walk(path):
        for n in files + dirs:
            try:
                newest = max(newest, os.lstat(os.path.join(d, n)).st_mtime)
            except OSError:
                pass
    return newest


def work_state(main):
    """work/ size and backup state: the backup repo is ../pruv-internal next to the main checkout; work/ is backed up
    when its newest file is not newer than the last backup commit (backup.sh is never run here)."""
    work = os.path.join(main, "work")
    if not os.path.isdir(work):
        return {"work_bytes": None, "backed_up": None, "backup_time": None}
    backup = os.path.join(os.path.dirname(main), "pruv-internal")
    ct = sh(["git", "-C", backup, "log", "-1", "--format=%ct"], cwd=None)
    newest = newest_mtime(work)
    bt = int(ct.strip()) if ct and ct.strip().isdigit() else None
    return {"work_bytes": du_bytes(work), "backed_up": (None if bt is None else newest <= bt),
            "backup_time": bt, "work_newest": int(newest)}


def fast(args):
    totals, files = line_counts()
    main = main_checkout()
    with open(PERF_HEADROOM, encoding="utf-8") as fh:
        above = parse_headroom(fh.read())
    m = {
        "date": datetime.datetime.now().strftime("%Y-%m-%d %H:%M"),
        "commit": (sh(["git", "rev-parse", "--short", "HEAD"]) or "?").strip(),
        "lines": totals,
        "top_files": top_files(files),
        "above_hand": above,
        "ci": ci_runs(),
        "disk_free": shutil.disk_usage("/").free,
        "pruv_bytes": du_bytes(main),
    }
    m.update(work_state(main))
    return m


# ---- slow measurements (--full) ---------------------------------------------------------------------------------

def time_cmd():
    """/usr/bin/time with the flag that prints peak memory: macOS -l (bytes), GNU -v (KiB)."""
    if not os.path.exists("/usr/bin/time"):
        return None
    return ["/usr/bin/time", "-l"] if sys.platform == "darwin" else ["/usr/bin/time", "-v"]


def parse_time(stderr):
    """(wall seconds or None, peak RSS bytes or None) from /usr/bin/time -l (macOS) or -v (GNU) output."""
    wall = rss = None
    m = re.search(r"([\d.]+)\s+real", stderr)
    if m:
        wall = float(m.group(1))
    m = re.search(r"(\d+)\s+maximum resident set size", stderr)
    if m:
        rss = int(m.group(1))
    m = re.search(r"Elapsed \(wall clock\) time(?: \([^)]*\))?:\s*(?:(\d+):)?(\d+):([\d.]+)", stderr)
    if m:
        wall = int(m.group(1) or 0) * 3600 + int(m.group(2)) * 60 + float(m.group(3))
    m = re.search(r"Maximum resident set size \(kbytes\):\s*(\d+)", stderr)
    if m:
        rss = int(m.group(1)) * 1024
    return wall, rss


def kv(line):
    return dict(t.split("=", 1) for t in line.split() if "=" in t)


def default_pruvc(main):
    exe = "pruvc.exe" if os.name == "nt" else "pruvc"
    for base in (ROOT, main):
        p = os.path.join(base, "pruvc-rs", "target", "release", exe)
        if os.path.exists(p):
            return p
    return os.path.join(ROOT, "pruvc-rs", "target", "release", exe)


def hand_exe(main, tmp):
    """m4_hand from an existing work/handbuild (this tree, then the main checkout), else cargo build into tmp."""
    exe = "m4_hand.exe" if os.name == "nt" else "m4_hand"
    for base in (ROOT, main):
        p = os.path.join(base, "bench", "l3", "budget", "work", "handbuild", "release", exe)
        if os.path.exists(p):
            return p
    env = dict(os.environ, CARGO_TARGET_DIR=os.path.join(tmp, "hand"))
    env["PATH"] = os.path.expanduser("~/.cargo/bin") + os.pathsep + env.get("PATH", "")
    c = subprocess.run(["cargo", "build", "--release", "--offline", "--quiet", "--bin", "m4_hand"], cwd=HAND_DIR,
                       env=env, capture_output=True, text=True)
    if c.returncode != 0:
        raise RuntimeError("cargo build m4_hand failed:\n" + c.stderr[-1500:])
    return os.path.join(tmp, "hand", "release", exe)


def full(args, main):
    """budget m4: build through m34_test/run.py build() into a temp dir with pruvc and rustc under /usr/bin/time,
    then M4 driver vs m4_hand perf runs (alternating, args.rounds rounds, n = 1000)."""
    tc = time_cmd()
    if tc is None:
        raise RuntimeError("/usr/bin/time not found (needed for peak memory)")
    sys.path.insert(0, M34)
    import run as m34  # noqa: E402  (bench/l3/budget/m34_test/run.py)
    tmp = tempfile.mkdtemp(prefix="pruv_health_")
    rec = {}
    orig_run = m34.run

    def timed_run(cmd, cwd=None, inp=None, timeout=600):
        tool = os.path.basename(cmd[0])
        key = "pruvc" if any(os.path.basename(a) in ("pruvc", "pruvc.exe", "pruvc.py") for a in cmd[:2]) \
            else ("rustc" if tool.startswith("rustc") else None)
        if key is None:
            return orig_run(cmd, cwd, inp, timeout)
        c = orig_run(tc + cmd, cwd, inp, timeout)
        rec[key] = parse_time(c.stderr or "")
        return c

    m34.WORK = tmp
    m34.run = timed_run
    try:
        exe = m34.build(args.pruvc, "m4")
        hand = hand_exe(main, tmp)
        res = {"pruvc_s": rec["pruvc"][0], "pruvc_rss": rec["pruvc"][1],
               "rustc_s": rec["rustc"][0], "rustc_rss": rec["rustc"][1],
               "binary_bytes": os.path.getsize(exe), "pruvc_path": args.pruvc, "m4": {}}
        samples = {}
        for r in range(args.rounds):
            for axis in AXES:
                sides = [("pruv", [exe, "perf", axis, str(M4_N), str(M4_REPS)], "total_ns_per_op"),
                         ("hand", [hand, "perf", axis, str(M4_N), str(M4_REPS)], "hand_ns_per_op")]
                if r % 2:
                    sides.reverse()
                for side, cmd, key in sides:
                    c = subprocess.run(tc + cmd, capture_output=True, text=True, timeout=600)
                    if c.returncode != 0:
                        raise RuntimeError("%s failed:\n%s" % (" ".join(cmd), c.stderr[-1500:]))
                    d = kv(c.stdout.strip().splitlines()[-1])
                    samples.setdefault((axis, side), []).append(
                        (float(d[key]), parse_time(c.stderr)[1] or 0, int(d.get("allocs", -1))))
        for axis in AXES:
            row = {}
            for side in ("pruv", "hand"):
                s = samples[(axis, side)]
                row[side] = {"ns": statistics.median(x[0] for x in s), "rss": statistics.median(x[1] for x in s),
                             "allocs": statistics.median(x[2] for x in s),
                             "ns_range": [min(x[0] for x in s), max(x[0] for x in s)]}
            res["m4"][axis] = row
        return res
    finally:
        m34.run = orig_run
        shutil.rmtree(tmp, ignore_errors=True)


# ---- row --------------------------------------------------------------------------------------------------------

def header():
    return ("| date | commit | pruvc lines | pruvc-rs lines | big changes | proof/compile | perf vs hand | CI | "
            "work/ | notes |\n|---|---|---|---|---|---|---|---|---|---|")


def row(m, last):
    def lines(name):
        v = m["lines"][name]
        prev = (last or {}).get("lines", {}).get(name)
        return fmt_int(v) + ("" if prev is None else " (%s)" % fmt_delta(v - prev))

    big = "; ".join("%s %s" % (os.path.basename(f), fmt_int(n)) for f, n in m["top_files"])
    f = m.get("full")
    if f:
        mib = lambda b: "%d MiB" % round(b / (1 << 20)) if b else "?"
        proof = "m4: pruvc %.2f s, %s peak (compiler + z3); rustc %.2f s, %s peak; binary %s B" % (
            f["pruvc_s"] or 0, mib(f["pruvc_rss"]), f["rustc_s"] or 0, mib(f["rustc_rss"]), fmt_int(f["binary_bytes"]))
    else:
        proof = "not measured (fast mode; --full)"
    above = m["above_hand"]
    perf = ("PERF_HEADROOM >1.0x hand: %d items (%s)" % (len(above), ", ".join("%s %.2f" % (i, r) for i, r, _ in above))
            if above else "PERF_HEADROOM: no item above 1.0x hand")
    if f:
        parts = []
        for axis in AXES:
            p, h = f["m4"][axis]["pruv"], f["m4"][axis]["hand"]
            parts.append("%s %.2fx time, %.2fx RSS, allocs %d/%d" % (
                axis, p["ns"] / h["ns"], p["rss"] / h["rss"] if h["rss"] else 0, p["allocs"], h["allocs"]))
        perf += "; M4 n=%d vs hand: %s" % (M4_N, "; ".join(parts))
    ci = "gh unavailable" if m["ci"] is None else ", ".join("%s %s" % (sha, c) for _, c, sha in m["ci"])
    if m.get("work_bytes") is None:
        work = "no work/"
    else:
        state = {True: "backed up", False: "changed since backup", None: "backup unknown"}[m["backed_up"]]
        work = "%s, %s" % (human(m["work_bytes"]), state)
    notes = "disk %s free%s; Pruv %s" % (human(m["disk_free"]), " (<20GB)" if m["disk_free"] < DISK_MIN else "",
                                        human(m["pruv_bytes"]) if m["pruv_bytes"] else "?")
    cells = [m["date"], m["commit"], lines("pruvc"), lines("pruvc-rs"), big, proof, perf, ci, work, notes]
    return "| " + " | ".join(c.replace("|", "/") for c in cells) + " |"


# ---- compare ----------------------------------------------------------------------------------------------------

def compare(m, last):
    """Lines past their thresholds (see the module docstring). `last` may be None or partial."""
    out = []
    last = last or {}
    for name, _ in COMPILERS:
        prev = last.get("lines", {}).get(name)
        if prev is not None and m["lines"][name] - prev > LINES_JUMP:
            out.append("lines: %s %s since %s (> +%s)" % (name, fmt_delta(m["lines"][name] - prev),
                                                        last.get("commit", "last state"), fmt_int(LINES_JUMP)))
    prev_items = {i for i, _, _ in last.get("above_hand", [])}
    for item, r, status in m["above_hand"]:
        out.append("perf: %s %.2fx hand%s [%s]" % (item, r, " (new)" if "above_hand" in last and item not in prev_items else "",
                                                  status[:60]))
    f, lf = m.get("full"), last.get("full")
    if f and lf:
        for key, label in (("pruvc_s", "m4 proof time"), ("rustc_s", "m4 rustc time"), ("binary_bytes", "m4 binary"),
                           ("pruvc_rss", "pruvc+z3 peak memory"), ("rustc_rss", "rustc peak memory")):
            a, b = f.get(key), lf.get(key)
            if a and b and a > b * (1 + GROWTH):
                out.append("%s: %.4g -> %.4g (+%.0f%%, > %d%%)" % (label, b, a, (a / b - 1) * 100, GROWTH * 100))
        for axis in AXES:
            for key, label in (("rss", "peak RSS"), ("allocs", "allocations")):
                a = f["m4"].get(axis, {}).get("pruv", {}).get(key)
                b = lf.get("m4", {}).get(axis, {}).get("pruv", {}).get(key)
                if a and b and a > b * (1 + GROWTH):
                    out.append("m4 %s %s: %.4g -> %.4g (+%.0f%%, > %d%%)" % (axis, label, b, a, (a / b - 1) * 100,
                                                                         GROWTH * 100))
    if f:
        for axis in AXES:
            p, h = f["m4"][axis]["pruv"], f["m4"][axis]["hand"]
            if p["ns"] > h["ns"]:
                out.append("perf: M4 %s n=%d %.2fx hand (median of runs, ranges Pruv %.1f-%.1f / hand %.1f-%.1f ns)" % (
                    axis, M4_N, p["ns"] / h["ns"], p["ns_range"][0], p["ns_range"][1], h["ns_range"][0],
                    h["ns_range"][1]))
    for _, c, sha in m["ci"] or []:
        if c not in ("success", "in_progress", "queued", "cancelled", "pending", "waiting", "requested"):
            out.append("ci: %s %s" % (sha, c))
    if m.get("backed_up") is False:
        out.append("work/: changed after the last backup (run backup.sh in the main session)")
    if m["disk_free"] < DISK_MIN:
        out.append("disk: %s free (< 20GB)" % human(m["disk_free"]))
    return out


# ---- state ------------------------------------------------------------------------------------------------------

def load_state(path):
    try:
        with open(path) as fh:
            return json.load(fh).get("runs", [])
    except (OSError, ValueError):
        return []


def last_log_row(path):
    """Fallback 'last state' from the last row of health_log.md: only the two line counts are reliable there."""
    try:
        with open(path, encoding="utf-8") as fh:
            rows = [l for l in fh if l.startswith("| 20")]
    except OSError:
        return None
    for l in reversed(rows):
        cells = [c.strip() for c in l.strip().strip("|").split("|")]
        nums = [re.match(r"([\d,]+)", c) for c in cells[2:4]]
        if len(cells) > 3 and all(nums):
            return {"commit": cells[1], "lines": {"pruvc": int(nums[0].group(1).replace(",", "")),
                                                  "pruvc-rs": int(nums[1].group(1).replace(",", ""))}}
    return None


def last_full(runs):
    for r in reversed(runs):
        if r.get("full"):
            return r["full"]
    return None


# ---- branchy functions ------------------------------------------------------------------------------------------

PY_BRANCH = (ast.If, ast.For, ast.While, ast.IfExp, ast.ExceptHandler, ast.comprehension)
RS_BRANCH = re.compile(r"\bif\b|\bwhile\b|\bfor\b|\bloop\b|=>|&&|\|\||\?(?=[\s.;)])")
RS_FN = re.compile(r"\bfn\s+(\w+)")


def py_branches(src, rel):
    out = []
    for node in ast.walk(ast.parse(src)):
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            n = 0
            for sub in ast.walk(node):
                if isinstance(sub, PY_BRANCH):
                    n += 1
                elif isinstance(sub, ast.BoolOp):
                    n += len(sub.values) - 1
                elif sub.__class__.__name__ == "match_case":
                    n += 1
            out.append((n, "%s:%d %s" % (rel, node.lineno, node.name)))
    return out


def strip_rs(src):
    """Rust source with comments, strings and char literals blanked (newlines kept), in one left-to-right pass so a
    '"' char literal or a "//" inside a string is read correctly; lifetimes ('a) are left alone."""
    out, i, n = [], 0, len(src)
    while i < n:
        c = src[i]
        if src.startswith("//", i):
            j = src.find("\n", i)
            i = n if j < 0 else j
        elif src.startswith("/*", i):
            depth, j = 0, i
            while j < n:
                if src.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif src.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                    if depth == 0:
                        break
                else:
                    j += 1
            out.append("\n" * src.count("\n", i, j))
            i = j
        elif c == '"' or (c in "rb" and re.match(r'(?:br|r)#*"', src[i:i + 40]) and
                          (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_"))):
            m = re.match(r'(?:b?r)(#*)"', src[i:i + 40])
            if m:
                end = src.find('"' + m.group(1), i + m.end())
                j = n if end < 0 else end + 1 + len(m.group(1))
            else:
                j = i + 1
                while j < n and src[j] != '"':
                    j += 2 if src[j] == "\\" else 1
                j += 1
            out.append('""' + "\n" * src.count("\n", i, j))
            i = j
        elif c == "'":
            m = re.match(r"'(?:\\(?:u\{[0-9a-fA-F]+\}|x[0-9a-fA-F]{2}|.)|[^'\\\n])'", src[i:i + 12])
            if m:
                out.append("' '")
                i += m.end()
            else:
                out.append(c)
                i += 1
        else:
            out.append(c)
            i += 1
    return "".join(out)


def rs_branches(src, rel):
    """Per fn: branch tokens between its body braces (nested fns and closures count toward the enclosing fn too)."""
    s = strip_rs(src)
    out = []
    for m in RS_FN.finditer(s):
        i = s.find("{", m.end())
        semi = s.find(";", m.end())
        if i < 0 or (0 <= semi < i):
            continue      # trait method declaration
        depth, j = 0, i
        while j < len(s):
            if s[j] == "{":
                depth += 1
            elif s[j] == "}":
                depth -= 1
                if depth == 0:
                    break
            j += 1
        out.append((len(RS_BRANCH.findall(s[i:j])), "%s:%d %s" % (rel, s.count("\n", 0, m.start()) + 1, m.group(1))))
    return out


def branchy(k=15):
    found = []
    for name, pat in COMPILERS:
        for rel in filter(None, (sh(["git", "ls-files", "-z", pat]) or "").split("\0")):
            with open(os.path.join(ROOT, rel), encoding="utf-8") as fh:
                src = fh.read()
            found += py_branches(src, rel) if rel.endswith(".py") else rs_branches(src, rel)
    return sorted(found, key=lambda x: (-x[0], x[1]))[:k]


# ---- main -------------------------------------------------------------------------------------------------------

def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--full", action="store_true", help="also measure budget m4 (proof, rustc, binary, memory vs hand)")
    ap.add_argument("--compare", action="store_true", help="print only the lines past their thresholds")
    ap.add_argument("--save", action="store_true", help="append this run to the state file")
    ap.add_argument("--header", action="store_true", help="print the table header before the row")
    ap.add_argument("--branchy", action="store_true", help="print the 15 most branchy functions and exit")
    ap.add_argument("--state", default=None, help="state file (default: work/health_state.json in the main checkout)")
    ap.add_argument("--log", default=None, help="health log for the fallback last row "
                                                "(default: docs/notes/health_log.md in the main checkout)")
    ap.add_argument("--pruvc", default=None, help="compiler for --full (default: pruvc-rs release binary)")
    ap.add_argument("--rounds", type=int, default=3, help="--full: alternating Pruv/hand rounds per M4 axis")
    args = ap.parse_args()

    if args.branchy:
        for n, where in branchy():
            print("%4d  %s" % (n, where))
        return 0
    main_dir = main_checkout()
    state = args.state or os.path.join(main_dir, "work", "health_state.json")
    runs = load_state(state)
    last = runs[-1] if runs else last_log_row(args.log or os.path.join(main_dir, "docs", "notes", "health_log.md"))

    t0 = time.time()
    m = fast(args)
    if args.full:
        args.pruvc = args.pruvc or default_pruvc(main_dir)
        m["full"] = full(args, main_dir)
    m["elapsed_s"] = round(time.time() - t0, 1)

    rc = 0
    if args.compare:
        ref = dict(last or {})
        if m.get("full") and not ref.get("full"):
            ref["full"] = last_full(runs)
        lines = compare(m, ref)
        print("compared with %s (%s)" % (ref.get("commit", "nothing"), ref.get("date", "health_log row")
                                         if ref else "no state"))
        for l in lines:
            print("  " + l)
        if not lines:
            print("  nothing past its threshold")
        rc = 1 if lines else 0
    else:
        if args.header:
            print(header())
        print(row(m, last))
    if args.save:
        os.makedirs(os.path.dirname(state), exist_ok=True)
        with open(state, "w") as fh:
            json.dump({"runs": (runs + [m])[-50:]}, fh, indent=1)
    print("(%.1f s)" % m["elapsed_s"], file=sys.stderr)
    return rc


if __name__ == "__main__":
    sys.exit(main())
