#!/usr/bin/env python3
"""Fake Grok ACP executable (never a real Grok, no network).

The importer spawns it as `<exe> agent --no-leader stdio` (argv exactly; no scenario/log paths are appended).
The scenario and the log are SIBLING FILES of the executable path:

    <exe>.scenario.json   input
    <exe>.log.json        transcript, rewritten after every event (so a killed process still leaves one)

The executable is normally a two-line shell wrapper written by common.place_acp:
    Q_ACP_EXE="$0" exec python3 -B q_server.py "$@"
`Q_ACP_EXE` (else argv[0]) names the executable; the wrapper adds nothing to argv.

Scenario: {"expect": {...}, "methods": {...}, "lifetime": SECONDS, "ignore_term": bool, "allow_no_reply": bool}
  expect.cwd         the child's cwd must be this directory (compared by realpath)
  expect.grokHome    the real GROK_HOME of the child must equal this string
  expect.env         {NAME: VALUE} parent-environment markers that must be inherited unchanged
  expect.thread / expect.folder   the session/load params
  methods            {"<method>": ACTION | [ACTION, ...]}: the k-th request of a method uses the k-th action
                     (the last repeats). Defaults: initialize -> loadSession true, session/load -> result null,
                     any other method -> error -32601.

ACTION keys (in the order they are performed):
  stderr / stderr_flood   text / number of bytes written to stderr first
  inherit_pipe            seconds: start a descendant that inherits stdout/stderr and sleeps (P7)
  pre                     messages sent before the response, one write each
  await                   number of client lines to read and log before the response
  delay                   seconds to sleep
  exit                    exit with this code before answering
  no_read                 seconds to sleep without reading stdin (blocked client write, P7)
  stall                   never answer (keep reading stdin)
  oversize                write N bytes without LF and stall
  result / error / omit_result / id / idtoken / also_correct / raw / rawhex / utf8
                          the response (omit_result: no result key at all; idtoken: raw JSON id token)
  together                items written in the SAME write as the response (P6: premature `{"id":2,...}`;
                          with p6:true the write must stay <= 512 bytes, with p12:true it must hold a pad > 8192)
  post                    items sent after the response, one write each; control items:
                          {"$sleep": S}, {"$close_stdin": true}, {"$pad": N}, {"$requests": N}, {"$exit": C}
  close_stdin             close stdin right after the response (P11); `post` then goes out without reading
  eof_no_exit             seconds: close stdout after the response and stay alive (P7)
Items: a dict message, a raw string, {"$raw": TEXT, "$reply_token": TOKEN}, {"$pad": N} (N spaces + LF),
{"$idtoken": TOKEN, ...} (raw JSON id). A dict with "method" and an id is a server request: the client must
answer with the same raw id token, error {code:-32601,message:"not handled"} and jsonrpc "2.0".
Macros as in the Codex fake: {"$str": "ab", "n": 3}, {"$list": ITEM, "n": N}.

Violations (recorded as strings in log["violations"]): argv, cwd, GROK_HOME, parent env, initialize with
id 1 and exact params, jsonrpc "2.0", LF, id tokens, session/load id 2 only after loadSession was
strictly true, an `initialized` (or any) notification, wrong replies to server requests.
"""
import copy
import json
import os
import signal
import subprocess
import sys
import time

ARGV = ['agent', '--no-leader', 'stdio']
INIT_PARAMS = {
    'protocolVersion': 1,
    'clientCapabilities': {'fs': {'readTextFile': False, 'writeTextFile': False}, 'terminal': False},
    'clientInfo': {'name': 'lair', 'title': 'Lair', 'version': '0.1.0'},
}
NOT_HANDLED = {'code': -32601, 'message': 'not handled'}


class Num(float):
    """A JSON number that keeps its raw token; it compares as the binary64 value."""

    def __new__(cls, token):
        obj = float.__new__(cls, float(token))
        obj.raw = token
        return obj


def load_raw(text):
    return json.loads(text, parse_int=Num, parse_float=Num)


