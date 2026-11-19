#!/usr/bin/env python3
"""Check M9 clock invariants and oracle comparisons against isolated mutants."""

import argparse
from argparse import Namespace
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
from run_m9 import build, check_dc, check_q, key

HERE = Path(__file__).resolve().parent
BUD = HERE.parent
WORK = BUD / "work" / "m9"
PRUVC_DEFAULT = "/Users/spica4345/Projects/pruv/pruvc-rs/target/release/pruvc"

# Match the update statement by helper name or by a direct write to field 17.
ACTIVE_UPDATE = (
    r"(?:\w+::)*(?:add_active\s*\([^;{}]*\)"
    r"|(?:wf|set_ef)\s*\([^,;{}]+,[^,;{}]+,\s*17\s*,[^;{}]*\))\s*;"
)
GUARDED_UPDATE = rf"\bif\s+[^{{}};]+\{{\s*({ACTIVE_UPDATE})\s*\}}"
MUTANTS = (
    # Keep the increment (waiting -> active) and drop only the decrement (active -> waiting).
    ("waiting_no_dec", ACTIVE_UPDATE, r"if !waiting { \g<0> }"),
    ("started_no_inc", GUARDED_UPDATE, ""),
    ("ended_no_dec", GUARDED_UPDATE, ""),
    # Drop only the waiting test; the base bound (ca >= 0) stays so the mutant still proves.
    ("ended_always_dec", r"\s*&&\s*w\s*!=\s*1\b", ""),
)


# These replacements are scoped to the named functions and must each match once.
CLOCK_MUTANTS = (
    ("m5_drop_no_clear", "drop_at", r"push\(inout fx, 13\);", ""),
    ("m6_fire_clear", "fire_at", r"push\(inout fx, 14\);", "push(inout fx, 13);"),
    ("m7_fire_late_forget", "fire_at",
     r"(  if ca >= 0 \{\n.*?^  \}\n)(  if bv\[0\].*?\n)", r"\2\1"),
    ("m8_close_no_settle", "close", r"  let a: i64 = settle_at\([^\n]+\);\n", ""),
    ("m9_drop_no_remove", "drop_at", r"  crate::bud_map::remove\(inout st, ce\);\n", ""),
    ("m10_fire_active_count", "fire_at", r"st\[ca \+ 0\]", "st[ca + 6]"),
)


def mutate(source, name, function_name, pattern, replacement):
    functions = list(re.finditer(rf"^(?:pub\s+)?fn {function_name}\b[^{{]*\{{\n(.*?)^\}}",
                                 source, re.MULTILINE | re.DOTALL))
    assert len(functions) == 1, f"{name}: expected one {function_name} function, found {len(functions)}"
    function = functions[0]
    body, matches = re.subn(pattern, replacement, function[1], flags=re.MULTILINE | re.DOTALL)
    assert matches == 1, f"{name}: expected one mutation site, found {matches}"
    return source[:function.start(1)] + body + source[function.end(1):]


def check_waiting_end(exe):
    sid, bid = key("s1"), key("b1")
    budget = [0, 0, 1, 0, 0, 0] + bid
    lines = ["case end-waiting-with-active", "put " + " ".join(map(str, sid + [len(budget)] + budget))]
    # Keep one active run so an erroneous decrement produces a mismatch even with clamping.
    for code, run, flag in ((1, "active", 0), (1, "waiting", 0), (2, "waiting", 1),
                            (3, "waiting", 0), (3, "active", 0)):
        op = [code] + sid + bid + key(run) + [flag]
        lines.append("call " + " ".join(map(str, op)))
    lines.append("end")
    result = subprocess.run([exe, "q"],
                            input="\n".join(lines) + "\n", capture_output=True, text=True)
    log = result.stdout.splitlines()
    bad = sum(line.startswith("I2 ") for line in log)
    summary = re.search(r"^i2_checks=(\d+) i2_bad=(\d+)$", result.stderr, re.MULTILINE)
    if (result.returncode != (3 if bad else 0) or not log or log[-1] != "end"
            or any(line.startswith("X ") for line in log)
            or summary is None or int(summary[1]) < len(lines) - 2 or int(summary[2]) != bad):
        sys.exit(f"end-waiting-with-active: driver failed (exit {result.returncode})\n"
                 + result.stderr + result.stdout)
    return bad


