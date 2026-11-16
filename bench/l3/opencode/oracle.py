"""Exact comparison of a Pruv runner against the pinned OpenCode TypeScript, with ONLY the OpenCode policies P1-P13
(docs/OPENCODE_IMPORT_SPEC.md; DESIGN section 7). It does not import or share any Grok policy classifier: numbering and meaning differ.

    compare(case, ts, impl, base=None) -> list of differences
    unexplained(diffs)                 -> the ones that fail a run
    baseline_cases(run)                -> {name: case}: TS reruns with ONLY a policy's trigger input edited (P4); test_impl calls it

`ts` / `impl` are the dicts returned by common.run_side; `base` is {name: run} of the baseline reruns (or None).
A difference is {area, detail, policy, severity}:
  policy    None = unexplained, else 'P1+P4'...: the policies whose TRIGGER is present in the case inputs and whose allowed effect is
            exactly what was seen
  severity  'diff' (a mismatch), 'limit' (a known limit such as a watchdog kill: explained, never a success),
            'note' (reported, not a mismatch: explained deviations; observations outside the equivalence domain)

Deviations handled (each only at its stated trigger and with its stated effect):
  P1 invalid UTF-8 / depth > 128 / tape > 1,048,576 records in the export  -> Pruv: read-failed (detail json, stage parse), no data
  P2 lone surrogate / split 120-unit title scalar                          -> U+FFFD / dropped split scalar (TS value transformed)
  P3 genuine U+FFFD at the 32,768-byte clip                                -> trailing U+FFFD of the clipped string ignored
  P4 unsupported export timestamp tokens                                   -> judged against the TS rerun on the case with ONLY those tokens edited
  P5 export of 16,777,217..33,554,432 bytes with a successful exit         -> Pruv: CoreError/InputTooLarge exactly (stage not compared)
  P8 plan text slice splitting an astral scalar                            -> split scalar dropped (TS value transformed)
  P9 database numeric coercion                                             -> updatedMs/updatedAt/maybeOpen recomputed independently
     from the SQL-selected rows recorded in the TS trace (never by re-running SQL on normalised values)
P6, P7, P10, P11, P12, P13 are EQUAL policies: they grant no relaxation of output or trace. A watchdog kill is never a success.

Exact (never relaxed): results, row order, optional-field presence, error stage/class/code (+ native/raw cause facts), the SQL
prepare/all/get sequence with typed parameters and typed ordered results, connection/statement lifetime, admission calls, temp
prefix/location, wx/mode, child argv/cwd/effective env/stdio, close/stat/read/rm order, fake-exporter transcript, final trees.
Normalised (representation only, common.normalize_trace): fixture roots, opaque handles, request ids, random temp suffixes,
monotonic clock readings. Narrowings made here and nowhere else: the `stage` of clock_now/observe records (a wait-phase label, not a
fact of the call), the env variable `_` (set by /bin/sh for the exec'd program), and the content hash of SQLite `-shm` files (size and
mode are compared).
"""
import sys
sys.dont_write_bytecode = True
import copy
import json
import math
import re
import struct

from common import bytes_of, logical_path, normalize_trace, TraceAssertion

SAFE = 9007199254740991
EXPORT_MAX = 33554432
PARSER_MAX = 16777216
LONE = re.compile('[\ud800-\udfff]')
HIGH_END = re.compile('[\ud800-\udbff]$')
THREAD = re.compile(r'[A-Za-z0-9][A-Za-z0-9_-]{0,127}\Z')
ES_SPACE = '\t\n\x0b\x0c\r \xa0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000\ufeff'
NOISE_ENV = ('_',)


def D(area, detail, policy=None, severity='diff'):
    return {'area': area, 'detail': detail, 'policy': policy, 'severity': severity}


def unexplained(diffs):
    return [d for d in diffs if d['severity'] == 'diff' and d['policy'] is None]


# ---- JSON values, numbers -----------------------------------------------------------------------------------------------
def is_num(x):
    return isinstance(x, (int, float)) and not isinstance(x, bool)


