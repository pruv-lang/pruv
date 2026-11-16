"""L3-8 forwarding integration I01-I10 (DESIGN section 8).

For every route the piece's own built runner (work/rs_rust/runner of bench/l3/<piece>, built by that piece's test_impl.py) is run
once on a fixture of that piece in its exact domain, with a fake child / fake exporter / fake app-server and a throwaway home:
no account, token or network. The captured rows or record become the entry's engine result through the case's `forward` key
(host.rs parse_script), and the entry runs once on rs/rust (work/rs_rust/harness). Asserted:
  - exactly one invocation (one engine trace entry) with the right route, op and arguments;
  - list: rows returned in order, each `keep` (same object) or `copy_title` with the title cut to 200 UTF-16 units, nothing else;
  - read: the same record ref comes back;
  - errors: the piece's error object is forwarded to the classifier unchanged (throw_existing with the same ref).
A route whose runner cannot produce the result this way is reported as an integration limit (claude), never stubbed.

  python3 integrate.py [--case I03] [--list]       exit 0 all asserted, 1 an assertion failed, 2 only limits / entry not ready
"""
import sys
sys.dont_write_bytecode = True
import argparse
import json
import shutil
import subprocess
import tempfile
from pathlib import Path

from common import FIXTURES, HERE, host_path, load_defaults, merge_case

BENCH = HERE.parent
ROUTES = ['claude', 'qwen', 'codex', 'grok', 'opencode']

# (list fixture, list error fixture, read fixture, read error fixture) of each piece, by the piece's own fixture names.
PICK = {
    'qwen': {'list': 'fixtures/normal.jsonl in a throwaway home (runner --list home /project)', 'read': 'fixtures/normal.jsonl (runner <path>)',
             'list_error': None, 'read_error': None},
    'codex': {'list': 'list_basic', 'read': 'read_basic', 'list_error': 'list_error', 'read_error': 'read_meta_error'},
    'grok': {'list': 'listing-mixed-missing-3', 'read': 'budgets-200-turns-no-omit', 'list_error': 'paths-empty-folder', 'read_error': 'window-history-is-directory'},
    'opencode': {'list': 'F04-limit-49', 'read': 'F06-extra-slashes', 'list_error': 'F03-count-prepare-empty-error', 'read_error': 'F06-invalid-thread'},
}
LIMITS = {
    'claude': 'bench/l3/claude is the pure packet API (head_packet / tail_packet / window_packet of a0..a4_root.pruv, runner work/a4/rs_rust/runner '
              'reads "op flag|bytes" lines and prints packet words). It has no listing or read driver that returns import rows or an '
              'ExternalRecord, no project/home walk and no JSON result, so there is nothing to register as the entry engine result.',
}
ASSERTS = ['exactly one engine invocation', 'route and args', 'list rows unchanged except the 200-unit title cut, order and metadata kept',
           'read returns the same record', 'piece error forwarded unchanged']


def fixture_for(route, op):
    """The I-group descriptor of fixtures/I.json (read back by run())."""
    d = {'route': route, 'op': op, 'piece': f'bench/l3/{route}', 'asserts': ASSERTS}
    if route in LIMITS:
        d['limit'] = LIMITS[route]
    else:
        d['driver'] = f'bench/l3/{route}/work/rs_rust/runner'
        d['picks'] = {k: v for k, v in PICK[route].items() if k.startswith(op) and v}
        d['no_error_fixture'] = [k for k, v in PICK[route].items() if k.startswith(op) and not v]
    return d


# ---- capture: one run of the piece's own runner ------------------------------------------------------------------
GROK_OPENCODE = r'''
import sys, json
sys.path.insert(0, %(piece)r)
import test_impl as t
r = t.load_fixtures()
cases = r[0] if isinstance(r, tuple) else r
case = [c for c in cases if c['name'] == %(name)r][0]
run = t.run_pair(case, 'entry_int', %(exe)r)
f = case.get('folder'); th = case.get('thread')
print(json.dumps({'result': run['result'], 'returncode': run['returncode'], 'folder': f if isinstance(f, str) else None, 'thread': th if isinstance(th, str) else None}))
'''

CODEX = r'''
import sys, json
sys.path.insert(0, %(piece)r)
import common as c
cases = json.load(open(%(piece)r + '/fixtures/cases.json'))['cases']
case = [x for x in cases if x['name'] == %(name)r][0]
req = c.request_for(case, 'entry_int', 0, c.fake_binary())
res = c.run_lines([%(exe)r], [req])[0]
print(json.dumps({'result': res, 'returncode': 0, 'folder': req['folder'], 'thread': req['thread'] if case['mode'] == 'read' else None}))
'''


def run_py(code, piece, **kw):
    p = subprocess.run([sys.executable, '-c', code % dict(kw, piece=str(piece))], capture_output=True, text=True, cwd=piece, timeout=600)
    if p.returncode:
        raise RuntimeError(f'{piece.name} driver: exit {p.returncode}\n{p.stdout[-800:]}\n{p.stderr[-1500:]}')
    return json.loads(p.stdout.strip().split('\n')[-1])


