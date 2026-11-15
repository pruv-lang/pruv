"""G10 net Q perf: a loopback 1 MiB transfer read in 4 KiB chunks; default verified builds.

A Rust server thread on 127.0.0.1 writes 1048576 bytes (offset % 251) in 65536-byte writes and closes. The client is
either the Pruv pump or a hand-written Rust client (std::net, 4096-byte reads); each timed run is one connect, read to
EOF and close in the same driver process, alternating Pruv and Rust. Mode 0 (per byte: every byte through net_byte into
a checksum) and mode 1 (bulk copy: a net_bytes copy of each chunk, first byte, last byte and length used) are recorded
separately. Both sides must return the exactly expected value (checksum, or total*1000 + first byte), so a lost or
duplicated byte fails the run. Memory is the maximum RSS of one single-side run of the driver process. Time is the
median of the rounds. No target is declared; the numbers are recorded.
"""
import sys
sys.dont_write_bytecode = True
import argparse
import json
from pathlib import Path
import os
import platform
import subprocess
import tempfile
import run as G

N = 1048576
MAXRSS = ('import resource,subprocess,sys\n'
          'r=subprocess.run(sys.argv[1:],capture_output=True)\n'
          'u=resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss\n'
          'print(r.returncode, u if sys.platform=="darwin" else u*1024)')


def peak(exe, mode, side, env):
    r = subprocess.run([sys.executable, '-c', MAXRSS, str(exe), str(mode), '1', side],
                       capture_output=True, text=True, env=env, timeout=120)
    rc, rss = r.stdout.split()
    return {'rc': int(rc), 'max_rss_bytes': int(rss)}


def measure(compiler, backend, repeat):
    env = dict(os.environ)
    with tempfile.TemporaryDirectory(prefix='perf_', dir=G.WORK) as tmp:
        wd = Path(tmp)
        c = G.G2.compile_pruv(compiler, str(G.HERE / 'perf/pump.pruv'), str(wd), backend)
        if c['rc']:
            return dict(c, status='pruv_error')
        exe, error = G.build(c, wd, (G.HERE / 'perf/driver.rs').read_text())
        if error:
            return {'status': 'compile_error', 'error': error}
        modes = {}
        for mode, name in ((0, 'per_byte'), (1, 'bulk_copy')):
            r = G.run([exe, str(mode), str(repeat)], timeout=600, env=env)
            if r['rc']:
                return dict(r, status='run_error', mode=name)
            data = json.loads(r['stdout'])
            p, native = data['pruv_ns'], data['rust_ns']
            mem = {side: peak(exe, mode, side, env) for side in ('pruv', 'rust')}
            if any(m['rc'] for m in mem.values()):
                return {'status': 'run_error', 'mode': name, 'memory': mem}
            modes[name] = {'bytes': data['total'], 'pruv_median_ms': p / 1e6, 'rust_median_ms': native / 1e6,
                           'ratio': p / native, 'pruv_max_rss_bytes': mem['pruv']['max_rss_bytes'],
                           'rust_max_rss_bytes': mem['rust']['max_rss_bytes'], 'raw': data}
        return {'status': 'ok', 'modes': modes}


def main():
    p = argparse.ArgumentParser(__doc__)
    p.add_argument('out', type=Path, nargs='?')
    p.add_argument('--compiler', choices=['py', 'rs'], default='py')
    p.add_argument('--backend', choices=['rust', 'llvm', 'both'], default='both')
    p.add_argument('--repeat', type=int, default=20)
    a = p.parse_args()
    if a.repeat < 1:
        p.error('--repeat must be positive')
    rows = {b: measure(a.compiler, b, a.repeat) for b in
            (['rust', 'llvm'] if a.backend == 'both' else [a.backend])}
    result = {'compiler': a.compiler, 'verified': True, 'repeat': a.repeat, 'bytes_per_run': N,
              'tools': G.G2.COMPILERS, 'platform': platform.platform(),
              'rustc': G.run([G.G2.RUSTC, '--version']), 'method': __doc__.strip(), 'rows': rows}
    out = a.out or G.HERE / 'results' / f'perf_{a.compiler}.json'
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(result, indent=2))
    for backend, r in rows.items():
        if r['status'] != 'ok':
            print(a.compiler, backend, json.dumps(r)[:600])
            continue
        for name, m in r['modes'].items():
            print(a.compiler, backend, name, json.dumps({k: v for k, v in m.items() if k != 'raw'}))
    return int(any(r['status'] != 'ok' for r in rows.values()))


if __name__ == '__main__':
    sys.exit(main())
