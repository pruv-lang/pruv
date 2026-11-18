#!/usr/bin/env python3
"""Check that the M9 active-count invariant detects four clock mutations."""

import argparse
import json
from pathlib import Path
import re
import subprocess
import sys

sys.dont_write_bytecode = True
from run_m9 import BUDLIB_WORK, key

HERE = Path(__file__).resolve().parent
BUD = HERE.parent
ROOT = BUD.parents[2]
WORK = BUD / "work" / "m9"
PRUVC_DEFAULT = "/Users/spica4345/Projects/pruv/pruvc-rs/target/release/pruvc"

# Match the update statement by helper name or by a direct write to field 17.
ACTIVE_UPDATE = (
    r"(?:\w+::)*(?:add_active\s*\([^;{}]*\)"
    r"|(?:wf|set_ef)\s*\([^,;{}]+,[^,;{}]+,\s*17\s*,[^;{}]*\))\s*;"
)
GUARDED_UPDATE = rf"\bif\s+[^{{}};]+\{{\s*({ACTIVE_UPDATE})\s*\}}"
MUTANTS = (
    ("waiting_no_dec", ACTIVE_UPDATE, ""),
    ("started_no_inc", GUARDED_UPDATE, ""),
    ("ended_no_dec", GUARDED_UPDATE, ""),
    # Drop only the waiting test; the base bound (ca >= 0) stays so the mutant still proves.
    ("ended_always_dec", r"\s*&&\s*w\s*!=\s*1\b", ""),
)


def mutate(source, name, pattern, replacement):
    # The first part of each mutant's name identifies the run operation.
    operation = name.split("_", 1)[0]
    functions = list(re.finditer(rf"^(?:pub\s+)?fn run_{operation}\b[^{{]*\{{\n(.*?)^\}}",
                                 source, re.MULTILINE | re.DOTALL))
    assert len(functions) == 1, f"{name}: expected one run_{operation} function, found {len(functions)}"
    function = functions[0]
    body, matches = re.subn(pattern, replacement, function[1])
    assert matches == 1, f"{name}: expected one active-count update, found {matches}"
    return source[:function.start(1)] + body + source[function.end(1):]


def check_waiting_end():
    sid, bid = key("s1"), key("b1")
    budget = [0, 0, 1, 0, 0, 0] + bid
    lines = ["case end-waiting-with-active", "put " + " ".join(map(str, sid + [len(budget)] + budget))]
    # Keep one active run so an erroneous decrement produces a mismatch even with clamping.
    for code, run, flag in ((1, "active", 0), (1, "waiting", 0), (2, "waiting", 1),
                            (3, "waiting", 0), (3, "active", 0)):
        op = [code] + sid + bid + key(run) + [flag]
        lines.append("call " + " ".join(map(str, op)))
    lines.append("end")
    result = subprocess.run([str(Path(BUDLIB_WORK) / "driver_m9"), "q"],
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


def run_q(pruvc, budlib, suffix):
    result = subprocess.run(
        [sys.executable, str(HERE / "run_m9.py"), "q", "--budlib", str(budlib), "--pruvc", pruvc,
         "--cases", str(WORK / f"cases{suffix}.jsonl"), "--oracle", str(WORK / f"oracle{suffix}.jsonl")],
        cwd=ROOT, capture_output=True, text=True,
    )
    first, _, log = result.stdout.partition("\n")
    if result.returncode not in (0, 1) or not first:
        sys.exit(f"{budlib.name} cases{suffix}: q failed (exit {result.returncode})\n"
                 + result.stderr + result.stdout)
    summary = json.loads(first)
    bad = summary["i2_bad"]
    fails = summary["count"]["fail"]
    i2_failures = sum(line.startswith("I2 FAIL ") for line in log.splitlines())
    if (summary["i2_checks"] <= 0 or bad != i2_failures
            or result.returncode != int(bool(bad or fails))):
        sys.exit(f"{budlib.name} cases{suffix}: inconsistent q result\n" + first)
    return bad, fails


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pruvc", default=PRUVC_DEFAULT)
    args = parser.parse_args()

    sources = {path.name: path.read_bytes() for path in sorted(BUD.glob("*.pruv"))}
    clock = sources["bud_clock.pruv"].decode("utf-8")
    variants = {"control": clock}
    for name, pattern, replacement in MUTANTS:
        variants[name] = mutate(clock, name, pattern, replacement)

    failed = False
    print("name i2_bad q_fail verdict", flush=True)
    for name, source in variants.items():
        budlib = WORK / "mut" / name
        budlib.mkdir(parents=True, exist_ok=True)
        for filename, contents in sources.items():
            (budlib / filename).write_bytes(contents)
        if name != "control":
            (budlib / "bud_clock.pruv").write_text(source, encoding="utf-8")

        q3_bad, q3_fails = run_q(args.pruvc, budlib, "")
        clock_bad, clock_fails = run_q(args.pruvc, budlib, "_clock")
        bad = q3_bad + clock_bad + check_waiting_end()
        fails = q3_fails + clock_fails
        if name == "control":
            verdict = "CLEAN" if bad == 0 and fails == 0 else "DIRTY"
        else:
            verdict = "KILLED" if bad > 0 else "SURVIVED"
        print(name, bad, fails, verdict, flush=True)
        failed |= verdict in ("DIRTY", "SURVIVED")
    return int(failed)


if __name__ == "__main__":
    sys.exit(main())
