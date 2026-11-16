"""Narrow, evidence-checked comparison of a Pruv runner against the pinned TypeScript (DESIGN §6, docs/GROK_IMPORT_SPEC.md).

    compare(case, ts, impl, p4_baseline=None) -> list of differences

`ts` / `impl` are the dicts returned by common.run_side. Every difference is {area, detail, policy, severity}:
  policy    None = unexplained, else 'P1'..'P12' (P8 does not apply to Grok): the policy whose TRIGGER is present in
            the case inputs and whose allowed difference is exactly what was seen
  severity  'diff' (a mismatch), 'limit' (a known limit such as a P7 watchdog kill: explained, never a success),
            'note' (reported, not a mismatch: e.g. live mutations during listing are outside the equivalence domain)
`unexplained(diffs)` are the ones that fail a run.

Exact (never relaxed): per-path primitive order, target lstat positions, wx/mode/rename/cleanup, entropy, ACP request
order and content, verify count, results, final tree bytes/kinds/modes. Normalised (representation only): fixture
roots, handles, dev/ino as relations, stream read/write splitting, timing of independent listing slots.
P10 numbers are compared as binary64; JSON numbers keep their raw token on the Pruv side (common.Num).
P4 is judged against a TS rerun on a case whose unsupported raw date tokens were edited (`p4_case`), never by deleting
timestamps from outputs.
"""
import sys
sys.dont_write_bytecode = True
import copy
import json
import math
import re
from common import CLASSES, CODES, ERRNO_CLASSES, STAGES, Num, bytes_of, concurrency_info, normalize_trace, tree_files

LONE = re.compile('[\ud800-\udfff]')
HIGH_END = re.compile('[\ud800-\udbff]$')
SAFE = 9007199254740991


def D(area, detail, policy=None, severity='diff'):
    return {'area': area, 'detail': detail, 'policy': policy, 'severity': severity}


def unexplained(diffs):
    return [d for d in diffs if d['severity'] == 'diff' and d['policy'] is None]


# ---- numbers and JSON values ---------------------------------------------------------------------------------------
def is_num(x):
    return isinstance(x, (int, float)) and not isinstance(x, bool)


def eq(a, b, notes):
    """Deep equality; numbers as binary64 (P10). A TS null against a Pruv token that overflows binary64 is noted 'P10'."""
    if is_num(a) and is_num(b):
        return float(a) == float(b) or (math.isnan(float(a)) and math.isnan(float(b)))
    if a is None and is_num(b) and math.isinf(float(b)) or b is None and is_num(a) and math.isinf(float(a)):
        notes.add('P10')
        return True
    if isinstance(a, dict) and isinstance(b, dict):
        return set(a) == set(b) and all(eq(a[k], b[k], notes) for k in a)
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(eq(x, y, notes) for x, y in zip(a, b))
    return type(a) is type(b) and a == b if isinstance(a, bool) or isinstance(b, bool) else a == b


def strings(v, fn):
    if isinstance(v, str):
        return fn(v)
    if isinstance(v, list):
        return [strings(x, fn) for x in v]
    if isinstance(v, dict):
        return {k: strings(x, fn) for k, x in v.items()}
    return v


# ---- result normalisation ------------------------------------------------------------------------------------------
def named(value, table):
    return table[value] if isinstance(value, int) and not isinstance(value, bool) and 0 <= value < len(table) else value