def check_variant(name, source, sources, pruvc, campaign):
    budlib = campaign / name
    build_dir = budlib / "build"
    budlib.mkdir(parents=True, exist_ok=True)
    for filename, contents in sources.items():
        (budlib / filename).write_bytes(contents)
    (budlib / "bud_clock.pruv").write_text(source, encoding="utf-8")
    exe = build(Namespace(pruvc=pruvc, budlib=str(budlib), out=str(build_dir), gen=None))

    results = {}
    for label, suffix in (("q3", ""), ("clock", "_clock"), ("stale", "_stale")):
        args = Namespace(cases=WORK / f"cases{suffix}.jsonl", oracle=WORK / f"oracle{suffix}.jsonl")
        results[label] = check_set(build_dir, label, check_q, args, exe)
    results["dc"] = check_set(build_dir, "dc", check_dc, Namespace(count=300, seed=20261117), exe)
    waiting_bad = check_waiting_end(exe)
    bad = waiting_bad + sum(row["i2_bad"] for row in results.values())
    q_fails = [results[label]["count"]["fail"] for label in ("q3", "clock", "stale")]
    dc_fails = results["dc"]["fail"]
    detected = bool(bad or any(q_fails) or dc_fails)
    if name == "control":
        verdict = "DIRTY" if detected else "CLEAN"
    else:
        verdict = "KILLED" if detected else "SURVIVED"
    return (name, bad, *q_fails, dc_fails, verdict)


def check_set(build_dir, name, check, args, exe):
    log = []
    exit_code = check(args, exe, log.append)
    (build_dir / f"{name}.log").write_text("\n".join(log) + "\n", encoding="utf-8")
    summary = json.loads(log[0])
    failures = summary["fail"] if name == "dc" else summary["count"]["fail"]
    if summary["i2_checks"] <= 0 or exit_code != int(bool(summary["i2_bad"] or failures)):
        sys.exit(f"{build_dir.parent.name} {name}: inconsistent result\n{log[0]}")
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pruvc", default=PRUVC_DEFAULT)
    parser.add_argument("--jobs", type=int, default=4)
    args = parser.parse_args()
    if args.jobs < 1:
        parser.error("--jobs must be positive")

    sources = {path.name: path.read_bytes() for path in sorted(BUD.glob("*.pruv"))}
    clock = sources["bud_clock.pruv"].decode("utf-8")
    variants = {"control": clock}
    for name, pattern, replacement in MUTANTS:
        function = "run_" + name.split("_", 1)[0]
        variants[name] = mutate(clock, name, function, pattern, replacement)
    for name, function, pattern, replacement in CLOCK_MUTANTS:
        variants[name] = mutate(clock, name, function, pattern, replacement)

    # one fresh directory per campaign: two concurrent mut_m9 runs never share a mutant library or build (P23 H6)
    (WORK / "mut").mkdir(parents=True, exist_ok=True)
    campaign = Path(tempfile.mkdtemp(prefix="campaign-", dir=WORK / "mut"))
    print(f"campaign {campaign}", file=sys.stderr, flush=True)
    failed = False
    print("name i2_bad q3_fail clock_fail stale_fail dc_fail verdict", flush=True)
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        futures = [pool.submit(check_variant, name, source, sources, args.pruvc, campaign)
                   for name, source in variants.items()]
        for future in futures:
            row = future.result()
            print(*row, flush=True)
            failed |= row[-1] in ("DIRTY", "SURVIVED")
    return int(failed)


if __name__ == "__main__":
    sys.exit(main())
