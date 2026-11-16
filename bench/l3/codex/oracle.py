"""Narrow, result-checked exceptions for the Q differential. Only the policies named in docs/CODEX_IMPORT_SPEC.md
(P1-P5 as in QWEN_IMPORT_SPEC, P6 framing, P8 plan text) explain a difference, each only where it can arise;
nothing else is accepted.

classify(label, actual, expected, ...) -> 'exact' | 'P1-...' | ... | 'unexplained'.
`expected` is the pinned TypeScript result, `actual` the Pruv pair's. Normalisations are applied to the TS side only
and the comparison must then be exact (timestamps may only be missing on the Pruv side, never different)."""
import re
from datetime import datetime, timezone

LONE = re.compile('[\ud800-\udfff]')
TS_KEYS = ('updatedAt', 'createdAt', 'completedAt', 'at')


def map_strings(v, fn):
    if isinstance(v, str):
        return fn(v)
    if isinstance(v, list):
        return [map_strings(x, fn) for x in v]
    if isinstance(v, dict):
        return {k: map_strings(x, fn) for k, x in v.items()}
    return v


def p2(v):
    """P2: a lone surrogate in a value is U+FFFD on the Pruv side."""
    return map_strings(v, lambda s: LONE.sub('�', s))


def p3(actual, expected):
    """P3: TS strips trailing U+FFFD after a byte clip; Pruv keeps genuine ones. Only a turn's prompt or reply can be
    clipped (32768 bytes), and a clipped Pruv string is 32765..32768 UTF-8 bytes; just those pairs are compared with
    trailing U+FFFD stripped on both sides."""
    if not (isinstance(actual, dict) and isinstance(expected, dict) and isinstance(actual.get('turns'), list)
            and isinstance(expected.get('turns'), list) and len(actual['turns']) == len(expected['turns'])):
        return actual, expected
    at, et = [], []
    for a, e in zip(actual['turns'], expected['turns']):
        if isinstance(a, dict) and isinstance(e, dict):
            a, e = dict(a), dict(e)
            for k in ('prompt', 'reply'):
                if isinstance(a.get(k), str) and isinstance(e.get(k), str) and 32765 <= len(a[k].encode('utf-8', 'surrogatepass')) <= 32768:
                    a[k], e[k] = a[k].rstrip('\ufffd'), e[k].rstrip('\ufffd')
        at.append(a)
        et.append(e)
    return dict(actual, turns=at), dict(expected, turns=et)


def p8(v):
    """P8: the 200-unit plan slice drops a split high surrogate (TS leaves it)."""
    if isinstance(v, dict) and isinstance(v.get('lastPlan'), dict):
        out = dict(v)
        plan = dict(v['lastPlan'])
        plan['items'] = [dict(i, text=re.sub('[\ud800-\udbff]$', '', i['text'])) if isinstance(i, dict) and isinstance(i.get('text'), str) else i for i in plan.get('items', [])]
        out['lastPlan'] = plan
        return out
    return v


def same_mod_ts(actual, expected, isos):
    """Equal, except that Pruv may omit a timestamp field the TS result has when that TS value is not one Pruv could
    read from any raw timestamp in the case (P4); a timestamp the Pruv result has must be identical."""
    if isinstance(actual, dict) and isinstance(expected, dict):
        for k in set(actual) | set(expected):
            if k in TS_KEYS:
                if k in actual and (k not in expected or actual[k] != expected[k]):
                    return False
                if k not in actual and expected[k] in isos:
                    return False
            elif k not in actual or k not in expected or not same_mod_ts(actual[k], expected[k], isos):
                return False
        return True
    if isinstance(actual, list) and isinstance(expected, list):
        return len(actual) == len(expected) and all(same_mod_ts(a, e, isos) for a, e in zip(actual, expected))
    return actual == expected


ISO = re.compile(r'\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z')