def expand(v, i=None):
    """Macros inside scenario data (same as the Codex fake)."""
    if isinstance(v, dict):
        if '$str' in v:
            return v['$str'] * v.get('n', 1)
        if '$list' in v:
            idx = [v.get('base', 0) + k for k in range(v['n'])]
            return [expand(v['$list'], k) for k in idx]
        return {k: expand(x, i) for k, x in v.items()}
    if isinstance(v, list):
        return [expand(x, i) for x in v]
    if isinstance(v, str) and i is not None:
        return i if v == '$i' else v.replace('{i}', str(i))
    return v


def token_of(value):
    """Raw JSON token of a decoded id."""
    return value.raw if isinstance(value, Num) else json.dumps(value)


def finite(token):
    try:
        x = float(token)
    except ValueError:
        return False
    return x - x == 0


class Server:
    def __init__(self, exe, args):
        self.exe = exe
        with open(exe + '.scenario.json', encoding='utf-8') as f:
            self.scenario = json.load(f)
        self.log_path = exe + '.log.json'
        self.expect = self.scenario.get('expect', {})
        self.t = {'argv': args, 'cwd': os.getcwd(), 'grokHome': os.environ.get('GROK_HOME'), 'pid': os.getpid(),
                  'recv': [], 'sent': [], 'violations': [], 'notes': [], 'exit': None}
        self.stdin = sys.stdin.buffer
        self.stdout_open = True
        self.stdin_closed = False
        self.seen_initialize = False
        self.load_ok = False          # initialize was answered with loadSession strictly true (correct id)
        self.next_id = 1
        self.used = {}
        self.pending = []             # raw id tokens of server requests that still wait for a client reply
        self.flush()

    # ---- log -----------------------------------------------------------------------------------------------
    def flush(self):
        tmp = self.log_path + '.tmp'
        with open(tmp, 'w', encoding='utf-8') as f:
            json.dump(self.t, f, ensure_ascii=True)
        os.replace(tmp, self.log_path)

    def violation(self, text):
        self.t['violations'].append(text)
        self.flush()

    def note(self, text):
        self.t['notes'].append(text)
        self.flush()

    def finish(self, reason):
        self.t['exit'] = reason
        self.flush()
        os._exit(0)

    # ---- output --------------------------------------------------------------------------------------------
    def write(self, data):
        """One os.write per call (atomic up to PIPE_BUF); a closed stdout silently drops data."""
        if not self.stdout_open:
            return
        view = memoryview(data)
        try:
            while view:
                view = view[os.write(1, view):]
        except (BrokenPipeError, OSError):
            self.finish('stdout-closed')
        self.t['sent'].append({'len': len(data), 'text': data[:200].decode('utf-8', 'backslashreplace')})
        self.flush()

    def encode(self, item, utf8=False):
        """One LF-terminated line for a scenario item; server requests are remembered."""
        if isinstance(item, str):
            return item.encode('utf-8') + b'\n'
        if '$raw' in item:
            if '$reply_token' in item:
                self.pending.append(item['$reply_token'])
            return item['$raw'].encode('utf-8') + b'\n'
        if '$pad' in item:
            return b' ' * item['$pad'] + b'\n'
        obj = expand(item)
        tok = obj.pop('$idtoken', None)
        no_reply = obj.pop('$no_reply', False)
        if 'jsonrpc' not in obj:
            obj = dict({'jsonrpc': '2.0'}, **obj)
        if tok is not None:
            rest = {k: v for k, v in obj.items() if k not in ('jsonrpc', 'id')}
            inner = json.dumps(rest, ensure_ascii=not utf8)[1:-1]
            text = '{"jsonrpc":%s,"id":%s%s}' % (json.dumps(obj.get('jsonrpc')), tok, (',' + inner) if inner else '')
            req_tok = tok
        else:
            text = json.dumps(obj, ensure_ascii=not utf8)
            req_tok = json.dumps(obj['id']) if 'id' in obj else None
        if 'method' in obj and req_tok is not None and not no_reply:
            self.pending.append(req_tok)
        return text.encode('utf-8') + b'\n'

    def send(self, item, utf8=False):
        self.write(self.encode(item, utf8))

    def send_control(self, item, utf8=False):
        """Items of `post`: control items or messages."""
        if isinstance(item, dict):
            if '$sleep' in item:
                time.sleep(item['$sleep'])
                return
            if '$close_stdin' in item:
                self.close_stdin()
                return
            if '$exit' in item:
                self.finish('scripted-exit-%s' % item['$exit'])
            if '$requests' in item:
                self.flood(item['$requests'], item.get('base', 100000))
                return
        self.send(item, utf8)

    def flood(self, n, base):
        """N server requests in writes of <= 60000 bytes; replies are not tracked (P7 blocked-write cases)."""
        buf = b''
        for k in range(n):
            buf += ('{"jsonrpc":"2.0","id":%d,"method":"x/flood","params":{}}\n' % (base + k)).encode()
            if len(buf) > 60000:
                self.write(buf)
                buf = b''
        if buf:
            self.write(buf)
        self.t['notes'].append('flood %d' % n)

    def close_stdin(self):
        if not self.stdin_closed:
            os.close(0)
            self.stdin_closed = True
            self.note('stdin closed')

    # ---- input ---------------------------------------------------------------------------------------------
    def startup_checks(self):
        if self.t['argv'] != ARGV:
            self.violation('argv %s' % json.dumps(self.t['argv']))
        want = self.expect.get('cwd')
        if want is not None and os.path.realpath(want) != os.path.realpath(os.getcwd()):
            self.violation('cwd %s != %s' % (os.getcwd(), want))
        if 'grokHome' in self.expect and os.environ.get('GROK_HOME') != self.expect['grokHome']:
            self.violation('GROK_HOME %r != %r' % (os.environ.get('GROK_HOME'), self.expect['grokHome']))
        for name, value in self.expect.get('env', {}).items():
            if os.environ.get(name) != value:
                self.violation('parent env %s not inherited' % name)

    def ingest(self, raw):
        """Log one client line; returns (kind, message, raw id token)."""
        if not raw.endswith(b'\n'):
            self.violation('client line without LF')
        text = raw.decode('utf-8', 'backslashreplace')
        try:
            msg = json.loads(raw.decode('utf-8'))
            tokmsg = load_raw(raw.decode('utf-8'))
        except Exception:
            self.t['recv'].append({'kind': 'invalid', 'text': text})
            self.violation('client sent invalid JSON')
            return 'invalid', None, None
        if not isinstance(msg, dict):
            self.t['recv'].append({'kind': 'invalid', 'text': text})
            self.violation('client message is not an object')
            return 'invalid', None, None
        if msg.get('jsonrpc') != '2.0':
            self.violation('jsonrpc is not "2.0"')
        tok = token_of(tokmsg['id']) if 'id' in tokmsg else None
        if 'method' in msg:
            kind = 'request' if 'id' in msg else 'notification'
        else:
            kind = 'reply'
        self.t['recv'].append({'kind': kind, 'text': text.rstrip('\n'), 'method': msg.get('method'), 'id': tok})
        self.flush()
        if kind == 'notification':
            self.violation('notification %s' % msg['method'])   # includes `initialized`
        elif kind == 'reply':
            self.check_reply(msg, tok)
        return kind, msg, tok

    def check_reply(self, msg, tok):
        """Reply to a server request: same raw id, error -32601 "not handled"."""
        match = None
        for want in self.pending:
            if want == tok or (finite(want) and tok is not None and finite(tok) and float(want) == float(tok)):
                match = want
            elif not finite(want) and want not in ('null', 'true', 'false') and not want.startswith('"') and tok == 'null':
                match = want   # P10: TS re-serializes an overflowing id as null; the oracle classifies it
        if match is None:
            self.violation('unexpected reply id %s' % tok)
        else:
            self.pending.remove(match)
        if msg.get('error') != NOT_HANDLED or 'result' in msg:
            self.violation('reply to a server request is not error -32601 "not handled"')
        extra = set(msg) - {'jsonrpc', 'id', 'error'}
        if extra:
            self.violation('reply has extra keys %s' % sorted(extra))

    # ---- requests ------------------------------------------------------------------------------------------
    def request(self, msg, tok):
        method = msg['method']
        want = self.next_id
        self.next_id += 1
        if tok != str(want):
            self.violation('request id token %s expected %d' % (tok, want))
        extra = set(msg) - {'jsonrpc', 'id', 'method', 'params'}
        if extra:
            self.violation('request has extra keys %s' % sorted(extra))
        params = msg.get('params')
        if method == 'initialize':
            if self.seen_initialize or want != 1:
                self.violation('initialize is not the first request with id 1')
            if params != INIT_PARAMS:
                self.violation('initialize params ' + json.dumps(params))
            self.seen_initialize = True
        elif method == 'session/load':
            if want != 2:
                self.violation('session/load id %d, expected 2' % want)
            if not self.seen_initialize:
                self.violation('session/load before initialize')
            elif not self.load_ok:
                self.violation('session/load although loadSession was not strictly true')
            exp = self.expect
            if 'thread' in exp and 'folder' in exp:
                if params != {'sessionId': exp['thread'], 'cwd': exp['folder'], 'mcpServers': []}:
                    self.violation('session/load params ' + json.dumps(params))
            elif not (isinstance(params, dict) and set(params) == {'sessionId', 'cwd', 'mcpServers'} and params['mcpServers'] == []):
                self.violation('session/load params shape ' + json.dumps(params))
        else:
            self.violation('unexpected method ' + method)
        actions = self.scenario.get('methods', {}).get(method)
        if actions is None:
            if method == 'initialize':
                actions = [{'result': {'agentCapabilities': {'loadSession': True}}}]
            elif method == 'session/load':
                actions = [{'result': None}]
            else:
                self.send({'id': json.loads(tok) if finite(tok) else tok, 'error': {'code': -32601, 'message': 'Method not found: ' + method}})
                return
        if isinstance(actions, dict):
            actions = [actions]
        k = self.used.get(method, 0)
        self.used[method] = k + 1
        self.perform(copy.deepcopy(actions[min(k, len(actions) - 1)]), method, msg['id'])

    def response_bytes(self, act, method, mid):
        utf8 = bool(act.get('utf8'))
        if 'rawhex' in act:
            return bytes.fromhex(act['rawhex']) + b'\n'
        if 'raw' in act:
            return act['raw'].encode('utf-8') + b'\n'
        item = {}
        if 'error' in act:
            item['error'] = act['error']
        elif not act.get('omit_result'):
            default = {'agentCapabilities': {'loadSession': True}} if method == 'initialize' else None
            item['result'] = act.get('result', default)
        if 'idtoken' in act:
            item['$idtoken'] = act['idtoken']
        else:
            item['id'] = act.get('id', mid)
        out = self.encode(item, utf8)
        for extra in act.get('together', []):
            out += self.encode(extra, utf8)
        return out

    def perform(self, act, method, mid):
        utf8 = bool(act.get('utf8'))
        if act.get('stderr'):
            sys.stderr.write(expand(act['stderr']))
            sys.stderr.flush()
        if act.get('stderr_flood'):
            left = act['stderr_flood']
            while left > 0:
                n = min(left, 8192)
                sys.stderr.write('e' * n)
                sys.stderr.flush()
                left -= n
        if act.get('inherit_pipe'):
            child = subprocess.Popen([sys.executable, '-c', 'import sys,time; time.sleep(float(sys.argv[1]))', str(act['inherit_pipe'])],
                                     stdin=subprocess.DEVNULL)
            self.note('descendant pid %d' % child.pid)
        for item in act.get('pre', []):
            self.send_control(item, utf8)
        for _ in range(act.get('await', 0)):
            got = self.stdin.readline()
            if not got:
                break
            self.ingest(got)
        if act.get('delay'):
            time.sleep(act['delay'])
        if act.get('exit') is not None:
            self.t['exit'] = 'scripted-exit-%s' % act['exit']
            self.flush()
            os._exit(act['exit'])
        if act.get('no_read'):
            self.note('no_read %s' % act['no_read'])
            time.sleep(act['no_read'])
            self.finish('no_read-done')
        if act.get('stall'):
            return
        if 'oversize' in act:
            self.write(b'a' * act['oversize'])
            return
        data = self.response_bytes(act, method, mid)
        pads = [i['$pad'] for i in act.get('together', []) if isinstance(i, dict) and '$pad' in i]
        if act.get('p6') and len(data) > 512:
            self.violation('scenario: p6 write has %d bytes (> 512)' % len(data))
        if act.get('p12') and not any(n > 8192 for n in pads):
            self.violation('scenario: p12 needs a pad > 8192 bytes')
        extra = b''
        if act.get('also_correct') and ('id' in act or 'idtoken' in act):
            good = {k: v for k, v in act.items() if k in ('result', 'error')}
            extra = self.encode(dict(good, id=mid), utf8)
        if method == 'initialize':
            # loadSession is granted when a line of all bytes written for this reply (the scripted reply, its `together`
            # items and an `also_correct` line) is, as a JSON.parse client reads it (invalid UTF-8 -> U+FFFD,
            # 1.0 == 1e0 == 1, any depth), a result for id 1 with loadSession true (an omitted result is not);
            # mirrors reference/tail.ts withRpcChild: a message with a string `method` is a server request (never a reply);
            # only a numeric id (not a boolean) matches the waiting request, and only the first such message settles it;
            # an object `error` rejects, `error: null` / a non-object error does not (resolve with `result`)
            ok = False
            for line in (data + extra).decode('utf-8', 'replace').split('\n'):
                try:
                    m = json.loads(line.strip())
                except (ValueError, RecursionError):
                    continue
                if not isinstance(m, dict) or isinstance(m.get('method'), str):
                    continue
                mid_in = m.get('id')
                if isinstance(mid_in, bool) or not isinstance(mid_in, (int, float)) or mid_in != mid:
                    continue
                err = m.get('error')
                res = None if isinstance(err, dict) else m.get('result')
                caps = res.get('agentCapabilities') if isinstance(res, dict) else None
                ok = isinstance(caps, dict) and caps.get('loadSession') is True
                break
            self.load_ok = ok
        self.write(data)
        if extra:
            self.write(extra)
        if act.get('close_stdin'):
            self.close_stdin()
        for item in act.get('post', []):
            self.send_control(item, utf8)
        if act.get('eof_no_exit'):
            os.close(1)
            self.stdout_open = False
            self.note('stdout closed, staying alive')
            time.sleep(act['eof_no_exit'])
            self.finish('eof_no_exit-done')
        if self.stdin_closed:
            self.finish('done-after-close_stdin')

    # ---- main loop -----------------------------------------------------------------------------------------
    def run(self):
        self.startup_checks()
        signal.alarm(int(self.scenario.get('lifetime', 120)))      # SIGALRM kills a leaked fake
        if self.scenario.get('ignore_term'):
            signal.signal(signal.SIGTERM, signal.SIG_IGN)
        else:
            signal.signal(signal.SIGTERM, lambda *_: self.finish('SIGTERM'))
        while True:
            raw = self.stdin.readline()
            if not raw:
                break
            kind, msg, tok = self.ingest(raw)
            if kind == 'request':
                self.request(msg, tok)
        if self.pending and not self.scenario.get('allow_no_reply') and not self.stdin_closed:
            self.violation('no reply to server request(s) %s' % ','.join(self.pending))
        self.finish('eof')


def main():
    exe = os.path.abspath(os.environ.get('Q_ACP_EXE') or sys.argv[0])
    Server(exe, sys.argv[1:]).run()


if __name__ == '__main__':
    main()
