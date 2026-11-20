#!/usr/bin/env python3
"""Interleaved parse and query measurements against hand Rust."""
import argparse
import json
import os
from pathlib import Path
import re
import statistics
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
from run_r03_equal import HERE, WORK, build_runner, run

SIZES = {"S": 64 * 1024, "M": 1024 * 1024, "L": 8 * 1024 * 1024}
PHASES = {"parse": 0, "query_chain1": 1, "query_chain10": 10, "query_chain100": 100}
SAMPLES = 7
LANES = ("pruv", "hand")
TIME = "/usr/bin/time"

HARNESS = r'''#![allow(dead_code)]
#[path = @PARSE@] mod a2;
#[path = @QUERY@] mod sol;
#[path = @HAND@] mod hand;
use std::{alloc::{GlobalAlloc, Layout, System}, cell::Cell, env, fs, hint::black_box, time::Instant};

#[derive(Clone, Copy, Default)]
struct Allocations {
    count: usize,
    bytes: usize,
}

thread_local! {
    static ACTIVE: Cell<Option<Allocations>> = const { Cell::new(None) };
}

fn record_allocation(ptr: *mut u8, size: usize) {
    if !ptr.is_null() {
        let _ = ACTIVE.try_with(|active| {
            if let Some(mut sample) = active.get() {
                sample.count += 1;
                sample.bytes += size;
                active.set(Some(sample));
            }
        });
    }
}

struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        record_allocation(ptr, layout.size());
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        record_allocation(ptr, layout.size());
        ptr
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let ptr = unsafe { System.realloc(ptr, layout, size) };
        record_allocation(ptr, size);
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

struct Job {
    data: Vec<u8>,
    ops: Vec<i64>,
    steps: usize,
}

impl Job {
    fn call(&self, generated: bool) -> Vec<i64> {
        let input = black_box(self.data.as_slice());
        if self.steps == 0 {
            return if generated { a2::parse_tape(input) } else { hand::parse_tape(input) };
        }
        let mode = if self.steps == 1 { 1 } else { 4 };
        let query = if generated { sol::query } else { hand::query };
        query(input, black_box(&self.ops), &[], "next", mode)
    }

    fn validate(&self, out: &[i64]) {
        match self.steps {
            0 => {
                assert_eq!(out[0..3], [5, 0, self.data.iter().rposition(|&b| b == b']').unwrap() as i64 + 1]);
                assert_eq!(out[3] as usize * 6, out.len());
            }
            1 => {
                assert_eq!(out.first(), Some(&i64::from(b'{')));
                assert_eq!(out.last(), Some(&i64::from(b'}')));
            }
            10 | 100 => assert_eq!(out, [0, 1, 42, 0, 52, 50]),
            _ => unreachable!(),
        }
    }

    fn sample(&self, generated: bool) {
        hand::COUNTERS.with(|c| c.set(hand::Counters::default()));
        ACTIVE.set(Some(Allocations::default()));
        let start = Instant::now();
        let out = black_box(self.call(generated));
        let elapsed = start.elapsed();
        let allocations = ACTIVE.replace(None).unwrap();
        let hand = hand::COUNTERS.get();
        self.validate(&out);
        let lane = if generated { "pruv" } else { "hand" };
        println!("{lane} {} {} {} {} {} {}", elapsed.as_nanos(),
                 allocations.count, allocations.bytes, hand.bytes_copied,
                 hand.allocations, hand.nodes_visited);
    }
}

fn main() {
    let args: Vec<_> = env::args().collect();
    let data = fs::read(&args[2]).unwrap();
    let steps = args[3].parse().unwrap();
    let mut ops = Vec::with_capacity(2 * steps);
    match steps {
        0 => {}
        1 => ops.extend([1, 2]),
        10 | 100 => {
            for level in 0..steps / 2 {
                let index = i64::from(steps == 100 && level == 0);
                ops.extend([1, index, 3, 0]);
            }
        }
        _ => unreachable!(),
    }
    let job = Job { data, ops, steps };
    if args[1] == "time" {
        let mut lanes = [true, false];
        for generated in lanes {
            job.validate(&black_box(job.call(generated)));
        }
        let samples: usize = args[4].parse().unwrap();
        for _ in 0..samples {
            for generated in lanes {
                job.sample(generated);
            }
            lanes.reverse();
        }
    } else {
        job.validate(&black_box(job.call(args[1] == "pruv")));
    }
}
'''


def documents():
    def branch(levels):
        value = b'{"next":42}'
        for _ in range(levels - 1):
            value = b'{"next":[' + value + b']}'
        return value

    prefix = b"[" + branch(5) + b"," + branch(50)
    records = [
        b"," + json.dumps({
            "name": f'row {i} "quoted"\n한글 😀', "count": i - 50, "active": bool(i % 2),
            "nested": [i, [i / 10, None, {"text": "line\tbreak", "extra": "x" * 84}]],
        }, ensure_ascii=bool(i % 2), separators=(",", ":")).encode()
        for i in range(100)
    ]
    chunk = b"".join(records)
    for size, length in SIZES.items():
        repeats, remaining = divmod(length - len(prefix) - 1, len(chunk))
        body = bytearray(prefix + chunk * repeats)
        for record in records:
            if len(record) > remaining:
                break
            remaining -= len(record)
            body.extend(record)
        body.extend(b"]" + b" " * remaining)
        yield size, body


