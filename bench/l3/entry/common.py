"""Shared helpers of the L3-8 entry tests (oracle.py, test_impl.py): fixture loading, the JV codec helpers, the
expected-vs-actual comparison (DESIGN section 7) and the host binary paths. Synthetic data only."""
import sys
sys.dont_write_bytecode = True
import json
import os
import shutil
import struct
from pathlib import Path

HERE = Path(__file__).resolve().parent
WORK = HERE / 'work'
FIXTURES = HERE / 'fixtures'
NODE = os.environ.get('ENTRY_NODE') or shutil.which('node') or '/opt/homebrew/bin/node'
# PF: the Python compiler pruvc/ is frozen (bundle PF); the Rust compiler is the product. Default pairs are rs only;
# the py pairs stay valid (ALL_PAIRS) and are selected explicitly with --pairs py/rust,... as the frozen reference.
ALL_PAIRS = ('py_rust', 'py_llvm', 'rs_rust', 'rs_llvm')
PAIRS = ('rs_rust', 'rs_llvm')
IGNORED_KEYS = ('name', 'expect', 'probe', 'integration', 'group', 'parent', 'subs')


def host_path(pair):
    """work/<pair>/harness for pair in py_rust, py_llvm, rs_rust, rs_llvm."""
    if pair not in ALL_PAIRS:
        raise ValueError(f'unknown pair {pair!r}')
    return WORK / pair / 'harness'