def eq(a, b):
    """Deep equality; numbers as binary64 values (P10: no JSON number passes through, so values never carry raw tokens)."""
    if is_num(a) and is_num(b):
        return float(a) == float(b) or (math.isnan(float(a)) and math.isnan(float(b)))
    if isinstance(a, dict) and isinstance(b, dict):
        return set(a) == set(b) and all(eq(a[k], b[k]) for k in a)
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(eq(x, y) for x, y in zip(a, b))
    if isinstance(a, bool) or isinstance(b, bool):
        return type(a) is type(b) and a == b
    return a == b


def strings(v, fn):
    if isinstance(v, str):
        return fn(v)
    if isinstance(v, list):
        return [strings(x, fn) for x in v]
    if isinstance(v, dict):
        return {k: strings(x, fn) for k, x in v.items()}
    return v


def utf16_len(s):
    return len(s.encode('utf-16-le', 'surrogatepass')) // 2


def es_trim(s):
    return s.strip(ES_SPACE)


# ---- time (the part of P4/P9 the oracle computes independently) -----------------------------------------------------------
def civil_from_days(z):
    z += 719468
    era = z // 146097
    doe = z - era * 146097
    yoe = (doe - doe // 1460 + doe // 36524 - doe // 146096) // 365
    y = yoe + era * 400
    doy = doe - (365 * yoe + yoe // 4 - yoe // 100)
    mp = (5 * doy + 2) // 153
    d = doy - (153 * mp + 2) // 5 + 1
    m = mp + 3 if mp < 10 else mp - 9
    return y + (1 if m <= 2 else 0), m, d


def iso_of_ms(ms):
    """JS `new Date(ms).toISOString()` for a finite time value; None beyond TimeClip (8.64e15). Years outside 0..9999 get the signed
    six-digit form (what Pruv's rows module formats locally)."""
    if not is_num(ms) or math.isnan(float(ms)) or math.isinf(float(ms)):
        return None
    t = int(ms)
    if abs(t) > 8640000000000000:
        return None
    days, rem = divmod(t, 86400000)
    y, m, d = civil_from_days(days)
    h, rem = divmod(rem, 3600000)
    mi, rem = divmod(rem, 60000)
    s, msec = divmod(rem, 1000)
    year = '%04d' % y if 0 <= y <= 9999 else ('+%06d' % y if y > 9999 else '-%06d' % -y)
    return '%s-%02d-%02dT%02d:%02d:%02d.%03dZ' % (year, m, d, h, mi, s, msec)


def iso_of_updated(v):
    """TS isoOf(number): seconds below 1e11, else milliseconds (the raw value is also what freshness uses)."""
    return iso_of_ms(v * 1000 if v < 1e11 else v)


def js_str_number(s):
    t = es_trim(s)
    if t == '':
        return 0.0
    if re.fullmatch(r'[+-]?Infinity', t):
        return float('-inf') if t[0] == '-' else float('inf')
    if re.fullmatch(r'[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?', t):
        return float(t)
    for pat, base in ((r'0[xX][0-9a-fA-F]+', 16), (r'0[bB][01]+', 2), (r'0[oO][0-7]+', 8)):
        if re.fullmatch(pat, t):
            return float(int(t[2:], base))
    return float('nan')


def js_number(v):
    """What TS `Number(row.updated)` gives for a typed SQL value (a BLOB is a Uint8Array: Number('1,2,3'))."""
    k = v['kind']
    if k == 'null':
        return 0.0
    if k == 'integer':
        return float(int(v['decimal']))
    if k == 'real':
        return struct.unpack('>d', bytes.fromhex(v['bits']))[0]
    if k == 'text':
        return js_str_number(v['text'])
    return js_str_number(','.join(str(b) for b in bytes.fromhex(v['hex'])))


def pruv_updated(v):
    """P9 `rows::updated_number`: safe INTEGER; finite integral safe REAL; trimmed TEXT [+-]?[0-9]+ within the safe range; else 0."""
    k = v['kind']
    if k == 'integer':
        return int(v['decimal'])
    if k == 'real':
        x = struct.unpack('>d', bytes.fromhex(v['bits']))[0]
        return int(x) if math.isfinite(x) and x == math.floor(x) and abs(x) <= SAFE else 0
    if k == 'text' and 'text' in v:
        s = es_trim(v['text'])
        if re.fullmatch(r'[+-]?[0-9]+', s):
            n = int(s)
            return n if abs(n) <= SAFE else 0
    return 0


def ts_updated(v):
    x = js_number(v)
    return x if math.isfinite(x) else 0


# ---- export bytes: P1 / P4 / P5 triggers ---------------------------------------------------------------------------------
STR = rb'"[^"\\]*(?:\\.[^"\\]*)*"'                  # unrolled string: linear and fast on multi-megabyte strings
TOKEN = re.compile(STR + rb'|[\[\]{}:,]|-?[0-9][0-9.eE+\-]*|true|false|null|\s+', re.S)
JSON_VALUE = re.compile(STR + rb'|-?[0-9][0-9.eE+\-]*|true|false|null', re.S)
DEPTH_TOKEN = re.compile(STR + rb'|[\[{]|[\]}]|-?[0-9][0-9.eE+\-]*|true|false|null', re.S)


def deep_or_invalid(data):
    """P1 trigger: invalid UTF-8, nesting over 128, or more than 1,048,576 tokens (an upper-level count of the six-word tape records)."""
    try:
        data.decode('utf-8')
    except UnicodeDecodeError:
        return True
    depth = tokens = 0
    for m in DEPTH_TOKEN.finditer(data):
        t = m.group(0)
        tokens += 1
        if t in (b'[', b'{'):
            depth += 1
            if depth > 128:
                return True
        elif t in (b']', b'}'):
            depth -= 1
    return tokens > 1048576


def p4_supported(token):
    """P4: safe lexical integers, trimmed unsigned digit strings, exact Gregorian YYYY-MM-DDTHH:mm:ss.sssZ (0000-9999); blank = no date.
    A value whose millisecond time falls outside years 0000-9999 is also outside the shared `time` domain."""
    text = token.decode('utf-8', 'replace')
    lo, hi = -62167219200000, 253402300799999

    def in_domain(n):
        ms = n * 1000 if n < 100000000000 else n
        return lo <= ms <= hi
    if text.startswith('"'):
        try:
            s = json.loads(text)
        except ValueError:
            return True
        s = es_trim(s)
        if s == '':
            return True
        if re.fullmatch(r'\d+', s):
            return in_domain(int(s))
        m = re.fullmatch(r'(\d{4})-(\d\d)-(\d\d)T(\d\d):(\d\d):(\d\d)\.(\d{3})Z', s)
        if not m:
            return False
        y, mo, d, h, mi, sec, _ = map(int, m.groups())
        leap = y % 4 == 0 and (y % 100 != 0 or y % 400 == 0)
        days = [31, 29 if leap else 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        return 1 <= mo <= 12 and 1 <= d <= days[mo - 1] and h < 24 and mi < 60 and sec < 60
    if text in ('true', 'false', 'null'):
        return True                                # not a date token at all: TS and Pruv both read no date
    if not re.fullmatch(r'-?(0|[1-9]\d*)', text):
        return False
    n = int(text)
    return abs(n) <= SAFE and in_domain(n)


def p4_edit(data):
    """Raw export bytes with every unsupported info.time.created / info.time.completed token replaced by the string "invalid": TS reads
    no date from it (as Pruv) yet it is not nullish, so completed falls back to created for the plan time only through isoOf's undefined.
    A tokenising scan (strings skipped as units), so text that merely contains the words is never touched. Unscannable input: unchanged."""
    if b'"created"' not in data and b'"completed"' not in data:
        return data
    out, last = [], 0
    stack = []                                      # per container: ['o', current key, expecting] or ['a']
    key_stack = []
    pos = 0
    n = len(data)
    try:
        while pos < n:
            m = TOKEN.match(data, pos)
            if not m:
                return data
            t, start, pos = m.group(0), pos, m.end()
            if t.isspace():
                continue
            top = stack[-1] if stack else None
            if t in (b'{', b'['):
                stack.append(['o', None, 'key'] if t == b'{' else ['a', None, 'val'])
                key_stack.append(top[1] if top and top[0] == 'o' else None)
            elif t in (b'}', b']'):
                if stack:
                    stack.pop()
                    key_stack.pop()
            elif t == b':':
                if top and top[0] == 'o':
                    top[2] = 'val'
            elif t == b',':
                if top and top[0] == 'o':
                    top[2] = 'key'
            elif top and top[0] == 'o' and top[2] == 'key' and t.startswith(b'"'):
                top[1] = json.loads(t.decode('utf-8'))
            elif JSON_VALUE.fullmatch(t) and top and top[0] == 'o' and top[2] == 'val':
                parent_key = key_stack[-1]
                if top[1] in ('created', 'completed') and parent_key == 'time' and not p4_supported(t):
                    out.append(data[last:start])
                    out.append(b'"invalid"')
                    last = pos
    except (ValueError, UnicodeDecodeError):
        return data
    out.append(data[last:])
    return b''.join(out)


def export_bytes(case):
    exp = case.get('export')
    if not isinstance(exp, dict) or 'bytes' not in exp:
        return None
    return bytes_of(exp['bytes'], case.get('blobs'))


def baseline_cases(run):
    """TS reruns with only the unsupported export timestamp tokens changed (P4). {} when the case has none."""
    data = export_bytes(run)
    if data is None or run.get('mode') != 'read' or len(data) > 4 * 1024 * 1024:
        return {}
    new = p4_edit(data)
    if new == data:
        return {}
    return {'p4': dict(run, name=run['name'] + '-p4', export=dict(run['export'], bytes={'hex': new.hex()}))}


def db_texts(case):
    db = case.get('db') or {}
    out = []
    for ins in db.get('inserts', []):
        for row in ins.get('rows', []):
            out.extend(v['text'] for v in row if isinstance(v, dict) and v.get('kind') == 'text')
    return out


def triggers(case):
    """The policies whose trigger input is present in the case (detected from the recipe, never declared)."""
    t = set()
    data = export_bytes(case)
    exited_ok = (case.get('export') or {}).get('mode', 'output') == 'output' and (case.get('export') or {}).get('exit', 0) == 0
    if data is not None and case.get('mode') == 'read':
        if deep_or_invalid(data):
            t.add('P1')
        if re.search(rb'\\u[dD][89a-fA-F][0-9a-fA-F]{2}|[\xf0-\xf4]', data):
            t.add('P2')
            t.add('P8')
        if b'\xef\xbf\xbd' in data or re.search(rb'\\u[fF][fF][fF][dD]', data):
            t.add('P3')
        if len(data) <= 4 * 1024 * 1024 and p4_edit(data) != data:
            t.add('P4')
        if PARSER_MAX < len(data) <= EXPORT_MAX and exited_ok:
            t.add('P5')
    texts = db_texts(case)
    if any(re.search('[\ud800-\udfff\U00010000-\U0010ffff]', x) for x in texts):
        t.add('P2')
    plan = case.get('planJson')
    if isinstance(plan, str) and re.search(r'\\u[dD][89a-fA-F]|[\U00010000-\U0010ffff]', plan):
        t.update(('P2', 'P8'))
    if case.get('mode') in ('list', 'rows'):
        t.add('P9?')                                 # decided from the recorded SQL rows in explain_p9
    return t


# ---- value transformers (applied to the TS side; the comparison afterwards must be exact) ---------------------------------
def source_strings(case):
    """Every source string of the case (export JSON keys/strings, DB texts, planJson strings), for cut_astral and p3."""
    out = []

    def walk(x):
        if isinstance(x, str):
            out.append(x)
        elif isinstance(x, list):
            for i in x:
                walk(i)
        elif isinstance(x, dict):
            for k, i in x.items():
                walk(k)
                walk(i)
    data = export_bytes(case)
    if data is not None:
        try:
            walk(json.loads(data.decode('utf-8', 'replace')))
        except (ValueError, RecursionError):
            pass
    walk(db_texts(case))
    plan = case.get('planJson')
    if isinstance(plan, str):
        try:
            walk(json.loads(plan))
        except (ValueError, RecursionError):
            pass
    return out


def cut_astral(s, src):
    """True when the final lone high surrogate of `s` is the first half of an astral scalar in a source string (the cut split it),
    False when the source itself had that lone surrogate there. The whole of `s` (all 120 or 200 units, not just its end) must
    occur in one source string, unit-aligned, followed there by a low surrogate, so an unrelated astral scalar cannot excuse a
    deleted original lone surrogate."""
    if not HIGH_END.search(s):
        return False
    whole = s.encode('utf-16-le', 'surrogatepass')
    for x in src:
        b = x.encode('utf-16-le', 'surrogatepass')
        i = b.find(whole)
        while i >= 0:
            j = i + len(whole)
            if i % 2 == 0 and j + 2 <= len(b) and 0xDC00 <= int.from_bytes(b[j:j + 2], 'little') <= 0xDFFF:
                return True
            i = b.find(whole, i + 1)
    return False


def p2(v, src=()):
    """P2: lone surrogates become U+FFFD; a 120-unit title cut that split an astral scalar loses the lone high surrogate (only when the
    source really had an astral scalar there: an original lone surrogate becomes U+FFFD like any other)."""
    def title(x):
        if isinstance(x, dict):
            return {k: (HIGH_END.sub('', s) if k == 'title' and isinstance(s, str) and utf16_len(s) == 120 and cut_astral(s, src) else title(s))
                    for k, s in x.items()}
        if isinstance(x, list):
            return [title(i) for i in x]
        return x
    return strings(title(v), lambda s: LONE.sub('\ufffd', s))


def p8(v, src=()):
    """P8: a plan item text of exactly 200 UTF-16 units whose cut split an astral scalar of the source loses that lone high surrogate."""
    if isinstance(v, dict):
        out = {k: p8(x, src) for k, x in v.items()}
        if isinstance(out.get('items'), list):
            out['items'] = [dict(i, text=HIGH_END.sub('', i['text']))
                            if isinstance(i, dict) and isinstance(i.get('text'), str) and utf16_len(i['text']) == 200 and cut_astral(i['text'], src) else i
                            for i in out['items']]
        return out
    if isinstance(v, list):
        return [p8(x, src) for x in v]
    return v


def clipped_here(a, src):
    """True when the Pruv string `a` (ending in a genuine U+FFFD) is the clip of a longer joined text: `a` splits into J + T where
    J is empty or ends with the two-LF part separator, T (which holds the trailing U+FFFD) is a prefix of one source string, and
    that source string continues past T (the 32,768-byte cut really happened right after that U+FFFD inside that part)."""
    starts = [0] + [m.end() for m in re.finditer('\n\n', a)]
    for k in starts:
        t = a[k:]
        if not t:
            continue
        for x in src:
            if len(x) > len(t) and x.startswith(t):
                return True
    return False


def p3(actual, expected, src=()):
    """P3: TS removes a genuine trailing U+FFFD after the 32,768-byte clip. Only a turn prompt/reply that was actually cut, whose Pruv
    value ends in that U+FFFD at 32,765..32,768 bytes, is compared with the trailing U+FFFD run removed from the Pruv side only."""
    a_rows, e_rows = actual.get('turns'), expected.get('turns')
    if not (isinstance(a_rows, list) and isinstance(e_rows, list) and len(a_rows) == len(e_rows)):
        return actual, expected
    na, ne = [], []
    for a, e in zip(a_rows, e_rows):
        if isinstance(a, dict) and isinstance(e, dict):
            a, e = dict(a), dict(e)
            for k in ('prompt', 'reply'):
                x, y = a.get(k), e.get(k)
                if isinstance(x, str) and isinstance(y, str) and x.endswith('\ufffd') and not y.endswith('\ufffd') \
                        and 32765 <= len(x.encode('utf-8', 'surrogatepass')) <= 32768 and x.rstrip('\ufffd') == y and clipped_here(x, src):
                    a[k] = y
        na.append(a)
        ne.append(e)
    return dict(actual, turns=na), dict(expected, turns=ne)


def select_rows(side):
    """The SQL-selected rows recorded in a run's trace (typed, in SQL order): the `all` result of the selection statement."""
    for e in side.get('trace', []):
        if e.get('event') == 'complete' and e.get('op') == 'db_query' and not e.get('error') and isinstance(e.get('result'), dict):
            cols = e['result'].get('columns')
            if cols == ['id', 'directory', 'title', 'updated'] and 'rows' in e['result']:      # the execute result, not the prepare's column list
                return e['result']['rows']
    return None


def p9(case, ts_side, expected):
    """P9: recompute updatedMs / updatedAt / maybeOpen of the TS result from the ORIGINAL selected rows (independent Pruv numeric rule).
    Returns (new expected, changed?). Never re-runs SQL and never touches order or row selection."""
    rows = expected.get('rows')
    sel = select_rows(ts_side)
    if not isinstance(rows, list) or sel is None or expected.get('mode') not in ('list', 'rows'):
        return expected, False
    valid = [r for r in sel if r[0].get('kind') == 'text' and 'text' in r[0] and THREAD.match(r[0]['text'])
             and r[1].get('kind') == 'text' and r[1].get('text', 'x') != '']
    if len(valid) != len(rows):
        return expected, False
    now = case.get('nowMs')
    out, changed = [], False
    for srow, row in zip(valid, rows):
        pms = pruv_updated(srow[3])
        new = dict(row)
        if expected['mode'] == 'rows':
            new['updatedMs'] = pms
        else:
            new.pop('updatedAt', None)
            new.pop('maybeOpen', None)
            at = iso_of_updated(pms) if pms > 0 else None
            if at is not None:
                new['updatedAt'] = at
            if pms > 0 and now is not None and now - pms < 120000:
                new['maybeOpen'] = True
        changed = changed or not eq(new, row)
        out.append(new)
    return dict(expected, rows=out), changed


# ---- result normalisation ------------------------------------------------------------------------------------------------
def norm_result(res, roots):
    """Envelope -> comparable form: logical roots; errors reduced to stage/class/code/rawCode/native (+ detail columns|json|shape)."""
    if res is None:
        return None
    r = strings(copy.deepcopy(res), lambda s: logical_path(s, roots))
    if r.get('status') == 'error':
        e = r.get('error') or {}
        view = {'stage': e.get('stage'), 'class': e.get('class'), 'code': e.get('code'), 'rawCode': e.get('rawCode'), 'native': e.get('native', 0)}
        if e.get('detail') in ('columns', 'json', 'shape'):
            view['detail'] = e['detail']
        return {'status': 'error', 'mode': r.get('mode'), 'error': view}
    return r


POST_READ_STAGES = ('parse', 'shape', 'convert', 'plan', 'result')


def explain_result(case, ts_side, tr, ir, trig, baseline):
    """None when equal, else the list of policies that make `ir` the expected result; False when unexplained."""
    names = []
    t = tr
    src = source_strings(case) if trig & {'P2', 'P3', 'P8'} else ()

    def transforms(x, base_side):
        local = []
        if tr.get('status') != 'ok' or x.get('status') != 'ok':
            return x, local
        if 'P8' in trig:
            n = p8(x, src)
            if n != x:
                local.append('P8')
                x = n
        if 'P2' in trig:
            n = p2(x, src)
            if n != x:
                local.append('P2')
                x = n
        if 'P9?' in trig:
            n, changed = p9(case, base_side, x)
            if changed:
                local.append('P9')
                x = n
        return x, local

    t, local = transforms(tr, ts_side)
    names += local
    a, t3 = ir, t
    if 'P3' in trig and t.get('status') == 'ok' and ir.get('status') == 'ok':
        a, t3 = p3(ir, t, src)
        if (a, t3) != (ir, t):
            names.append('P3')
    if eq(t3, a):
        return names
    if 'P4' in trig and baseline is not None:
        b, bl = transforms(baseline, ts_side)
        a4, b4 = p3(ir, b, src) if 'P3' in trig and b.get('status') == 'ok' and ir.get('status') == 'ok' else (ir, b)
        if eq(b4, a4):
            return sorted(set(names + bl + ['P4']))
    err = ir.get('error') or {}
    # P1/P5 only replace a parse that TypeScript actually reached: TS succeeded or failed at parse or later (an earlier TS error such as
    # output-close EIO must stay an exact mismatch), and Pruv failed at its parse stage, not earlier.
    ts_parsed = tr.get('status') == 'ok' or (tr.get('status') == 'error' and (tr.get('error') or {}).get('stage') in POST_READ_STAGES)
    pruv_parse = ir.get('status') == 'error' and err.get('stage') == 'parse'
    if 'P1' in trig and ts_parsed and pruv_parse and err.get('class') == 'EngineImportError' and err.get('code') == 'read-failed' \
            and err.get('detail') == 'json':
        return sorted(set(names + ['P1']))
    if 'P5' in trig and ts_parsed and pruv_parse and err.get('class') == 'CoreError' and err.get('code') == 'InputTooLarge':
        return sorted(set(names + ['P5']))
    return False


# ---- fake exporter transcript, trace views ---------------------------------------------------------------------------------
def exporter_view(side):
    log = side.get('exporter')
    if log is None:
        return None
    return {'argv': log.get('argv'), 'violations': log.get('violations', []), 'fd1': (log.get('fd1') or {}).get('regular'),
            'written': log.get('written'), 'exit': log.get('exit')}


def trace_view(events, roots):
    out = normalize_trace(events, roots)
    for e in out:
        if e.get('op') in ('clock_now', 'observe'):
            e['stage'] = None
        a = e.get('args')
        if e.get('op') == 'spawn' and isinstance(a, dict) and isinstance(a.get('env'), list):
            a['env'] = [kv for kv in a['env'] if kv[0] not in NOISE_ENV]
    return out


def temp_tags(events):
    """{random mkdtemp suffix: '@<k>'} in creation order, as normalize_trace names them."""
    tags = {}
    for e in events or []:
        if e.get('op') == 'mkdtemp' and e.get('event') == 'complete' and not e.get('error'):
            m = re.search(r'opencode-([A-Za-z0-9]{6})$', str((e.get('result') or {}).get('path') or ''))
            if m and m.group(1) not in tags:
                tags[m.group(1)] = '@%d' % (len(tags) + 1)
    return tags


def tree_view(tree, tags=None):
    out = {}
    for k, v in tree.items():
        for suffix, tag in (tags or {}).items():
            k = re.sub(r'(^|/)opencode-%s(?=/|$)' % re.escape(suffix), lambda m, tag=tag: m.group(1) + 'opencode-' + tag, k)
        if k.endswith('-shm') and v.get('kind') == 'file':
            v = {x: y for x, y in v.items() if x != 'sha256'}
        out[k] = v
    return out


def spawned_to_close(events):
    spawn_ok = any(e.get('op') == 'spawn' and e.get('event') == 'complete' and not e.get('error') for e in events)
    closed = any(e.get('op') == 'observe' and e.get('event') == 'complete' and (e.get('result') or {}).get('eventKind') == 1 for e in events)
    return spawn_ok and closed


# ---- the comparison ---------------------------------------------------------------------------------------------------------
def compare(case, ts, impl, base=None):
    diffs = []
    trig = triggers(case)
    meta = ts.get('meta') or {}
    if meta.get('node') != '26.4.0':
        diffs.append(D('environment', 'TS reference ran on Node %s, not 26.4.0' % meta.get('node')))
    for name, side in (('ts', ts), ('impl', impl)):
        if side.get('watchdog'):
            diffs.append(D('watchdog', '%s killed by the watchdog (a failed observation, never a success)' % name, None, 'diff'))
        elif side.get('returncode') not in (0, None):
            diffs.append(D('runner', '%s exited with %s: %s' % (name, side['returncode'], side.get('stderr', '')[-300:])))
        if side.get('result') is None and not side.get('watchdog'):
            diffs.append(D('runner', '%s printed no result' % name))
        elif isinstance((side.get('result') or {}).get('status'), str) and side['result']['status'] not in ('ok', 'error'):
            diffs.append(D('harness', '%s: %s' % (name, json.dumps(side['result'])[:300])))
        elif ((side.get('result') or {}).get('error') or {}).get('class') == 'HarnessError':
            diffs.append(D('harness', '%s: HarnessError %s' % (name, (side['result']['error'].get('detail') or '')[:300])))
    if any(d['area'] in ('harness',) for d in diffs) or ts.get('result') is None or impl.get('result') is None:
        return diffs
    if ts['before'] != impl['before']:
        diffs.append(D('setup', 'the two sides built different trees or database images'))

    tr, ir = norm_result(ts['result'], ts['roots']), norm_result(impl['result'], impl['roots'])
    baseline = None
    b = (base or {}).get('p4') if isinstance(base, dict) else None
    if b and b.get('result'):
        baseline = norm_result(b['result'], b['roots'])
    result_policy = None
    if not eq(tr, ir):
        why = explain_result(case, ts, tr, ir, trig, baseline)
        if why is not False and why:
            result_policy = '+'.join(why)
            diffs.append(D('result', 'explained by ' + result_policy, result_policy, 'note'))
        else:
            detail = 'ts=%s impl=%s' % (json.dumps(tr, default=str)[:400], json.dumps(ir, default=str)[:400])
            if 'P4' in trig and baseline is None:
                detail += ' (P4 trigger but no baseline rerun was supplied)'
            diffs.append(D('result', detail))

    # primitive trace -----------------------------------------------------------------------------------------------------
    try:
        tt, it = trace_view(ts['trace'], ts['roots']), trace_view(impl['trace'], impl['roots'])
    except TraceAssertion as exc:
        diffs.append(D('trace-normalisation', str(exc)))
        tt = it = None
    if tt is not None and tt != it:
        at = next((i for i, (a, c) in enumerate(zip(tt, it)) if a != c), min(len(tt), len(it)))
        a, c = (tt[at] if at < len(tt) else None), (it[at] if at < len(it) else None)
        if isinstance(a, dict) and isinstance(c, dict) and isinstance(a.get('args'), dict) and isinstance(c.get('args'), dict):
            # Name the differing argument fields first; a long env list otherwise hides the difference past the cut.
            keys = sorted(set(a['args']) | set(c['args']))
            for k in keys:
                x, y = a['args'].get(k), c['args'].get(k)
                if x != y:
                    if isinstance(x, list) and isinstance(y, list):
                        xs, ys = [json.dumps(e) for e in x], [json.dumps(e) for e in y]
                        only = 'only ts=%s only impl=%s' % ([e for e in xs if e not in ys][:8], [e for e in ys if e not in xs][:8])
                        diffs.append(D('trace', 'record %d args.%s differs (order or members): %s' % (at, k, only[:800])))
                    else:
                        diffs.append(D('trace', 'record %d args.%s: ts=%s impl=%s' % (at, k, json.dumps(x)[:300], json.dumps(y)[:300])))
        diffs.append(D('trace', 'first difference at record %d of %d/%d: ts=%s impl=%s' % (at, len(tt), len(it), json.dumps(tt[at:at + 1])[:500], json.dumps(it[at:at + 1])[:500])))

    # fake exporter transcript ---------------------------------------------------------------------------------------------
    te, ie = exporter_view(ts), exporter_view(impl)
    for name, view, side in (('ts', te, ts), ('impl', ie, impl)):
        if view is None and spawned_to_close(side['trace']):
            diffs.append(D('exporter', '%s: the child ran to close but no exporter log exists (missing measurement)' % name))
        if view and view['violations']:
            diffs.append(D('exporter', '%s: %s' % (name, '; '.join(view['violations'][:3]))))
    if te != ie and te is not None and ie is not None:
        diffs.append(D('exporter', 'transcripts differ: ts=%s impl=%s' % (json.dumps(te), json.dumps(ie))))
    elif (te is None) != (ie is None):
        diffs.append(D('exporter', 'the fake exporter ran on one side only (ts=%s impl=%s)' % (te is not None, ie is not None)))

    # final trees ------------------------------------------------------------------------------------------------------------
    for part in ('src', 'home', 'tmp', 'folder'):
        a, c = tree_view(ts['after'][part], temp_tags(ts.get('trace'))), tree_view(impl['after'][part], temp_tags(impl.get('trace')))
        if a != c:
            bad = sorted(k for k in set(a) | set(c) if a.get(k) != c.get(k))
            diffs.append(D('tree', '%s differs at %s' % (part, bad[:5])))

    # coverage / domain notes ---------------------------------------------------------------------------------------------
    declared = set(case.get('policies') or [])
    deviations = {'P1', 'P2', 'P3', 'P4', 'P5', 'P8', 'P9'}
    missing = sorted(p for p in declared & deviations if p not in trig and not (p == 'P9' and 'P9?' in trig))
    if missing:
        diffs.append(D('policy-declared', 'declared %s but no trigger input was detected in the case' % '+'.join(missing), None, 'note'))
    return diffs