def capture_qwen(mode):
    piece = BENCH / 'qwen'
    exe = piece / 'work/rs_rust/runner'
    src = piece / 'fixtures/normal.jsonl'
    with tempfile.TemporaryDirectory(prefix='entry-int-qwen-') as tmp:
        home = Path(tmp) / 'home'
        chats = home / 'projects/-project/chats'
        chats.mkdir(parents=True)
        target = chats / 'thread1.jsonl'
        shutil.copy(src, target)
        import os
        os.utime(target, (1700000000, 1700000000))  # old enough: no maybeOpen, deterministic
        if mode == 'list':
            out = subprocess.run([str(exe), '--list', str(home), '/project'], capture_output=True, text=True, timeout=120)
        else:
            out = subprocess.run([str(exe), str(target)], capture_output=True, text=True, timeout=120)
    if out.returncode:
        raise RuntimeError(f'qwen runner exit {out.returncode}: {out.stderr[-800:]}')
    got = json.loads(out.stdout.strip().split('\n')[0])
    if mode == 'list':
        return {'result': got, 'returncode': 0, 'folder': '/project', 'thread': None, 'rows': got}
    return {'result': got['record'], 'returncode': 0, 'folder': '/project', 'thread': 'thread1'}


def capture(route, kind):
    """kind: list | read | list_error | read_error -> {kind: ok|error, rows|record|error, folder, thread, fixture}."""
    piece = BENCH / route
    exe = str(piece / 'work/rs_rust/runner')
    if not Path(exe).is_file():
        raise RuntimeError(f'{route}: runner artifact missing ({exe}); build it with that piece\'s test_impl.py')
    if route == 'qwen':
        got = capture_qwen(kind)
        name = PICK['qwen'][kind]
    else:
        name = PICK[route][kind]
        got = run_py(CODEX if route == 'codex' else GROK_OPENCODE, piece, name=name, exe=exe)
    res = got['result']
    out = {'fixture': name, 'folder': got.get('folder') or '/work', 'thread': got.get('thread') or 't-1'}
    base = kind.split('_')[0]
    if route == 'codex':
        if isinstance(res, dict) and 'error' in res:
            out.update(kind='error', error={'code': res['error'], 'message': res['error']})
        else:
            out.update(kind='ok', rows=res if base == 'list' else None, record=res if base == 'read' else None)
    elif route in ('grok', 'opencode'):
        if res is None or res.get('status') != 'ok':
            e = (res or {}).get('error') or {}
            out.update(kind='error', error={'code': e.get('code') or 'read-failed', 'message': e.get('message') or str(e.get('code'))})
        else:
            out.update(kind='ok', rows=res.get('rows') if base == 'list' else None, record=res if base == 'read' else None)
    else:
        out.update(kind='ok', rows=got['result'] if base == 'list' else None, record=got['result'] if base == 'read' else None)
    return out


# ---- the entry run ----------------------------------------------------------------------------------------------
def units(s):
    return len(s.encode('utf-16-le')) // 2


def cut200(s):
    return s.encode('utf-16-le')[:400].decode('utf-16-le', errors='surrogatepass')


def forward_rows(rows):
    out = []
    for i, row in enumerate(rows):
        out.append(dict({'ref': f'r{i}'}, **row))
    return out


def entry_case(cid, route, op, cap, rows=None):
    args = {'source': {'ref': 's1', 'engine': route}, 'folder': cap['folder'], 'options': {'ref': 'o1'}}
    case = {'id': cid, 'op': 'list_external' if op == 'list' else 'read_external', 'args': args}
    if op == 'read':
        args['thread'] = cap['thread']
    if cap['kind'] == 'error':
        case['forward'] = {'reject': {'class': 'EngineImportError', 'ref': 'e1', 'code': cap['error']['code'], 'message': cap['error']['message']}}
    elif op == 'list':
        case['forward'] = {'rows': forward_rows(rows if rows is not None else cap['rows'])}
    else:
        case['forward'] = {'record': 'rec'}
    return merge_case(load_defaults(), case)


def jv_text(jv):
    if isinstance(jv, str):
        return jv
    if isinstance(jv, dict) and 'u16' in jv:
        return ''.join(chr(u) for u in jv['u16'])
    return None


