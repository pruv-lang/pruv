"""Shared helpers of the Codex import tests: fake binary, case compilation, runners, comparison."""
import sys
sys.dont_write_bytecode = True
import json
import os
import shutil
import stat
import subprocess
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
WORK = HERE / 'work'
NODE = os.environ.get('CODEX_NODE', os.environ.get('MCP_NODE', '/opt/homebrew/bin/node'))
sys.path.insert(0, str(HERE))
from q_server import expand  # noqa: E402  (macro expansion shared with the fake server)

DEFAULT_META = {'id': 't-1', 'cwd': '/work/proj'}


def fake_binary():
    """Executable wrapper that runs q_server.py (argv after `app-server --stdio` is [scenario, log])."""
    WORK.mkdir(parents=True, exist_ok=True)
    p = WORK / 'fakecodex'
    p.write_text("#!/bin/sh\nexec '%s' -B '%s' \"$@\"\n" % (sys.executable, HERE / 'q_server.py'))
    p.chmod(p.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    return str(p)


def compile_case(c):
    """Scenario of a case (see fixtures/README in README.md): explicit `scenario`, else built from list/meta/pages."""
    if 'scenario' in c:
        return c['scenario']
    m = {}
    if 'init' in c:
        m['initialize'] = c['init']
    if c['mode'] == 'list':
        m['thread/list'] = c['list_actions'] if 'list_actions' in c else [{'result': c.get('list', {})}]
    else:
        m['thread/read'] = c['read_actions'] if 'read_actions' in c else [{'result': {'thread': c.get('meta', DEFAULT_META)}}]
        if 'pages_actions' in c:
            m['thread/turns/list'] = c['pages_actions']
        elif 'pages' in c:
            m['thread/turns/list'] = [{'result': p} for p in c['pages']]
    return {'methods': m}


def request_for(case, side, index, binary):
    """JSON request for the TS driver / the Pruv runner; `side` keeps the logs apart."""
    d = WORK / 'cases' / side
    d.mkdir(parents=True, exist_ok=True)
    scenario = d / f'{index}.scenario.json'
    log = d / f'{index}.log'
    if log.exists():
        log.unlink()
    scenario.write_text(json.dumps(compile_case(case)))
    return {
        'mode': case['mode'], 'binary': case.get('binary') or binary, 'args': [str(scenario), str(log)],
        'cwd': str(HERE), 'folder': expand(case.get('folder', '/work/proj')), 'thread': expand(case.get('thread', 't-1')),
        'timeout': case.get('timeout', 20000),
    }


def read_log(index, side):
    p = WORK / 'cases' / side / f'{index}.log'
    if not p.exists():
        return None
    return [json.loads(x) for x in p.read_text(encoding='utf-8').split('\n') if x.strip()]


def run_lines(cmd, requests, cwd=HERE, timeout=None):
    """One request per stdin line, one JSON result per stdout line."""
    p = subprocess.run([str(x) for x in cmd], input=''.join(json.dumps(r) + '\n' for r in requests), text=True,
                       capture_output=True, cwd=cwd, timeout=timeout or max(120, len(requests) * 8))
    if p.returncode:
        raise RuntimeError(f'{cmd[0]}: exit {p.returncode}\n{p.stderr[-4000:]}')
    # split on newline only: str.splitlines also breaks at U+2028/U+2029/U+0085, which JSON leaves unescaped
    rows = [json.loads(s) for s in p.stdout.split('\n') if s]
    if len(rows) != len(requests):
        raise RuntimeError(f'{len(rows)} results for {len(requests)} requests\n{p.stderr[-2000:]}')
    return rows


def node_command():
    return [NODE, '--experimental-strip-types', '--import', HERE / 'reference/hooks.mjs', HERE / 'reference.mjs']


def calls_of(log):
    """The non-handshake requests the client sent, as (method, params)."""
    out = []
    for r in log or []:
        m = r.get('recv')
        if isinstance(m, dict) and 'method' in m and 'id' in m and m['method'] != 'initialize':
            out.append((m['method'], m.get('params')))
    return out


def violations_of(log):
    return [r['violation'] for r in log or [] if 'violation' in r]


def subset(want, got):
    if isinstance(want, dict):
        return isinstance(got, dict) and all(k in got and subset(v, got[k]) for k, v in want.items())
    if isinstance(want, list):
        return isinstance(got, list) and len(want) == len(got) and all(subset(a, b) for a, b in zip(want, got))
    return want == got


def check_case(case, result, log, other_result=None, other_log=None):
    """Problems of one case on one side; empty when it passes. `other_*` is the TS side for comparison."""
    problems = []
    if 'expected' in case:
        want = expand(case['expected'])
        if result != want:
            problems.append(f'result {json.dumps(result)[:300]} != expected {json.dumps(want)[:300]}')
    if other_result is not None and result != other_result:
        problems.append(f'result {json.dumps(result)[:300]} != TS {json.dumps(other_result)[:300]}')
    if case.get('expect_no_spawn'):
        if log is not None:
            problems.append('child was spawned')
        return problems
    if not case.get('allow_violations'):
        v = violations_of(log)
        if v:
            problems.append('protocol violations: ' + '; '.join(v[:3]))
    if other_log is not None and log != other_log:
        problems.append('request log differs from TS')
    calls = calls_of(log)
    if 'expect_calls' in case:
        want = case['expect_calls']
        if len(calls) != len(want) or not all(m == w['method'] and subset(w.get('params', {}), p) for (m, p), w in zip(calls, want)):
            problems.append(f'calls {json.dumps(calls)[:300]} != {json.dumps(want)[:300]}')
    for method, n in case.get('expect_count', {}).items():
        got = sum(1 for m, _ in calls if m == method)
        if got != n:
            problems.append(f'{got} {method} calls, expected {n}')
    for msg in case.get('expect_recv', []):
        if not any(r.get('recv') == msg for r in log or []):
            problems.append(f'client never sent {json.dumps(msg)}')
    return problems


def load_cases():
    return json.loads((HERE / 'fixtures/cases.json').read_text(encoding='utf-8'))['cases']
