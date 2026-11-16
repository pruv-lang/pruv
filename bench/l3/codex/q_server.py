#!/usr/bin/env python3
"""Fake `codex app-server --stdio` driven by a scenario file (never a real Codex, no network).

    q_server.py app-server --stdio SCENARIO.json LOG.jsonl

The oracle (TS) and the Pruv hosts spawn the same fake through a wrapper script, so the extra argv after
`app-server --stdio` is [SCENARIO, LOG]. Every line the client sends is recorded in LOG as {"recv": <parsed>}.
Protocol violations are recorded as {"violation": "..."}: first request is initialize with id 1 and the exact
params, `initialized` (params {}) arrives before any other request, request ids count 1,2,3..., and every
request's params have exactly the documented shape.

Scenario: {"methods": {"<method>": [ACTION, ...]}}. The k-th request of a method uses the k-th action (the last
one repeats). Missing method -> error -32601. ACTION keys:
  result / error      response body (default result {})
  id                  response id override (wrong id); also_correct sends the correct response afterwards
  idtoken             raw JSON token used as the response id (e.g. "1.0")
  pre / post          messages (objects or raw strings) sent before / after the response
  raw / rawhex        send this text / these bytes (+LF) instead of a response
  stall               never answer (keep reading stdin until EOF)
  exit                exit with this code before answering (stderr text from "stderr")
  oversize            write N bytes without a newline, then stall
  await               number of client lines to read and log after `pre`, before the response
  delay               seconds to sleep before answering
  utf8                send raw UTF-8 instead of ASCII escapes
Macros inside results: {"$str": "ab", "n": 3} -> "ababab"; {"$list": ITEM, "n": N} -> N items where the string "$i"
becomes the integer index and "{i}" inside strings becomes the index.
"""
import copy
import json
import os
import sys
import time

INIT_PARAMS = {
    'clientInfo': {'name': 'lair', 'title': 'Lair', 'version': '0.0.1'},
    'capabilities': {'experimentalApi': True, 'requestAttestation': False},
}
SOURCE_KINDS = ['cli', 'vscode', 'exec', 'appServer']


def expand(v, i=None):
    if isinstance(v, dict):
        if '$str' in v:
            return v['$str'] * v.get('n', 1)
        if '$list' in v:
            idx = [v.get('base', 0) + k for k in range(v['n'])]
            if v.get('desc'):
                idx.reverse()
            return [expand(v['$list'], k) for k in idx]
        if '$join' in v:
            return v.get('sep', '').join(str(x) for x in expand(v['$join'], i))
        if '$concat' in v:
            out = []
            for part in v['$concat']:
                out.extend(expand(part, i))
            return out
        if '$cat' in v:
            return ''.join(expand(x, i) for x in v['$cat'])
        return {k: expand(x, i) for k, x in v.items()}
    if isinstance(v, list):
        return [expand(x, i) for x in v]
    if isinstance(v, str) and i is not None:
        if v == '$i':
            return i
        return v.replace('{i}', str(i))
    return v


