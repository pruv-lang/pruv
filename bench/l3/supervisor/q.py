#!/usr/bin/env python3
"""L3-2 Q differential: the Pruv supervisor (four pairs) against the TS reference on Node, plus write-failure
injection (docs/COMMAND_SUPERVISOR_SPEC.md, "Q reference and oracle").

Fixtures are the A3 fixtures (test_a3.py) that the TS supervisor can also run: every case except bad input and the
control-connect failure, which have no TS counterpart. The Pruv packet is mapped through the host rules (negative
status to a signal name, stopped 1/2 to 'stop'/'disconnect', file_open to fileBytes presence) and compared with the
TS `exit` message field by field. Only the policies P1-P7 may differ; each allowed difference is named.
Writes results/q_diff.json. Exit 1 on an unexplained difference.
"""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys

sys.dont_write_bytecode = True
import common  # noqa: E402
import test_a3  # noqa: E402
from checker import parse_packet  # noqa: E402

HERE = common.HERE
WORK = HERE / 'work' / 'q'
REF = HERE / 'reference' / 'drive.mjs'
NODE = os.environ.get('NODE', 'node')


def a3_cases():
    got = []
    test_a3.run_lines = lambda name, root, harness, cases, **kw: got.extend(cases) or 0
    test_a3.main()
    out = []
    for label, line, _ in got:
        w = line.split(' ')
        dec = lambda x: '' if x == '-' else bytes.fromhex(x).decode()  # noqa: E731
        c = dict(label=label, mode=w[0], delay=int(w[1]), shell=dec(w[2]), argv=dec(w[3]), cwd=dec(w[4]),
                 out=dec(w[5]), limit=int(w[6]), tail=int(w[7]), line=line)
        if c['mode'] in ('noconn', 'zero') or label in ('tail 0', 'tail too big', 'limit negative', 'limit too big',
                                                          'argv no nul'):
            continue
        c['args'] = c['argv'].split('\0')[:-1]
        out.append(c)
    return out


def summary(path):
    try:
        return test_a3.summary(Path(path).read_bytes())
    except OSError:
        return '-'


def wrap(cmd, fsize):
    """Run cmd with SIGXFSZ ignored and RLIMIT_FSIZE = fsize (inherited across exec), or as is."""
    if fsize is None:
        return cmd
    pre = ('import os,resource,signal,sys; signal.signal(signal.SIGXFSZ, signal.SIG_IGN); '
           f'resource.setrlimit(resource.RLIMIT_FSIZE, ({fsize}, {fsize})); os.execvp(sys.argv[1], sys.argv[1:])')
    return [sys.executable, '-c', pre] + cmd


def run_pruv(exe, cases, fsize=None):
    payload = ''.join(c['line'] + '\n' for c in cases)
    r = subprocess.run(wrap([str(exe)], fsize), input=payload, capture_output=True, text=True, timeout=900)
    lines = r.stdout.splitlines()
    if r.returncode != 0 or len(lines) != len(cases):
        raise RuntimeError(f'pruv runner rc={r.returncode}: {r.stderr[-500:]}')
    res = []
    for ln in lines:
        p, started, summ = ln.split(' | ')
        res.append(dict(packet=[int(x) for x in p.split()], started=int(started), file=summ))
    return res


def run_ts(cases, fsize=None):
    ts_cases = []
    for c in cases:
        out = c['out'] + '.ts'
        try:
            os.unlink(out)
        except OSError:
            pass
        ts_cases.append(dict(shell=c['shell'], args=c['args'], cwd=c['cwd'], out=out, limit=c['limit'],
                             tail=c['tail'], mode=c['mode'], delay=c['delay']))
    cf, rf = WORK / 'ts_cases.json', WORK / 'ts_results.json'
    cf.write_text(json.dumps(ts_cases))
    r = subprocess.run(wrap([NODE, str(REF), str(cf), str(rf)], fsize), capture_output=True, text=True,
                       timeout=1800)
    if r.returncode != 0:
        raise RuntimeError(f'TS driver rc={r.returncode}: {r.stderr[-500:]}')
    res = json.loads(rf.read_text())
    for c, x in zip(ts_cases, res):
        x['file'] = summary(c['out'])
    return res


def from_pruv(p):
    """The packet through the host rules, in the TS exit message's terms."""
    h, tail = p[:12], bytes(p[12:])
    if h[1] == 2:
        return dict(spawnError=True)
    if h[1] != 1:
        return dict(kind=h[1])
    code = signame = None
    if h[3] == 1:
        if h[2] >= 0:
            code = h[2]
        else:
            signame = signal.Signals(-h[2]).name
    return dict(code=code, signal=signame, stopped={0: None, 1: 'stop', 2: 'disconnect'}[h[4]],
                truncated=bool(h[6]), bytes=h[9], fileBytes=h[10] if h[8] else None, fileError=h[7] != 0,
                tail=tail, reaped=bool(h[5]))