def timing(exe, path, steps):
    lines = run([exe, "time", path, str(steps), str(SAMPLES)], capture_output=True).stdout
    lanes = {lane: {"samples": []} for lane in LANES}
    order = []
    for line in lines.splitlines():
        lane, ns, count, size, copied, hand_allocs, visits = line.split()
        order.append(lane)
        sample = {"ms": int(ns) / 1e6, "allocations": int(count), "allocated_bytes": int(size)}
        if lane == "hand":
            sample["counters"] = {
                "bytes_copied": int(copied), "allocations": int(hand_allocs), "nodes_visited": int(visits),
            }
        lanes[lane]["samples"].append(sample)
    assert order == [lane for i in range(SAMPLES) for lane in (LANES if i % 2 == 0 else LANES[::-1])]
    for lane in lanes.values():
        for field in ("ms", "allocations", "allocated_bytes"):
            lane[f"median_{field}"] = statistics.median(s[field] for s in lane["samples"])
    return lanes, order


def rss_method():
    probe = subprocess.run([TIME, "-l", "/usr/bin/true"], text=True, capture_output=True)
    if probe.returncode == 0 and "maximum resident set size" in probe.stderr:
        return "/usr/bin/time -l", None
    if "sysctl kern.clockrate: Operation not permitted" in probe.stderr and hasattr(os, "wait4"):
        return "wait4(ru_maxrss)", probe.stderr.strip().splitlines()[-1]
    raise RuntimeError(f"Cannot collect RSS using {TIME} -l: {probe.stderr.strip()}")


def peak_rss(method, command):
    if method == "/usr/bin/time -l":
        result = run([TIME, "-l", *command], capture_output=True)
        match = re.search(r"^\s*(\d+)\s+maximum resident set size", result.stderr, re.MULTILINE)
        if match is None:
            raise RuntimeError(f"Missing peak RSS: {result.stderr}")
        return int(match[1])
    command = list(map(str, command))
    pid = os.posix_spawn(command[0], command, os.environ)
    _, status, usage = os.wait4(pid, 0)
    if status:
        raise RuntimeError(f"RSS child {command}: exit {os.waitstatus_to_exitcode(status)}")
    return usage.ru_maxrss if sys.platform == "darwin" else 1024 * usage.ru_maxrss


def save(path, data):
    temp = path.with_suffix(".tmp")
    temp.write_text(json.dumps(data, indent=2) + "\n")
    temp.replace(path)


def gate_cells(report):
    cells = []
    for size, phases in report["results"].items():
        for phase, row in phases.items():
            lanes = row["lanes"]
            cell_size = "S" if max(l["median_ms"] for l in lanes.values()) < 1 else size
            for axis, field, unit in (
                ("time", "median_ms", "ms"),
                ("rss", "peak_rss_bytes", "bytes"),
                ("allocs", "median_allocations", "calls"),
            ):
                if not all(field in lanes[lane] for lane in LANES):
                    continue
                cell = dict(name=f"{report['root']}/{phase}/{size}", axis=axis, size=cell_size,
                            kind="proportional", pruv=lanes["pruv"][field],
                            hand=lanes["hand"][field], unit=unit)
                if axis != "rss":
                    cell["runs"] = SAMPLES
                cells.append(cell)
    return {"source": f"measure_{Path(report['root']).stem}.json", "cells": cells}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default="a4_root.pruv", type=Path)
    args = parser.parse_args()
    root = args.root if args.root.is_file() else HERE / args.root
    if not root.is_file():
        parser.error(f"root not found: {args.root}")
    root = root.resolve()
    WORK.mkdir(parents=True, exist_ok=True)
    raw = WORK / f"measure_{root.stem}.json"
    report = {
        "root": root.name, "parse_root": "a2_root.pruv", "samples_per_lane": SAMPLES,
        "warmups_per_lane": 1, "document_bytes": SIZES.copy(), "results": {},
        "allocated_bytes": "Requested bytes summed across successful alloc, alloc_zeroed and realloc calls.",
        "complete": False,
    }
    save(raw, report)
    method, reason = rss_method()
    report["rss_method"] = method
    if reason:
        report["rss_time_error"] = reason
        print(f"RSS fallback: {reason}; using {method}", flush=True)
    save(raw, report)
    with tempfile.TemporaryDirectory(prefix="measure-build-", dir=WORK) as directory:
        build = Path(directory)
        exe = build_runner(build, HARNESS, root)
        for size, data in documents():
            assert len(data) == SIZES[size]
            path = build / "input.json"
            path.write_bytes(data)
            report["results"][size] = {}
            for phase, steps in PHASES.items():
                lanes, order = timing(exe, path, steps)
                report["results"][size][phase] = {"lanes": lanes, "sample_order": order}
                save(raw, report)
                save(WORK / "perf_cells.json", gate_cells(report))
                for lane in LANES:
                    lanes[lane]["peak_rss_bytes"] = peak_rss(method, [exe, lane, path, str(steps)])
                    save(raw, report)
                save(WORK / "perf_cells.json", gate_cells(report))
                print(f"{size} {phase}: {lanes['pruv']['median_ms']:.3f} / "
                      f"{lanes['hand']['median_ms']:.3f} ms", flush=True)
    report["complete"] = True
    save(raw, report)


if __name__ == "__main__":
    main()
