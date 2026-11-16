#!/usr/bin/env python3
"""Fake OpenCode exporter (never a real OpenCode, no network, no account).

The importer spawns it as `<exe> --pure export <thread>` (argv exactly), cwd = the case folder, stdin and stderr ignored,
fd 1 = an already open regular file. The scenario and the log are SIBLING FILES of the executable path (argv[0], never
resolved, so a symlink or a copy of this script both work; common.place_exporter rewrites the shebang to an absolute python):

    <exe>.scenario.json   input
    <exe>.out             raw bytes written through fd 1 (absent = empty output)
    <exe>.log             JSON transcript, rewritten after every event (a killed process still leaves one)

Scenario: {"expect": {...}, "mode": "output"|"signal"|"hang", "exit": N, "signal": N, "term": "exit"|"ignore", "termExit": N,
           "lifetime": SECONDS}
  expect.argv      default ["--pure", "export", expect.thread]
  expect.thread    the thread id argument (argv[3])
  expect.cwd       the child's cwd (compared by realpath)
  expect.env       {NAME: VALUE | null}: null means the variable must be absent, a string means exactly that value
  expect.detached  true (default): the child is a session/process-group leader (setsid, like Node detached:true)
  mode output      write the bytes, then exit with `exit` (default 0; nonzero = failed export)
  mode signal      write the bytes, then kill itself with `signal` (default 15) under the default disposition
  mode hang        write the bytes (unless "write": false), then wait; term "exit" (default): exit `termExit` (default 0)
                   on SIGTERM; term "ignore": ignore SIGTERM until SIGKILL
  lifetime         SIGALRM kills a leaked fake after this many seconds (default 120)

Violations (strings in log["violations"]): argv, cwd, env presence/absence, fd 1 not a regular file, stdin/stderr not
/dev/null, not a group leader. The fake writes nothing to stdout except the export bytes (never log text).
"""
import json
import os
import signal
import stat
import sys
import time

EXE = os.path.abspath(sys.argv[0])
LOG = EXE + '.log'
T = {'argv': sys.argv[1:], 'cwd': os.getcwd(), 'pid': os.getpid(), 'violations': [], 'notes': [], 'fd1': None, 'exit': None, 'written': 0}


def flush():
    tmp = LOG + '.tmp'
    with open(tmp, 'w', encoding='utf-8') as f:
        json.dump(T, f, ensure_ascii=True)
    os.replace(tmp, LOG)


def violation(text):
    T['violations'].append(text)
    flush()


def note(text):
    T['notes'].append(text)
    flush()


def finish(code, reason):
    T['exit'] = reason
    flush()
    os._exit(code)


def is_devnull(fd):
    try:
        a = os.fstat(fd)
        b = os.stat('/dev/null')
    except OSError:
        return False
    return stat.S_ISCHR(a.st_mode) and a.st_rdev == b.st_rdev


def checks(scn):
    exp = scn.get('expect', {})
    want = exp.get('argv')
    if want is None:
        want = ['--pure', 'export', exp['thread']] if 'thread' in exp else None
    if want is not None and T['argv'] != want:
        violation('argv %s != %s' % (json.dumps(T['argv']), json.dumps(want)))
    if len(T['argv']) != 3 or T['argv'][:2] != ['--pure', 'export']:
        violation('argv shape %s' % json.dumps(T['argv']))
    if 'cwd' in exp and os.path.realpath(exp['cwd']) != os.path.realpath(os.getcwd()):
        violation('cwd %s != %s' % (os.getcwd(), exp['cwd']))
    for name, value in exp.get('env', {}).items():
        have = os.environ.get(name)
        if value is None:
            if have is not None:
                violation('env %s is present but must be absent' % name)
        elif have != value:
            violation('env %s = %r, expected %r' % (name, have, value))
    try:
        st = os.fstat(1)
        T['fd1'] = {'regular': stat.S_ISREG(st.st_mode), 'mode': stat.S_IMODE(st.st_mode), 'size': st.st_size}
        if not stat.S_ISREG(st.st_mode):
            violation('fd 1 is not a regular file (mode %o)' % st.st_mode)
    except OSError as e:
        violation('fstat(1) failed: %s' % e)
    if not is_devnull(0):
        violation('stdin is not /dev/null')
    if not is_devnull(2):
        violation('stderr is not /dev/null')
    if exp.get('detached', True) and not (os.getsid(0) == os.getpid() and os.getpgrp() == os.getpid()):
        violation('not a session/process-group leader (detached spawn expected)')


def write_output():
    try:
        with open(EXE + '.out', 'rb') as f:
            data = f.read()
    except FileNotFoundError:
        data = b''
    view = memoryview(data)
    try:
        while view:
            n = os.write(1, view[:1 << 20])
            T['written'] += n
            view = view[n:]
    except OSError as e:
        violation('write to fd 1 failed: %s' % e)
    note('output written: %d bytes' % T['written'])


def main():
    with open(EXE + '.scenario.json', encoding='utf-8') as f:
        scn = json.load(f)
    flush()
    signal.alarm(int(scn.get('lifetime', 120)))
    mode = scn.get('mode', 'output')
    term = scn.get('term', 'exit')
    if mode == 'hang':
        if term == 'ignore':
            signal.signal(signal.SIGTERM, signal.SIG_IGN)
        else:
            def on_term(*_):
                note('SIGTERM received')
                finish(int(scn.get('termExit', 0)), 'SIGTERM')
            signal.signal(signal.SIGTERM, on_term)
    checks(scn)
    if mode != 'hang' or scn.get('write', True):
        write_output()
    if mode == 'output':
        finish(int(scn.get('exit', 0)), 'exit')
    if mode == 'signal':
        sig = int(scn.get('signal', 15))
        T['exit'] = 'self-signal %d' % sig
        flush()
        signal.signal(sig, signal.SIG_DFL)
        os.kill(os.getpid(), sig)
        time.sleep(30)
        finish(70, 'signal did not terminate')
    note('hanging (term=%s)' % term)
    while True:
        time.sleep(0.05)


if __name__ == '__main__':
    main()