def norm_result(res, roots):
    """Result JSON -> comparable form: logical roots, defaults for N/A fields, names for stage/class/code, no detail."""
    if res is None:
        return None
    r = copy.deepcopy(res)
    for key, default in (('omittedBefore', False), ('rows', []), ('title', None), ('model', None), ('value', None), ('present', False), ('copySkipped', 0)):
        if r.get(key) is None:
            r[key] = default
    if isinstance(r.get('status'), int):
        r['status'] = 'ok' if r['status'] == 0 else 'error'
    prefixes = [(roots[k], n) for k, n in (('src', '$SRC'), ('dst', '$DST'), ('src_real', '$SRC'), ('dst_real', '$DST')) if roots.get(k)]
    r = strings(r, lambda s: next((n + s[len(p):] for p, n in prefixes if s == p or s.startswith(p + '/')), s))
    err = r.get('error')
    if err:
        err.pop('detail', None)
        err['stage'] = named(err.get('stage'), STAGES)
        err['class'] = named(err.get('class'), CLASSES)
        err['errnoClass'] = named(err.get('errnoClass', 'none'), ERRNO_CLASSES)
        code = named(err.get('code'), CODES)
        err['code'] = None if code in (None, 'none') else code
        err.setdefault('native', 0)
        r['error'] = error_view(err)
    return r


def error_view(e):
    """The compared part of an error: stage/class/code always; OS facts for raw OS errors; raw code for RPC errors
    (an absent code differs from JSON null); the name for URIError/AbortError. Diagnostic text is never compared."""
    view = {'stage': e['stage'], 'class': e['class'], 'code': e['code']}
    if e['class'] == 'RawOsError':
        view.update(errnoClass=e['errnoClass'], native=e['native'], raw=e.get('raw'))
    if e['class'] == 'RpcResponseError':
        view.update(raw=e.get('raw'), raw_present='raw' in e)
    if e['class'] in ('URIError', 'AbortError'):
        view['name'] = e.get('name')
    return view


# ---- case inspection: P-triggers -----------------------------------------------------------------------------------
DATE_KEY = re.compile(rb'"(updated_at|timestamp|created_at)"\s*:\s*("(?:[^"\\]|\\.)*"|-?[0-9][0-9.eE+-]*)')
ISO_Z = re.compile(r'^(\d{4})-(\d\d)-(\d\d)T(\d\d):(\d\d):(\d\d)\.(\d{3})Z$')
MSG_KEY = re.compile(rb'"num_chat_messages"\s*:\s*(-?[0-9][0-9.eE+-]*)')
BIG_EXP = re.compile(rb'-?\d+(?:\.\d+)?[eE][+-]?\d+')