def from_ts(m):
    if m is None:
        return None
    if 'spawnError' in m:
        return dict(spawnError=True)
    return dict(code=m.get('code'), signal=m.get('signal'), stopped=m.get('stopped'), truncated=m['truncated'],
                bytes=m['bytes'], fileBytes=m.get('fileBytes'), fileError=m.get('fileError') is not None,
                tail=m['tail'], reaped=m['reaped'])


def compare(c, pr, ts):
    """Return (status, notes): status 'same', 'allowed' or 'DIFF'."""
    notes, allowed = [], []
    parse_packet(pr['packet'], c['tail']) if pr['packet'][0] == 0 else None
    a = from_pruv(pr['packet'])
    if pr['file'] != ts['file']:
        if c['label'].startswith('fault') and a.get('fileError'):
            allowed.append('P3 file has the bytes of the partial write')
        else:
            notes.append(f"file {pr['file']} vs {ts['file']}")
    if pr['started'] != int(ts['started']):
        notes.append(f"started {pr['started']} vs {ts['started']}")
    b = from_ts(ts['exit'])
    if b is None:
        # Disconnect: the TS exit message cannot reach a disconnected parent; the file is the comparison.
        if c['mode'] != 'close':
            notes.append('TS sent no exit message')
        elif a.get('stopped') != 'disconnect':
            notes.append(f"pruv stopped {a.get('stopped')}")
        else:
            allowed.append('disconnect: no TS exit message, file compared')
        return ('DIFF' if notes else 'allowed' if allowed else 'same'), notes + allowed
    if 'tail' in b:
        t = b['tail']
        if t and '\udc00' <= t[0] <= '\udfff':
            t = t[1:]
            allowed.append('P5 lone low surrogate dropped')
        b['tail'] = t.encode('utf-8', 'surrogatepass')
    if b.get('stopped') is not None and a.get('code') is None and a.get('signal') is None:
        allowed.append(f"P1 stop is a kill (TS {b['code']}/{b['signal']}, Pruv status unknown)")
        b['code'] = b['signal'] = None
    if c['label'].startswith('fault') and a.get('fileError') and b.get('fileError') and \
            a.get('fileBytes') != b.get('fileBytes'):
        allowed.append(f"P3 whole writes (fileBytes {a['fileBytes']} vs {b['fileBytes']})")
        b['fileBytes'] = a['fileBytes']
    for k in sorted(set(a) | set(b)):
        if a.get(k) != b.get(k):
            notes.append(f'{k}: pruv {a.get(k)!r} vs ts {b.get(k)!r}'[:300])
    return ('DIFF' if notes else 'allowed' if allowed else 'same'), notes + allowed


def fault_cases():
    """Write failure: three 4096-byte chunks 0.2 s apart under RLIMIT_FSIZE 6000. The second write is cut at
    6000 bytes and the third is not attempted."""
    code = ('import sys,time\nfor i in range(3):\n sys.stdout.buffer.write(bytes([65+i])*4096); '
            'sys.stdout.flush(); time.sleep(0.2)')
    out = str(WORK / 'fault0.bin')
    line = ' '.join(['wait', '0', test_a3.hx(sys.executable), test_a3.hx(test_a3.blob(['-c', code])), '-',
                     test_a3.hx(out), str(1 << 20), '16'])
    return [dict(label='fault write', mode='wait', delay=0, shell=sys.executable, args=['-c', code], cwd='',
                 out=out, limit=1 << 20, tail=16, line=line)]


def main():
    WORK.mkdir(parents=True, exist_ok=True)
    cases = a3_cases()
    faults = fault_cases()
    g2, run = common.configure(WORK)
    report = dict(cases=len(cases) + len(faults), pairs={}, ts={})
    ts = run_ts(cases)
    ts_fault = run_ts(faults, fsize=6000)
    bad = 0
    for compiler, backend in common.PAIRS:
        key = f'{compiler}/{backend}'
        exe = common.build(g2, run, compiler, backend, HERE / 'supervise.pruv', WORK / f'{compiler}_{backend}',
                           test_a3.HARNESS, 'q', common.LIB_FLAGS)
        rows = []
        for group, tsr, fs in ((cases, ts, None), (faults, ts_fault, 6000)):
            pr = run_pruv(exe, group, fs)
            for c, p, t in zip(group, pr, tsr):
                status, notes = compare(c, p, t)
                rows.append(dict(label=c['label'], status=status, notes=notes))
                if status == 'DIFF':
                    bad += 1
                    print(f'{key}: {c["label"]}: DIFF {notes}')
        counts = {s: sum(r['status'] == s for r in rows) for s in ('same', 'allowed', 'DIFF')}
        print(f'{key}: {counts}', flush=True)
        report['pairs'][key] = dict(counts=counts, rows=rows)
    (HERE / 'results').mkdir(exist_ok=True)
    (HERE / 'results' / 'q_diff.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    return int(bad > 0)


if __name__ == '__main__':
    sys.exit(main())
