#!/usr/bin/env python3
"""Measure the R10 core and whole-process lanes against hand Rust on macOS."""

import argparse
import json
import re
import statistics
import subprocess
import sys
import time

sys.dont_write_bytecode = True
import common
import test_a3

HERE = common.HERE
HAND = HERE / 'hand'
WORK = HERE / 'work' / 'r10'
PAIRS = [('rs', 'rust'), ('rs', 'llvm')]
GEN = r'''
#[path = "@PATTERN@"]
mod pattern;
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::AsFd;

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut remaining: usize = args[1].parse().unwrap();
    let chunk: usize = args[2].parse().unwrap();
    assert!(chunk > 0);
    let buffer: Vec<u8> = pattern::PATTERN.iter().copied().cycle().take(chunk + 63).collect();
    let mut offset = 0;
    let mut out = File::from(io::stdout().as_fd().try_clone_to_owned()?);
    while remaining != 0 {
        let n = remaining.min(chunk);
        out.write_all(&buffer[offset..offset + n])?;
        remaining -= n;
        offset = (offset + n) % pattern::PATTERN.len();
    }
    Ok(())
}
'''


def build_rust(rustc, run, source, directory):
    directory.mkdir(parents=True, exist_ok=True)
    main = directory / 'main.rs'
    main.write_text(source)
    exe = directory / 'runner'
    run([rustc, '-O', '--edition', '2021', main, '-o', exe])
    return exe


def fixture(gen, size, chunk, out, tail):
    hx = test_a3.hx
    return ' '.join(['wait', '0', hx(str(gen)), hx(test_a3.blob([str(size), str(chunk)])),
                     '-', hx(str(out)), str(1 << 30), str(tail)]) + '\n'


def timed(exe, lane, args, size, payload):
    started = time.perf_counter_ns()
    result = subprocess.run(['/usr/bin/time', '-l', str(exe), *args], input=payload,
                            capture_output=True, text=True, timeout=600)
    elapsed = time.perf_counter_ns() - started
    if result.returncode:
        raise RuntimeError(f'{exe}: exit {result.returncode}\n{result.stderr}')
    rss = re.search(r'^\s*(\d+)\s+maximum resident set size\s*$', result.stderr, re.M)
    if not rss:
        raise RuntimeError(f'{exe}: missing macOS max RSS in {result.stderr!r}')
    if lane == 'core':
        ns, length, digest = result.stdout.split()
        return (int(ns), int(rss[1])), (length, digest)

    packet, started_byte, summary = result.stdout.strip().split(' | ')
    fields = packet.split()
    if (fields[:11] != ['0', '1', '0', '1', '0', '1', '0', '0', '1', str(size), str(size)]
            or started_byte != '1' or not summary.startswith(f'{size}:')):
        raise RuntimeError(f'{exe}: unexpected whole-process result {fields[:11]}, {started_byte}, {summary}')
    return (elapsed, int(rss[1])), (packet, started_byte, summary)


def measure(pruv, hand, pair, lane, label, size, chunk, tail, rounds, gen):
    name = f'{lane}/{pair}/chunk={chunk}/T={tail}'
    args = [str(tail), str(chunk), str(size)] if lane == 'core' else []
    payload = fixture(gen, size, chunk, WORK / 'capture.bin', tail) if lane == 'whole' else None
    samples = {'pruv': [], 'hand': []}
    expected = None
    for index in range(rounds):
        order = [('pruv', pruv), ('hand', hand)]
        if index % 2:
            order.reverse()
        for side, exe in order:
            sample, output = timed(exe, lane, args, size, payload)
            if expected is None:
                expected = output
            elif output != expected:
                raise RuntimeError(f'{name} {label} round {index + 1}: {side} output differs from the first run')
            samples[side].append(sample)

    cells = []
    for index, (axis, unit) in enumerate([('time', 'ns'), ('rss', 'bytes')]):
        p, h = (statistics.median(sample[index] for sample in samples[side])
                for side in ('pruv', 'hand'))
        cells.append(dict(name=name, axis=axis, size=label, kind='proportional',
                          pruv=p, hand=h, ratio=p / h, unit=unit, runs=rounds))
        print(f'{name:34} {label} {axis:4} {p:12g} {h:12g} {p / h:8.3f}x', flush=True)
    return cells


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--quick', action='store_true', help='run one round at 16 MiB')
    quick = parser.parse_args().quick
    core_root = HERE / 'core_root.pruv'
    supervise_root = HERE / 'supervise.pruv'
    if not core_root.is_file():
        parser.error(f'missing Pruv core root: {core_root}')
    g2, run = common.configure(WORK)
    pattern = str(HAND / 'pattern.rs')
    core_main = (HAND / 'core_main.rs').read_text().replace('@PATTERN@', pattern)
    whole_main = test_a3.HARNESS
    hand_core = build_rust(g2.RUSTC, run, core_main.replace('@SOL@', str(HAND / 'core_hand.rs')),
                           WORK / 'hand_core')
    hand_whole = build_rust(g2.RUSTC, run, whole_main.replace('@SOL@', str(HAND / 'supervise_hand.rs')),
                            WORK / 'hand_whole')
    gen = build_rust(g2.RUSTC, run, GEN.replace('@PATTERN@', pattern), WORK / 'gen')
    sizes = [('M', 16 << 20)] if quick else [('M', 16 << 20), ('L', 64 << 20)]
    rounds = 1 if quick else 7
    cells = []
    print(f'{"case":34} S {"axis":4} {"Pruv":>12} {"hand":>12} {"Pruv/hand":>9}', flush=True)
    for compiler, backend in PAIRS:
        pair = f'{compiler}/{backend}'
        directory = WORK / f'{compiler}_{backend}'
        core_flags = common.manifest_flags(core_root) if core_root.with_suffix('.pruv.json').is_file() else ()
        pruv_core = common.build(g2, run, compiler, backend, core_root, directory / 'core',
                                 core_main, 'r10_core', core_flags)
        pruv_whole = common.build(g2, run, compiler, backend, supervise_root, directory / 'whole',
                                  whole_main, 'r10_whole', common.manifest_flags(supervise_root))
        for label, size in sizes:
            for chunk in (4, 65536):
                for tail in (4096, 65536):
                    cells.extend(measure(pruv_core, hand_core, pair, 'core', label, size, chunk, tail, rounds, gen))
            cells.extend(measure(pruv_whole, hand_whole, pair, 'whole', label, size, 65536, 4096, rounds, gen))

    (WORK / 'perf_cells.json').write_text(json.dumps({'source': 'supervisor/r10', 'cells': cells}, indent=2) + '\n')


if __name__ == '__main__':
    main()