def main():
    argv = sys.argv[1:]
    if argv[:2] != ['app-server', '--stdio'] or len(argv) < 4:
        sys.stderr.write('usage: q_server.py app-server --stdio SCENARIO LOG\n')
        os._exit(99)
    with open(argv[2], encoding='utf-8') as f:
        scenario = json.load(f)
    log = open(argv[3], 'w', encoding='utf-8')
    out = sys.stdout.buffer

    def rec(o):
        log.write(json.dumps(o, ensure_ascii=True) + '\n')
        log.flush()

    def violation(text):
        rec({'violation': text})

    def write(b):
        try:
            out.write(b)
            out.flush()
        except (BrokenPipeError, ValueError):
            os._exit(0)

    def send(o, utf8=False):
        if isinstance(o, str):
            write(o.encode('utf-8') + b'\n')
        else:
            write(json.dumps(expand(o), ensure_ascii=not utf8).encode('utf-8') + b'\n')

    methods = scenario.get('methods', {})
    used = {}
    expect_id = 1
    seen_first = False
    initialized = False

    def check_params(method, params):
        if method == 'initialize':
            if params != INIT_PARAMS:
                violation('initialize params ' + json.dumps(params))
        elif method == 'thread/list':
            want_keys = {'cwd', 'limit', 'sortKey', 'useStateDbOnly', 'sourceKinds'}
            if not isinstance(params, dict) or set(params) != want_keys or params['limit'] != 50 \
                    or params['sortKey'] != 'updated_at' or params['useStateDbOnly'] is not True \
                    or params['sourceKinds'] != SOURCE_KINDS or not isinstance(params['cwd'], list) or len(params['cwd']) != 1:
                violation('thread/list params ' + json.dumps(params))
        elif method == 'thread/read':
            if params != {'threadId': params.get('threadId'), 'includeTurns': False} or not isinstance(params.get('threadId'), str):
                violation('thread/read params ' + json.dumps(params))
        elif method == 'thread/turns/list':
            want = {'threadId', 'limit', 'sortDirection', 'itemsView'}
            keys = set(params)
            if keys not in (want, want | {'cursor'}) or params['limit'] != 20 or params['sortDirection'] != 'desc' \
                    or params['itemsView'] != 'summary' or not isinstance(params['threadId'], str):
                violation('thread/turns/list params ' + json.dumps(params))
            if 'cursor' in params and (not isinstance(params['cursor'], str) or params['cursor'] == ''):
                violation('cursor is not a nonempty string')
        else:
            violation('unexpected method ' + method)

    def perform(act, mid):
        utf8 = bool(act.get('utf8'))
        if act.get('stderr'):
            sys.stderr.write(expand(act['stderr']))
            sys.stderr.flush()
        for m in act.get('pre', []):
            send(m, utf8)
        for _ in range(act.get('await', 0)):
            # read the client's replies to `pre` before answering, so a client that closes right after the
            # response cannot race the log
            got = sys.stdin.buffer.readline()
            if not got:
                break
            try:
                rec({'recv': json.loads(got.decode('utf-8'))})
            except Exception:
                violation('client sent invalid JSON')
        if act.get('delay'):
            time.sleep(act['delay'])
        if act.get('exit') is not None:
            os._exit(act['exit'])
        if act.get('stall'):
            return
        if 'oversize' in act:
            write(b'a' * act['oversize'])
            return
        if 'rawhex' in act:
            write(bytes.fromhex(act['rawhex']) + b'\n')
        elif 'raw' in act:
            write(act['raw'].encode('utf-8') + b'\n')
        elif 'idtoken' in act:
            body = ('"error":' + json.dumps(expand(act['error']))) if 'error' in act else ('"result":' + json.dumps(expand(act.get('result', {}))))
            write(('{"id":' + act['idtoken'] + ',' + body + '}').encode('utf-8') + b'\n')
        else:
            resp = {'id': act.get('id', mid)}
            if 'error' in act:
                resp['error'] = act['error']
            else:
                resp['result'] = act.get('result', {})
            send(resp, utf8)
            if act.get('also_correct'):
                good = {'id': mid}
                if 'error' in act:
                    good['error'] = act['error']
                else:
                    good['result'] = act.get('result', {})
                send(good, utf8)
        for m in act.get('post', []):
            send(m, utf8)

    for raw in sys.stdin.buffer:
        try:
            msg = json.loads(raw.decode('utf-8'))
        except Exception:
            violation('client sent invalid JSON')
            continue
        rec({'recv': msg})
        if not isinstance(msg, dict) or 'method' not in msg:
            continue
        method = msg['method']
        if 'id' not in msg:
            if method == 'initialized':
                if msg.get('params') != {}:
                    violation('initialized params')
                if not seen_first:
                    violation('initialized before initialize')
                initialized = True
            else:
                violation('unexpected notification ' + method)
            continue
        mid = msg['id']
        if not seen_first and method != 'initialize':
            violation('first request is not initialize')
        seen_first = True
        if mid != expect_id:
            violation('request id %r expected %d' % (mid, expect_id))
        expect_id += 1
        if method != 'initialize' and not initialized:
            violation('request before initialized')
        check_params(method, msg.get('params'))
        lst = methods.get(method)
        if lst is not None and not isinstance(lst, list):
            lst = expand(lst)
        if lst is None and method == 'initialize':
            lst = [{'result': {'userAgent': 'fake'}}]
        if not lst:
            send({'id': mid, 'error': {'code': -32601, 'message': 'Method not found: ' + method}})
            continue
        k = used.get(method, 0)
        used[method] = k + 1
        perform(copy.deepcopy(lst[min(k, len(lst) - 1)]), mid)


if __name__ == '__main__':
    main()