def iso_of(v):
    """The canonical ISO string Pruv reads from a raw timestamp (P4: integers, seconds below 1e11 else milliseconds;
    trimmed unsigned digit strings as integers; canonical ISO strings as they are), or None."""
    if isinstance(v, str) and ISO.fullmatch(v):
        return v
    if isinstance(v, str) and v.strip().isdigit() and v.strip().isascii():
        v = int(v.strip())
    if not isinstance(v, int) or isinstance(v, bool) or abs(v) > 9007199254740991:
        return None
    ms = v * 1000 if v < 100000000000 else v
    try:
        d = datetime.fromtimestamp(ms / 1000, timezone.utc)
    except (OverflowError, OSError, ValueError):
        return None
    if not 0 <= d.year <= 9999:
        return None
    return d.strftime('%Y-%m-%dT%H:%M:%S.') + f'{ms % 1000:03d}Z'


def source_isos(case):
    """Every timestamp Pruv could produce from the case's raw server data."""
    out = set()

    def walk(v):
        if isinstance(v, dict):
            for k, x in v.items():
                if k in ('updatedAt', 'createdAt', 'startedAt', 'completedAt'):
                    iso = iso_of(x)
                    if iso:
                        out.add(iso)
                walk(x)
        elif isinstance(v, list):
            for x in v:
                walk(x)
    walk(case or {})
    return out


def plan_fallback(case):
    """P4 + rule 6: lastPlan.at is completedAt ?? createdAt after isoOf, so when P4 drops the plan turn's completedAt
    Pruv's at is that turn's createdAt. Returns (iso_of(startedAt), True) for the first plan turn when Pruv rejects its
    completedAt, else None."""
    case = case or {}
    acts = case['pages_actions'] if 'pages_actions' in case else [{'result': x} for x in case.get('pages', [])]
    if not isinstance(acts, list):
        return None
    for act in acts:
        if not isinstance(act, dict):
            continue
        data = ((act.get('result') or {}).get('data')) if isinstance(act.get('result'), dict) else None
        for t in data if isinstance(data, list) else []:
            items = t.get('items') if isinstance(t, dict) else None
            if not isinstance(items, list):
                continue
            if any(isinstance(i, dict) and i.get('type') == 'plan' and isinstance(i.get('text'), str) and i['text'].strip() for i in items):
                if iso_of(t.get('completedAt')) is not None:
                    return None
                return (iso_of(t.get('startedAt')), True)
    return None


def classify(label, actual, expected, case=None):
    if actual == expected:
        return 'exact'
    mode = label.split(':')[0]
    # P1 / P6: Pruv skips the line (invalid UTF-8) or ignores the response (id token is not exactly the decimal id),
    # so it waits and times out where TS answered.
    if mode == 'probe-P1' and actual == {'error': 'timeout'}:
        return 'P1-invalid-utf8'
    if mode == 'probe-P6' and actual == {'error': 'timeout'}:
        return 'P6-id-token'
    names = []
    e = expected
    # P8 first: the split high surrogate it drops would otherwise be turned into U+FFFD by P2
    for name, fn in (('P8-plan-cut', p8), ('P2-surrogate', p2)):
        n = fn(e)
        if n != e:
            names.append(name)
            e = n
    a, n = p3(actual, e)
    if (a, n) != (actual, e):
        names.append('P3-trailing-fffd')
        e = n
    if a == e and names:
        return '+'.join(names)
    isos = source_isos(case)
    if same_mod_ts(a, e, isos):
        return '+'.join(names + ['P4-timestamp'])
    # the plan turn's completedAt was dropped by P4, so Pruv's lastPlan.at fell back to its createdAt (rule 6)
    fb = plan_fallback(case)
    if fb and isinstance(a, dict) and isinstance(e, dict) and isinstance(a.get('lastPlan'), dict) and isinstance(e.get('lastPlan'), dict):
        ap = dict(a['lastPlan'])
        if ap.get('at') == fb[0] and ap.get('at') != e['lastPlan'].get('at'):
            ap.pop('at', None)
            ep = dict(e['lastPlan'])
            ep.pop('at', None)
            if same_mod_ts(dict(a, lastPlan=ap), dict(e, lastPlan=ep), isos):
                return '+'.join(names + ['P4-timestamp'])
    return 'unexplained'