def check(case, route, op, cap, rows, got):
    """-> list of problems of one entry result."""
    bad = []
    tr = got.get('trace') or []
    if len(tr) != 1:
        bad.append(f'engine invoked {len(tr)} times (want exactly 1): {json.dumps(tr)[:200]}')
    else:
        t = tr[0]
        a = t[3] if len(t) > 3 else {}
        if t[:3] != ['engine', route, op] or a.get('source') != {'ref': 's1'} or a.get('options') != {'ref': 'o1'} or jv_text(a.get('folder')) != cap['folder']:
            bad.append(f'route/args differ: {json.dumps(t)[:300]}')
        if op == 'read' and jv_text(a.get('thread')) != cap['thread']:
            bad.append('read thread argument differs')
        if op == 'list' and 'thread' in a:
            bad.append('list call carries a thread')
    if cap['kind'] == 'error':
        if got.get('terminal') != 'throw_existing' or got.get('value') != {'ref': 'e1'}:
            bad.append(f'error not forwarded unchanged: {got.get("terminal")} {json.dumps(got.get("value"))[:200]}')
        return bad
    if got.get('terminal') != 'ok':
        bad.append(f'terminal {got.get("terminal")} {json.dumps(got.get("value"))[:200]}')
        return bad
    if op == 'read':
        if got.get('value') != {'ref': 'rec'}:
            bad.append(f'read record differs: {json.dumps(got.get("value"))[:200]}')
        return bad
    val = got['value']
    if not isinstance(val, list) or len(val) != len(rows):
        return bad + [f'{len(val) if isinstance(val, list) else val} rows returned for {len(rows)}']
    for i, (row, v) in enumerate(zip(rows, val)):
        title = row.get('title')
        if isinstance(title, str) and units(title) > 200:
            want = cut200(title)
            if v[0] != 'copy_title' or v[1] != {'ref': f'r{i}'} or jv_text(v[2]) != want:
                bad.append(f'row {i}: title not cut to 200 units: {json.dumps(v, ensure_ascii=False)[:160]}')
        elif v != ['keep', {'ref': f'r{i}'}]:
            bad.append(f'row {i} changed: {json.dumps(v, ensure_ascii=False)[:160]}')
    return bad


def run_entry(cases):
    exe = host_path('rs_rust')
    if not Path(exe).is_file():
        raise RuntimeError(f'entry runner {exe} missing: run test_impl.py --pairs rs/rust first')
    payload = ''.join(json.dumps(c, ensure_ascii=False).replace('\u2028', '\\u2028').replace('\u2029', '\\u2029').replace('\u0085', '\\u0085') + '\n' for c in cases)
    p = subprocess.run([str(exe)], input=payload, capture_output=True, text=True, encoding='utf-8', timeout=600)
    lines = [ln for ln in p.stdout.split('\n') if ln.strip()]
    if len(lines) != len(cases):
        raise RuntimeError(f'entry runner exit {p.returncode}: {len(lines)} results for {len(cases)} cases {p.stderr[-800:]}')
    return [json.loads(ln) for ln in lines]


def jobs_for(cid, route, op):
    """-> [(label, entry case, check args)] for one I case: the success run, the 200-unit cut variant (list) and the error run."""
    out = []
    ok = capture(route, op)
    rows = ok['rows'] if op == 'list' else None
    out.append((f'{cid} {op}', entry_case(cid, route, op, ok), ok, rows))
    if op == 'list' and rows:
        longer = [dict(r) for r in rows]
        longer[0]['title'] = (longer[0].get('title') or '') + 'x' * 205  # synthetic: one row past the 200-unit cut
        out.append((f'{cid} list+long-title', entry_case(cid + '/cut', route, op, ok, longer), ok, longer))
    key = op + '_error'
    if PICK[route].get(key):
        er = capture(route, key)
        out.append((f'{cid} {key}', entry_case(cid + '/err', route, op, er), er, None))
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--case', action='append', default=[], help='I01..I10')
    ap.add_argument('--list', action='store_true')
    args = ap.parse_args(argv)
    descs = []
    for g in json.loads((FIXTURES / 'I.json').read_text(encoding='utf-8')):
        if not args.case or g['id'] in args.case:
            descs.append(g)
    if args.list:
        for g in descs:
            print(g['id'], g['name'], g['integration'].get('limit', 'runnable')[:80])
        return 0
    failed = False
    ready = True
    limits = 0
    jobs = []
    for g in descs:
        it = g['integration']
        if 'limit' in it:
            limits += 1
            print(f'{g["id"]} {g["name"]}: INTEGRATION LIMIT: {it["limit"]}')
            continue
        try:
            jobs += [(g['id'],) + j for j in jobs_for(g['id'], it['route'], it['op'])]
        except Exception as exc:
            failed = True
            print(f'{g["id"]} {g["name"]}: DRIVER ERROR {exc}')
    if jobs:
        results = run_entry([j[2] for j in jobs])
        for (gid, label, case, cap, rows), got in zip(jobs, results):
            route, op = case['args']['source']['engine'], 'list' if case['op'] == 'list_external' else 'read'
            if got.get('terminal') == 'boundary_error' and got.get('value', {}).get('code') == 'InternalInvariant':
                ready = False
                print(f'{label}: captured from {cap["fixture"]} ({cap["kind"]}); entry core not implemented (InternalInvariant: {got.get("note")}), assertions not run')
                continue
            problems = check(case, route, op, cap, rows, got)
            print(f'{label}: ' + ('PASS ' if not problems else 'FAIL ') + f'({cap["fixture"]}, {cap["kind"]})')
            for p in problems:
                print('   ' + p)
            failed |= bool(problems)
    print(f'{len(descs)} integration cases, {limits} integration limits' + ('' if ready else ', entry core not ready'))
    if failed:
        return 1
    return 0 if ready and not limits else 2


if __name__ == '__main__':
    sys.exit(main())
