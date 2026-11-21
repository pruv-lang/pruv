#!/usr/bin/env python3
"""Compare direct Rust and compiled Pruv on the R02 entry workloads."""

import argparse
import json
import os
import re
import signal
import statistics
import subprocess
import sys
import time
from pathlib import Path

sys.dont_write_bytecode = True
from perf_wb import case

HERE = Path(__file__).resolve().parent
HARNESS = {lane: HERE / "work" / directory / "harness"
           for lane, directory in (("pruv", "dev"), ("hand", "hand"))}
RUNS = 7
TIMEOUT = 60
COUNTER = "PRUV_R02_REPORT_PREV"
TIME = ["/usr/bin/time", "-l"]
WAIT4 = [sys.executable, "-c", """
import os, sys, time
started = time.perf_counter()
pid = os.posix_spawn(sys.argv[1], sys.argv[1:], os.environ)
_, status, usage = os.wait4(pid, 0)
print(f"r02_wall {time.perf_counter() - started}", file=sys.stderr)
print(f"{usage.ru_maxrss} maximum resident set size", file=sys.stderr)
sys.exit(os.waitstatus_to_exitcode(status))
"""]


def run(lane, request, *, timer=(), counters=False):
    env = os.environ.copy()
    env.pop(COUNTER, None)
    if counters:
        env[COUNTER] = "1"
    started = time.perf_counter()
    with subprocess.Popen(
        [*timer, str(HARNESS[lane])], stdin=subprocess.PIPE,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, start_new_session=True,
    ) as process:
        try:
            output, stderr = process.communicate(request, timeout=TIMEOUT)
        except (subprocess.TimeoutExpired, KeyboardInterrupt):
            # The timer and its harness share this run's private process group.
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.communicate()
            raise
    elapsed = time.perf_counter() - started
    if process.returncode:
        raise RuntimeError(f"{lane}: {stderr.decode(errors='replace')}")
    if not timer:
        return output, stderr
    rss = re.search(rb"(?m)^\s*(\d+)\s+maximum resident set size\s*$", stderr)
    if rss is None:
        raise RuntimeError(f"{lane}: timer did not report maximum RSS")
    if timer == WAIT4:
        elapsed = float(re.search(rb"(?m)^r02_wall (\S+)$", stderr)[1])
    return output, elapsed, int(rss[1])


def check_match(actual, expected, label):
    if actual != expected:
        raise AssertionError(f"{label}: Pruv and hand stdout differ")


def report_counters(label, request, expected):
    output, stderr = run("pruv", request, counters=True)
    check_match(output, expected, label)
    if json.loads(output).get("terminal") != "ok":
        raise AssertionError(f"{label}: workload did not finish successfully")
    words = re.findall(rb"(?m)^r02_prev_words (\d+)$", stderr)
    if not words:
        raise AssertionError(f"{label}: no r02_prev_words lines; rebuild work/dev/harness (sh dev_build.sh)")
    if words:
        total = sum(map(int, words))
        print(f"{label}: Pruv entry_step={len(words)}, prev={total} words / {8 * total} bytes; "
              "hand has no packet boundary", flush=True)
    print(f"{label}: stdout={len(output.splitlines())} JSON lines / {len(output)} bytes per lane",
          flush=True)


def measure(size, count, timer):
    label = f"{size} ({count} sessions)"
    request = (case(count) + "\n").encode()
    measurements = {lane: [] for lane in HARNESS}
    expected = None
    for _ in range(RUNS):
        for lane in HARNESS:
            output, elapsed, rss = run(lane, request, timer=timer)
            if expected is None:
                expected = output
            check_match(output, expected, label)
            measurements[lane].append((elapsed, rss))
    report_counters(label, request, expected)
    cells = []
    for index, (axis, unit) in enumerate((("time", "s"), ("rss", "bytes"))):
        medians = {
            lane: statistics.median(sample[index] for sample in measurements[lane])
            for lane in HARNESS
        }
        cells.append(dict(name="bound_session_index", axis=axis, size=size,
                          kind="proportional", unit=unit, runs=RUNS, **medians))
    return cells


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    # One clock for both lanes: wait4 times the harness alone and reports its max RSS.
    timer = WAIT4

    with (HERE / "fixtures/D.json").open() as fixture:
        read_case = json.load(fixture)[0]["subs"][1]
    request = (json.dumps(read_case) + "\n").encode()
    output, _ = run("hand", request)
    report_counters("read_external", request, output)

    cells = measure("M", 1000, timer)
    # L is fixed; a timeout fails the run instead of relabelling a smaller load as L.
    cells.extend(measure("L", 4000, timer))
    args.out.write_text(json.dumps(cells, indent=2) + "\n")


if __name__ == "__main__":
    main()