def p4_supported(token):
    """P4: safe lexical integers; trimmed unsigned digit strings; exact Gregorian YYYY-MM-DDTHH:mm:ss.sssZ; blank = no date."""
    text = token.decode('utf-8', 'replace')
    if text.startswith('"'):
        try:
            s = json.loads(text)
        except ValueError:
            return True
        s = s.strip()
        if s == '' or re.fullmatch(r'\d+', s):
            return True
        m = ISO_Z.match(s)
        if not m:
            return False
        y, mo, d, h, mi, sec, _ = map(int, m.groups())
        leap = y % 4 == 0 and (y % 100 != 0 or y % 400 == 0)
        days = [31, 29 if leap else 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        return 1 <= mo <= 12 and 1 <= d <= days[mo - 1] and h < 24 and mi < 60 and sec < 60
    return bool(re.fullmatch(r'-?(0|[1-9]\d*)', text)) and abs(int(text)) <= SAFE


def p4_edit(data):
    """Raw bytes with every unsupported date token replaced by the string "invalid": TS reads no date from it (as Pruv),
    yet it is not nullish, so `timestamp ?? created_at` does not fall back to the other key."""
    return DATE_KEY.sub(lambda m: m.group(0) if p4_supported(m.group(2)) else b'"' + m.group(1) + b'":"invalid"', data)


def p4_case(case):
    """The TS baseline case of P4: only unsupported raw date tokens edited. None when the case has none."""
    changed = False
    tree = []
    for e in case.get('tree', []):
        if e['kind'] == 'file':
            data = bytes_of(e.get('bytes', {'utf8': ''}), case.get('blobs', {}))
            new = p4_edit(data)
            if new != data:
                changed = True
                e = dict(e, bytes={'hex': new.hex()})
        tree.append(e)
    return dict(case, tree=tree, name=case['name'] + '-p4') if changed else None


def p1_edit(path, data):
    """Bytes as Pruv's parser sees them under P1: a .jsonl line, or a whole other JSON file, that is invalid UTF-8 or
    deeper than 128 becomes same-length non-JSON filler (TS then ignores the line / finds no summary or active list,
    as Pruv does). Sizes stay, so size caps and byte windows are unchanged."""
    if path.endswith('.jsonl'):
        parts = data.split(b'\n')
        return b'\n'.join(b'x' * len(l) if deep_or_invalid(l) else l for l in parts)
    return b'x' * len(data) if deep_or_invalid(data) else data


def p1_case(case):
    """The TS baseline case of P1: only the rejected JSON edited (p1_edit). None when nothing changes."""
    changed = False
    tree = []
    for e in case.get('tree', []):
        if e['kind'] == 'file':
            data = bytes_of(e.get('bytes', {'utf8': ''}), case.get('blobs', {}))
            new = p1_edit(e['path'], data)
            if new != data:
                changed = True
                e = dict(e, bytes={'hex': new.hex()})
        tree.append(e)
    return dict(case, tree=tree, name=case['name'] + '-p1') if changed else None


def p14_case(case):
    """The TS baseline case of P1+P4: the rejected lines are decided on the ORIGINAL bytes (p1_edit), then the P4 edit is
    applied to what remains, so an edit of a date token can never remove the cause of a rejection. None when nothing changes."""
    changed = False
    tree = []
    for e in case.get('tree', []):
        if e['kind'] == 'file':
            data = bytes_of(e.get('bytes', {'utf8': ''}), case.get('blobs', {}))
            new = p4_edit(p1_edit(e['path'], data))
            if new != data:
                changed = True
                e = dict(e, bytes={'hex': new.hex()})
        tree.append(e)
    return dict(case, tree=tree, name=case['name'] + '-p14') if changed else None


def p9_bad_threads(case):
    """Threads whose summary.json num_chat_messages token is a number P9 does not keep."""
    bad = set()
    for path, data in tree_files(case):
        m = re.search(r'/([^/]+)/summary\.json$', path)
        toks = MSG_KEY.findall(data)
        if m and toks:
            t = toks[-1].decode()
            if not (re.fullmatch(r'(0|[1-9]\d*)', t) and int(t) <= SAFE):
                bad.add(m.group(1))
    return bad


def max_depth(line):
    depth = best = 0
    in_str = esc = False
    for ch in line:
        if in_str:
            esc = (not esc) and ch == '\\'
            in_str = esc or ch != '"'
        elif ch == '"':
            in_str = True
        elif ch in '[{':
            depth += 1
            best = max(best, depth)
        elif ch in ']}':
            depth -= 1
    return best


def deep_or_invalid(data):
    try:
        text = data.decode('utf-8')
    except UnicodeDecodeError:
        return True
    if text.count('[') + text.count('{') < 129:
        return False
    return any(max_depth(l) > 128 for l in text.split('\n') if l.count('[') + l.count('{') >= 129)


def acp_actions(case):
    out = []
    for v in case.get('acp', {}).values():
        out.extend(v if isinstance(v, list) else [v])
    return [a for a in out if isinstance(a, dict)]


def triggers(case):
    """The P-policies whose trigger input is present in the case (detected from the recipe, not declared)."""
    files = tree_files(case)
    acts = acp_actions(case)
    blobs = case.get('blobs', {})
    t = set()
    sched = [a for r in case.get('schedule', {}).get('rules', []) for a in r.get('actions', [])]
    if any(deep_or_invalid(d) for _, d in files):
        t.add('P1')
    for a in acts:
        if 'rawhex' in a and deep_or_invalid(bytes.fromhex(a['rawhex'])):
            t.update(('P1', 'P1-ACP'))
    # lone surrogate escapes, or an astral scalar (raw 4-byte UTF-8 or an escaped pair) that a 120-unit cut can split
    if any(re.search(rb'\\u[dD][89a-fA-F][0-9a-fA-F]{2}|[\xf0-\xf4]', d) for _, d in files):
        t.add('P2')
    if any(b'\xef\xbf\xbd' in d or re.search(rb'\\u[fF][fF][fF][dD]', d) for _, d in files):
        t.add('P3')
    if any(DATE_KEY.search(d) and any(not p4_supported(m.group(2)) for m in DATE_KEY.finditer(d)) for _, d in files):
        t.add('P4')
    if any(len(d) > 16777216 for _, d in files) or any('bytes' in a and len(bytes_of(a['bytes'], blobs)) > 16000000 for a in sched):
        t.add('P5')
    for a in acts:
        tok = a.get('idtoken')
        if tok is not None and tok not in ('1', '2') or 'oversize' in a or a.get('p6'):
            t.add('P6')
        if a.get('close_stdin') or any(isinstance(i, dict) and '$close_stdin' in i for i in a.get('post', [])):
            t.add('P11')
        if a.get('p12'):
            t.add('P12')
        if any(k in a for k in ('eof_no_exit', 'no_read', 'inherit_pipe')) or any(isinstance(i, dict) and '$requests' in i for i in a.get('post', [])):
            t.add('P7')
    if case.get('acp_options', {}).get('ignore_term'):
        t.add('P7')
    if p9_bad_threads(case):
        t.add('P9')
    if any(BIG_EXP.search(d) and any(math.isinf(float(x.decode())) for x in BIG_EXP.findall(d)) for _, d in files) \
            or any(re.search(r'-?\d+(?:\.\d+)?[eE][+-]?\d{3,}', json.dumps(a)) for a in acts):
        t.add('P10')
    return t


# ---- value transformers (applied to the TS side; the comparison afterwards must be exact) ----------------------------
def utf16_len(s):
    return len(s.encode('utf-16-le', 'surrogatepass')) // 2


def p2(v):
    """P2: lone surrogate values become U+FFFD; a 120-unit cut that split a scalar loses the lone high surrogate."""
    def title(x):
        if isinstance(x, dict):
            return {k: (HIGH_END.sub('', s) if k == 'title' and isinstance(s, str) and utf16_len(s) == 120 else title(s)) for k, s in x.items()}
        if isinstance(x, list):
            return [title(i) for i in x]
        return x
    return strings(title(v), lambda s: LONE.sub('�', s))


def p3(actual, expected):
    """P3: TS removes a genuine trailing U+FFFD after the 32768-byte clip. Only turn prompt/reply strings of 32765..32768
    Pruv bytes are compared with trailing U+FFFD stripped on both sides."""
    a_rows, e_rows = actual.get('rows'), expected.get('rows')
    if not (isinstance(a_rows, list) and isinstance(e_rows, list) and len(a_rows) == len(e_rows)):
        return actual, expected
    na, ne = [], []
    for a, e in zip(a_rows, e_rows):
        if isinstance(a, dict) and isinstance(e, dict):
            a, e = dict(a), dict(e)
            for k in ('prompt', 'reply'):
                if isinstance(a.get(k), str) and isinstance(e.get(k), str) and 32765 <= len(a[k].encode('utf-8', 'surrogatepass')) <= 32768:
                    a[k], e[k] = a[k].rstrip('�'), e[k].rstrip('�')
        na.append(a)
        ne.append(e)
    return dict(actual, rows=na), dict(expected, rows=ne)


def p9(expected, bad):
    """P9: Pruv drops `messages` where the summary token is not a plain nonnegative integer <= 2^53-1."""
    rows = expected.get('rows')
    if not (isinstance(rows, list) and bad):
        return expected
    return dict(expected, rows=[{k: v for k, v in r.items() if not (k == 'messages' and r.get('thread') in bad)} if isinstance(r, dict) else r for r in rows])


def explain_result(case, ts, im, trig, baseline, base1=None, base14=None):
    """None when equal, else the list of policies that make `im` the expected result; False when unexplained."""
    notes, names = set(), []
    t = ts
    for name, fn in (('P2', p2),):
        if name in trig:
            n = fn(t)
            if n != t:
                names.append(name)
                t = n
    if 'P9' in trig:
        n = p9(t, p9_bad_threads(case))
        if n != t:
            names.append('P9')
            t = n
    a, t3 = im, t
    if 'P3' in trig:
        a, t3 = p3(im, t)
        if (a, t3) != (im, t):
            names.append('P3')
    if eq(t3, a, notes):
        if 'P10' in notes and 'P10' in trig:
            names.append('P10')
        elif 'P10' in notes:
            return False
        return names
    if 'P4' in trig:
        if baseline is None:
            return False
        b = baseline
        for fn in (p2,) if 'P2' in trig else ():
            b = fn(b)
        if 'P9' in trig:
            b = p9(b, p9_bad_threads(case))
        a4, b4 = p3(im, b) if 'P3' in trig else (im, b)
        if eq(b4, a4, notes):
            return names + ['P4']
    if 'P1' in trig and base1 is not None:
        b = p2(base1) if 'P2' in trig else base1
        if 'P9' in trig:
            b = p9(b, p9_bad_threads(case))
        a1, b1 = p3(im, b) if 'P3' in trig else (im, b)
        if eq(b1, a1, notes):
            return names + ['P1']
    if 'P1' in trig and 'P4' in trig and base14 is not None:
        b = p2(base14) if 'P2' in trig else base14
        if 'P9' in trig:
            b = p9(b, p9_bad_threads(case))
        a14, b14 = p3(im, b) if 'P3' in trig else (im, b)
        if eq(b14, a14, notes):
            return names + ['P1', 'P4']
    return False


def acp_target_only(ts, im, bad, policy):
    """An ACP policy that flips a copy's outcome also flips its target: the side that failed removed it, the side that
    succeeded kept it. The tree difference is explained only when it is exactly one copied target (a directory and the
    entries under it), present on the successful side and absent on the failed side; the rest of the tree is equal."""
    if policy not in ('P6', 'P7', 'P11', 'P12') or not bad:
        return False
    ok_side = [x for x in (ts, im) if x['result'].get('status') == 'ok']
    bad_side = [x for x in (ts, im) if x['result'].get('status') != 'ok']
    if len(ok_side) != 1 or len(bad_side) != 1:
        return False
    top = bad[0]
    if any(k != top and not k.startswith(top + '/') for k in bad):
        return False
    return all(k in ok_side[0]['after']['dst'] and k not in bad_side[0]['after']['dst'] for k in bad)


def acp_class_policy(ts, im, trig):
    """P1/P5/P6/P7/P11/P12: differences that only exist at those triggers, checked on the error/result pair."""
    te, ie = ts.get('error'), im.get('error')
    ic = ie['code'] if ie else None
    tc = te['code'] if te else None
    if 'P5' in trig and ic == 'InputTooLarge' and tc != 'InputTooLarge':
        return 'P5'
    if 'P11' in trig and ie and (ic in ('read-failed', 'grok-load-failed')) and tc != ic:
        return 'P11'
    if 'P12' in trig and ts.get('status') != 'ok' and im.get('status') == 'ok' and tc in ('grok-load-failed', 'timeout'):
        return 'P12'
    if 'P6' in trig and ie and ic in ('timeout', 'read-failed', 'grok-load-failed') and (ts.get('status') == 'ok' or tc != ic):
        return 'P6'
    if 'P7' in trig and ie and ic in ('timeout', 'read-failed', 'grok-load-failed') and tc != ic:
        return 'P7'
    # P1 on rejected JSON in a file is judged only by the P1 baseline rerun (explain_result); here only a rejected ACP
    # reply line (recorded as 'P1-ACP' by triggers) can explain an error/result difference
    if 'P1-ACP' in trig:
        if ts.get('status') == 'ok' and ie and ic in ('not-found', 'timeout', 'read-failed', 'grok-load-failed'):
            return 'P1'
        # a rejected ACP reply line is never delivered: Pruv waits out the request that TS failed on another ground
        if te and ie and ic == 'timeout' and te.get('stage') in ('initialize', 'session/load') and te.get('stage') == ie.get('stage'):
            return 'P1'
    return None


def monotone(ts_rows, im_rows):
    """Pruv's rows are a subsequence of TS's (a rejected summary / line only removes things)."""
    it = iter(ts_rows)
    return all(any(r == t for t in it) for r in im_rows)


# ---- the comparison ------------------------------------------------------------------------------------------------
def acp_view(side):
    log = side.get('acp')
    if log is None:
        return None
    from common import load_raw
    recv = []
    for r in log.get('recv', []):
        try:
            recv.append(load_raw(r['text']))
        except ValueError:
            recv.append(r['text'])
    return {'argv': log.get('argv'), 'recv': recv, 'violations': log.get('violations', []), 'grokHome': log.get('grokHome')}


def first_spawn(trace):
    return next((i for i, r in enumerate(trace) if r.get('op') == 'spawn'), None)


def compare(case, ts, impl, p4_baseline=None):
    """Differences between the pinned TypeScript run `ts` and a Pruv runner run `impl` of the same case. The baseline is
    None, the P4 baseline run, or {'P4': run or None, 'P1': run or None} (test_impl.run_reference)."""
    p1_baseline = p14_baseline = None
    if isinstance(p4_baseline, dict) and 'result' not in p4_baseline:
        p1_baseline, p14_baseline, p4_baseline = p4_baseline.get('P1'), p4_baseline.get('P1+P4'), p4_baseline.get('P4')
    diffs = []
    trig = triggers(case)
    if ts.get('meta') and ts['meta'].get('node') != '26.4.0':
        diffs.append(D('environment', 'TS reference ran on Node %s, not 26.4.0' % ts['meta'].get('node')))
    for name, side in (('ts', ts), ('impl', impl)):
        if side.get('watchdog'):
            diffs.append(D('watchdog', '%s killed by the watchdog (never a success)' % name, 'P7' if 'P7' in trig else None, 'limit' if 'P7' in trig else 'diff'))
        elif side.get('returncode') not in (0, None):
            diffs.append(D('runner', '%s exited with %s: %s' % (name, side['returncode'], side.get('stderr', '')[-300:])))
        if side.get('result') is None and not side.get('watchdog'):
            diffs.append(D('runner', '%s printed no result' % name))
    if ts.get('result') is None or impl.get('result') is None:
        return diffs
    if ts['before'] != impl['before']:
        diffs.append(D('setup', 'the two sides built different trees'))
    tr, ir = norm_result(ts['result'], ts['roots']), norm_result(impl['result'], impl['roots'])
    baseline = norm_result(p4_baseline['result'], p4_baseline['roots']) if p4_baseline and p4_baseline.get('result') else None
    base1 = norm_result(p1_baseline['result'], p1_baseline['roots']) if p1_baseline and p1_baseline.get('result') else None
    base14 = norm_result(p14_baseline['result'], p14_baseline['roots']) if p14_baseline and p14_baseline.get('result') else None

    # result ---------------------------------------------------------------------------------------------------
    result_policy = None
    notes = set()
    if not (eq(tr, ir, notes) and not notes):
        why = explain_result(case, tr, ir, trig, baseline, base1, base14)
        if why:
            diffs.append(D('result', 'explained by ' + '+'.join(why), '+'.join(why), 'note'))
        else:
            result_policy = acp_class_policy(tr, ir, trig)
            detail = 'ts=%s impl=%s' % (json.dumps(tr, default=str)[:400], json.dumps(ir, default=str)[:400])
            if result_policy is None and 'P4' in trig and baseline is None:
                detail += ' (P4 trigger: run oracle.p4_case on the TS side and pass it as p4_baseline)'
            diffs.append(D('result', detail, result_policy))

    # ACP transcript -------------------------------------------------------------------------------------------
    ta, ia = acp_view(ts), acp_view(impl)
    acp_policy = result_policy if result_policy in ('P6', 'P7', 'P11', 'P12', 'P1', 'P5') else None
    if (ta is None) != (ia is None):
        diffs.append(D('acp', 'the fake ACP was started on one side only (ts=%s impl=%s)' % (ta is not None, ia is not None), acp_policy))
    elif ta is not None:
        for name, v in (('ts', ta), ('impl', ia)):
            if v['violations']:
                diffs.append(D('acp-violation', '%s: %s' % (name, '; '.join(v['violations'][:3]))))
        notes = set()
        if ta['argv'] != ia['argv']:
            diffs.append(D('acp', 'argv differs: ts=%s impl=%s' % (ta['argv'], ia['argv'])))
        if not eq(ta['recv'], ia['recv'], notes):  # the request stream is the RPC part an ACP policy may explain
            diffs.append(D('acp', 'requests differ: ts=%s impl=%s' % (json.dumps(ta['recv'], default=str)[:300], json.dumps(ia['recv'], default=str)[:300]), acp_policy))
        elif notes:
            diffs.append(D('acp', 'server-request id echoed as null by TS (binary64 overflow)', 'P10' if 'P10' in trig else None))

    # primitive trace -----------------------------------------------------------------------------------------
    tt = normalize_trace(ts['trace'], ts['roots'], case['mode'], reference=True)
    it = normalize_trace(impl['trace'], impl['roots'], case['mode'])
    if tt != it:
        at = next((i for i, (a, b) in enumerate(zip(tt, it)) if a != b), min(len(tt), len(it)))
        spawn = first_spawn(tt)
        allowed = acp_policy in ('P6', 'P7', 'P11', 'P12') and spawn is not None and at > spawn
        diffs.append(D('trace', 'first difference at record %d: ts=%s impl=%s' % (at, json.dumps(tt[at:at + 1]), json.dumps(it[at:at + 1])), acp_policy if allowed else None))

    # final trees ---------------------------------------------------------------------------------------------
    for part in ('src', 'dst'):
        if ts['after'][part] != impl['after'][part]:
            keys = sorted(set(ts['after'][part]) | set(impl['after'][part]))
            bad = [k for k in keys if ts['after'][part].get(k) != impl['after'][part].get(k)]
            diffs.append(D('tree', '%s differs at %s' % (part, bad[:5]),
                           acp_policy if part == 'dst' and acp_target_only(ts, impl, bad, acp_policy) else None))

    # concurrency conditions (list mode) ------------------------------------------------------------------------
    want = case.get('concurrency')
    if want:
        for name, side in (('ts', ts), ('impl', impl)):
            info = concurrency_info(side['trace'], side['roots'])
            if info['max_outstanding'] < want.get('max_outstanding', 0):
                diffs.append(D('concurrency', '%s: only %d requests outstanding together' % (name, info['max_outstanding'])))
            for early, late in want.get('started_before', []):
                if not (early in info['first_issue'] and late in info['last_complete'] and info['first_issue'][early] < info['last_complete'][late]):
                    diffs.append(D('concurrency', '%s: %s did not start before %s completed' % (name, early, late)))
    if case['mode'] == 'list' and any(r.get('actions') for r in case.get('schedule', {}).get('rules', [])):
        diffs.append(D('domain', 'live mutations during listing are outside the tested equivalence domain; reported, not a match', None, 'note'))
    return diffs