# ---- JV helpers (DESIGN 5.1) ----
def jv_string(s):
    """A Python str as a JV string: plain JSON string when it is valid UTF-16, else {"u16":[units]}."""
    try:
        s.encode('utf-16-le')
        return s
    except UnicodeEncodeError:
        raw = s.encode('utf-16-le', 'surrogatepass')
        return {'u16': list(struct.unpack('<%dH' % (len(raw) // 2), raw))}


def jv_u16(units):
    return {'u16': list(units)}


def jv_num(x):
    """A float as {"num":"16hex"} (IEEE-754 bits); a safe integer stays a plain JSON number."""
    if isinstance(x, int) and abs(x) <= 2 ** 53 - 1:
        return x
    return {'num': struct.pack('>d', float(x)).hex()}


def jv_ref(name):
    return {'ref': name}


UNDEF = {'undef': 1}
ABSENT = {'absent': 1}


def jv_obj(pairs):
    return {'obj': [[k, v] for k, v in pairs]}


def jv_arr(items):
    return {'arr': list(items)}


def jv_text(jv):
    """Best-effort readable text of a JV string (for reports)."""
    if isinstance(jv, str):
        return jv
    if isinstance(jv, dict) and 'u16' in jv:
        return struct.pack('<%dH' % len(jv['u16']), *jv['u16']).decode('utf-16-le', 'replace')
    return json.dumps(jv, ensure_ascii=False)


# ---- fixtures ----
def _load_json(path):
    with open(path, encoding='utf-8') as f:
        return json.load(f)


def merge_case(defaults, case):
    """A case merged over the defaults: case keys win; dict-valued keys merge one level deep (script, args)."""
    merged = json.loads(json.dumps(defaults))
    for key, value in case.items():
        if isinstance(value, dict) and isinstance(merged.get(key), dict):
            merged[key].update(value)
        else:
            merged[key] = value
    return merged


def expand_case(case):
    """A fixture entry with "subs" (variants sharing name/op/args/script, each with its own full id and expect) becomes one
    case per sub, the sub's keys merged over the parent's; an entry without subs is itself."""
    if 'subs' not in case:
        return [case]
    parent = {k: v for k, v in case.items() if k != 'subs'}
    parent.pop('id', None)
    out = []
    for sub in case['subs']:
        out.append(merge_case(parent, sub))
    return out


def load_defaults():
    return _load_json(FIXTURES / '_defaults.json') if (FIXTURES / '_defaults.json').exists() else {}


def load_cases(group=None):
    """Cases of fixtures/<group>.json (or of every group file when group is None), subs expanded, each merged over
    fixtures/_defaults.json. Each case carries group (the file stem) and parent (the fixture entry id, e.g. D01 for D01/list)."""
    defaults = load_defaults()
    if group is None:
        files = sorted(p for p in FIXTURES.glob('*.json') if not p.name.startswith('_'))
    else:
        files = [FIXTURES / f'{group}.json']
    cases = []
    for path in files:
        for entry in _load_json(path):
            for case in expand_case(entry):
                merged = merge_case(defaults, case)
                merged.setdefault('group', path.stem)
                merged.setdefault('parent', entry['id'])
                cases.append(merged)
    return cases


def strip_case(case):
    """The case as sent to a runner: without name, expect, probe, integration."""
    return {k: v for k, v in case.items() if k not in IGNORED_KEYS}


# ---- comparison ----
NATIVE_PRIMITIVES = ('realpath', 'stat', 'homedir', 'cwd')


def _first_diff(a, b, path=''):
    if type(a) is not type(b):
        return path or '$'
    if isinstance(a, dict):
        for k in list(a) + [k for k in b if k not in a]:
            if k not in a or k not in b:
                return f'{path}.{k}'
            d = _first_diff(a[k], b[k], f'{path}.{k}')
            if d is not None:
                return d
        if list(a) != list(b):
            return f'{path}(key order)'
        return None
    if isinstance(a, list):
        for i in range(min(len(a), len(b))):
            d = _first_diff(a[i], b[i], f'{path}[{i}]')
            if d is not None:
                return d
        return f'{path}(length)' if len(a) != len(b) else None
    return None if a == b else (path or '$')


def _native_thrown(result):
    """True when the result's thrown error comes from a realpath/stat/homedir/cwd throw (its message is native prose)."""
    return result.get('terminal') in ('throw_plain_error', 'throw_import') and any(
        isinstance(t, list) and t and t[0] in NATIVE_PRIMITIVES for t in result.get('trace', []))


def _mask_messages(value):
    if isinstance(value, dict):
        return {k: ('<native>' if k == 'message' else _mask_messages(v)) for k, v in value.items()}
    if isinstance(value, list):
        return [_mask_messages(v) for v in value]
    return value


def _boundary_code(res):
    """The boundary error code of a result line (terminal boundary_error with a {"code":..} value), else None."""
    value = res.get('value')
    if res.get('terminal') == 'boundary_error' and isinstance(value, dict):
        return value.get('code')
    return None


def compare(expected, actual, *, probe_too_large=False, excluded=False, engine_policy=False):
    """Compare two result lines (id, terminal, value, trace, driver) exactly, no sorting. Returns (cls, first_field):
    cls in exact / P5 / P6 / engine-policy / excluded-domain / unexplained; first_field is None for exact.

    P6 is strict: both sides must be the InputTooLarge boundary error (terminal boundary_error, code InputTooLarge) with
    the same trace; only then is a remaining difference (for example the value's other fields) a resource-bound note.
    excluded-domain is strict: the caller passes excluded=True only when forbidden_reached(case) holds (the session holding
    the forbidden engine value is not excluded from the bound index, so the oracle really stringified it), the oracle
    completed (terminal ok), the actual side is the InvalidPacket boundary error, and the traces agree (the host made the
    same requests before it rejected)."""
    keys = ('terminal', 'value', 'trace')
    diffs = {k: _first_diff(expected.get(k), actual.get(k), k) for k in keys}
    first = next((d for d in diffs.values() if d is not None), None)
    if first is None:
        return 'exact', None
    if probe_too_large and _boundary_code(expected) == 'InputTooLarge' and _boundary_code(actual) == 'InputTooLarge' \
            and expected.get('trace') == actual.get('trace'):
        return 'P6', first
    if excluded and expected.get('terminal') == 'ok' and _boundary_code(actual) == 'InvalidPacket' \
            and expected.get('trace') == actual.get('trace'):
        return 'excluded-domain', first
    if engine_policy:
        return 'engine-policy', first
    if _native_thrown(expected) and _native_thrown(actual) \
            and expected.get('terminal') == actual.get('terminal') and expected.get('trace') == actual.get('trace') \
            and _mask_messages(expected.get('value')) == _mask_messages(actual.get('value')):
        return 'P5', first
    return 'unexplained', first


FORBIDDEN_ENGINE = {'ref': 'badeng'}


def forbidden_reached(case):
    """True when boundSessionIndex reaches the stringification of the forbidden engine value of an excluded_ok case:
    replays excludedFromBound (session.temporary === true || isTemporary(id)) over the sessions in order with the scripted
    isTemporary answers (default {ok:false}). A thrown answer ends the run first; an excluded forbidden session is skipped
    before sessionThreads, so in both cases the forbidden value is never stringified and the result is not excluded-domain."""
    if case.get('tag') != 'excluded_ok' or case.get('op') not in ('bound_session_index', 'bound', 9):
        return False
    queue = list((case.get('script') or {}).get('isTemporary') or [])
    for s in (case.get('args') or {}).get('sessions') or []:
        if not isinstance(s, dict):
            return False
        if s.get('temporary') is True:
            excluded = True
        else:
            ans = queue.pop(0) if queue else {'ok': False}
            if 'throw' in ans or not isinstance(ans.get('ok'), bool):
                return False   # a throw ends the run; a non-boolean answer is not modelled here (stays unexplained)
            excluded = ans['ok']
        et = s.get('externalThread')
        if isinstance(et, dict) and et.get('engine') == FORBIDDEN_ENGINE:
            return not excluded
    return False


# ---- probe expectations (shared by gen_fixtures.py and q.py so the derivation cannot drift) ----
def _probe_slots(trace, probe_script):
    """The ordered awaits of a run, from its oracle trace. Every await carries one request. account_at requests are awaits
    too but are not traced: each Claude account row costs one untraced account_at right after `accounts claude` (the Claude
    account body issues no other request). Codex account bodies interleave account_at with traced requests, so a non-empty
    codex row list appends a None marker and stops: a probe inside that phase cannot be derived from the trace alone."""
    slots = []
    for t in trace:
        slots.append(t)
        if t[:1] == ['accounts']:
            spec = (probe_script or {}).get('accounts', {}).get(t[1])
            spec = spec[0] if isinstance(spec, list) and spec else spec
            rows = spec.get('ok') if isinstance(spec, dict) else None
            if isinstance(rows, list):
                if t[1] == 'codex' and rows:
                    slots.append(None)
                    break
                slots.extend([UNTRACED] * len(rows))
    return slots


UNTRACED = object()


def probe_derivable(at, trace, probe_script=None):
    """True when a probe at step `at` lies within the derivable requests of the case."""
    slots = _probe_slots(trace, probe_script)
    return at <= len(slots) and None not in slots[:at]


def probe_expect(probe, oracle_res, probe_script=None):
    """Exact-limit probes expect the oracle; reentry expects InvalidPacket with the full trace.
    Other corruptions expect InvalidPacket (InputTooLarge for oversize probes) with the requests served before step `at`.
    Raises AssertionError when the probe is beyond the derivable requests."""
    c, k = probe['corrupt'], probe['at']
    trace = oracle_res['trace']
    if c in ('exact_limit', 'exact_limit_phys'):
        return {'terminal': oracle_res['terminal'], 'value': oracle_res['value'], 'trace': trace, 'driver': None}
    if c == 'reentry':
        return {'terminal': 'boundary_error', 'value': {'code': 'InvalidPacket'}, 'trace': trace, 'driver': None}
    code = 'InputTooLarge' if c in ('oversize', 'oversize_phys') else 'InvalidPacket'
    slots = _probe_slots(trace, probe_script)
    if not (k <= len(slots) and None not in slots[:k]):
        raise AssertionError(f'probe at {k} is beyond the derivable requests of the case')
    served = [t for t in slots[:k] if t is not UNTRACED]
    return {'terminal': 'boundary_error', 'value': {'code': code}, 'trace': served, 'driver': 'InvalidPacket' if c == 'duplicate' else None}
