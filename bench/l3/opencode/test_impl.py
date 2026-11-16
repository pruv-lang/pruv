#!/usr/bin/env python3
"""L3-7 OpenCode import: builds the four verified pairs (py/rust, py/llvm, rs/rust, rs/llvm), runs every fixture of
fixtures/_defaults.json + fixtures/F*.json on the pinned TypeScript (node reference.mjs) and on each pair, and compares them with
oracle.compare (result, primitive trace, final trees). A difference passes only when oracle.py names a policy whose trigger is in
the fixture. Only this piece is built; no repository-wide suites.

    PATH="$HOME/.cargo/bin:$PATH" GROK_TS_TRANSPILER=/path/to/typescript python3 bench/l3/opencode/test_impl.py
        [--pairs py/rust,rs/llvm] [--only SUBSTRING]... [--scope A|B|C|D] [--unit packet|host|export|convert]
        [--verify-only] [--reference-only] [--slow] [--jobs N] [--keep] [--list]

  --verify-only    true verification, no host linking, no execution: root.pruv and everything it reaches (its manifest packages and
                   local modules) through each selected compiler (the same py/rs command lines the builds use); the command lines
                   and outputs are logged to work/logs/verify_*.log. `--verify-only` is passed to the compiler when it accepts it;
                   otherwise the verified compile is emitted into a throwaway work/verify directory (never `--no-verify`).
  --scope A|B|C|D  verify only that package's modules through a temporary entry/manifest in work/scope_<X>
                   (A util packet; B + path admit sql rows list; C + export; D + entry convert plan).
  --unit host      host unit tests: SQLite typed round trip, wx/stdout fd + spawn/observe, delta patches, result rendering, protocol
                   rejections (needs only cargo/rustc; no Pruv compiler). packet/export/convert select the fixture groups that
                   exercise those modules on the selected pairs (default rs/rust).
  --reference-only run only the pinned TypeScript and check each fixture's `expected` block.
  --slow           include `slow` fixtures (the real 20 s deadline). Timer-sensitive cases always run serially.

This module is also the fixture library of q.py and q_mutants.py (load_fixtures, expand_case, run_reference, judge, build_pair ...).

Fixture files: a JSON array of cases per file; _defaults.json supplies only OMITTED top-level fields (an explicit empty string or
object is kept; env.mode/env.values fill individually). Conveniences expanded here (the host/reference/oracle see none of them):
  export.bytes / tree bytes / schedule action bytes: {"json": VALUE[, "size": N]} -> JSON text of VALUE; {"$raw": TEXT} inside it is
      inserted verbatim (1e400, escaped surrogates, invalid JSON); "size" pads a dict VALUE with a "pad" key to exactly N bytes.
      Plain recipes {"utf8"} {"hex"} {"repeat","count"} {"concat"} stay as they are.
  {"$rep": "x", "n": 129} / {"$cat": [A, B]} in any string field.
  "kills": ["M01"]  mutants this fixture should kill (q_mutants.py)   "disabled": "why"   "slow": true   "serial": true
  "expected": {status, class, code, stage, rowCount, omitted, title, model, planPresent, value}  checked on the pinned TypeScript.
"""
import sys
sys.dont_write_bytecode = True
import argparse
import copy
import importlib.util
import inspect
import json
import os
import re
import shutil
import subprocess
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from common import (HERE, WORK, TraceAssertion, bytes_of, clean, normalize_trace, place_db, place_exporter, read_exporter_log,
                    reference_command, run_side, tokens_of)

FIXTURES = HERE / 'fixtures'
PAIRS = [(c, b) for c in ('py', 'rs') for b in ('rust', 'llvm')]
SCOPES = {'A': ['util', 'packet'], 'B': ['util', 'packet', 'path', 'admit', 'sql', 'rows', 'list'], 'C': ['util', 'packet', 'export'],
          'D': ['util', 'packet', 'entry', 'convert', 'plan']}
UNIT_SELECT = {'packet': ['F16-data-dir', 'F16-folder', 'F16-plan', 'F01-xdg'],
               'export': ['F07', 'F08', 'F09-nonzero-oversized'],
               'convert': ['F09', 'F10', 'F11', 'F12', 'F13', 'F14', 'F15', 'F16-plan']}


# ---- fixtures: expansion of the conveniences ---------------------------------------------------------------------
def dump(v):
    """JSON text of v. {"$raw": TEXT} is inserted verbatim; nothing round-trips through float."""
    if isinstance(v, dict):
        if '$raw' in v:
            return v['$raw']
        return '{' + ','.join(json.dumps(k, ensure_ascii=False) + ':' + dump(x) for k, x in v.items()) + '}'
    if isinstance(v, list):
        return '[' + ','.join(dump(x) for x in v) + ']'
    return json.dumps(v, ensure_ascii=False)


def macro(v):
    if isinstance(v, dict):
        if set(v) == {'$rep', 'n'}:
            return v['$rep'] * v['n']
        if set(v) == {'$cat'}:
            return ''.join(macro(x) for x in v['$cat'])
        return {k: macro(x) for k, x in v.items()}
    if isinstance(v, list):
        return [macro(x) for x in v]
    return v


def fix_bytes(b):
    """{"json": V[, "size": N]} -> {"utf8": TEXT}; recurses into concat/repeat."""
    if not isinstance(b, dict):
        return b
    if 'json' in b:
        v = b['json']
        if 'size' in b:
            base = dump(dict(v, pad=''))
            need = b['size'] - len(base.encode('utf-8'))
            if need < 0:
                raise ValueError('size %d too small for the document (%d bytes)' % (b['size'], len(base.encode('utf-8'))))
            return {'utf8': dump(dict(v, pad='x' * need))}
        return {'utf8': dump(v)}
    if 'concat' in b:
        return {'concat': [fix_bytes(x) for x in b['concat']]}
    if 'repeat' in b:
        return {'repeat': fix_bytes(b['repeat']), 'count': b['count']}
    return b


def merge_defaults(case, defaults):
    out = copy.deepcopy(case)
    for k, v in defaults.items():
        if k not in out:
            out[k] = copy.deepcopy(v)
    if isinstance(out.get('env'), dict) and isinstance(defaults.get('env'), dict):
        for k, v in defaults['env'].items():
            out['env'].setdefault(k, copy.deepcopy(v))
    return out


def expand_case(case, defaults=None):
    c = macro(merge_defaults(case, defaults or {}))
    for e in c.get('tree', []):
        if 'bytes' in e:
            e['bytes'] = fix_bytes(e['bytes'])
    if isinstance(c.get('export'), dict) and 'bytes' in c['export']:
        c['export']['bytes'] = fix_bytes(c['export']['bytes'])
    for r in (c.get('schedule') or {}).get('rules', []):
        for a in r.get('actions', []):
            if 'bytes' in a:
                a['bytes'] = fix_bytes(a['bytes'])
    return c


def prepare(case):
    """The copy that is run: bytes recipes of schedule actions become hex (host.rs reads only utf8/hex)."""
    c = copy.deepcopy(case)
    blobs = c.get('blobs', {})
    for r in (c.get('schedule') or {}).get('rules', []):
        for a in r.get('actions', []):
            if 'bytes' in a and not (isinstance(a['bytes'], dict) and set(a['bytes']) <= {'utf8', 'hex'}):
                a['bytes'] = {'hex': bytes_of(a['bytes'], blobs).hex()}
    return c


def load_fixtures(only=(), slow=False, include_disabled=False):
    """All fixtures of fixtures/F*.json, expanded; each gets `category` (file stem) and `group` (F04 ...). -> (cases, skipped)."""
    dpath = FIXTURES / '_defaults.json'
    defaults = json.loads(dpath.read_text(encoding='utf-8')) if dpath.exists() else {}
    cases, skipped, seen = [], [], set()
    for f in sorted(FIXTURES.glob('F*.json')):
        raw = json.loads(f.read_text(encoding='utf-8'))
        for c in (raw['cases'] if isinstance(raw, dict) else raw):
            c = dict(c, category=f.stem)
            if c['name'] in seen:
                raise ValueError('duplicate fixture name ' + c['name'])
            seen.add(c['name'])
            if only and not any(o in c['name'] for o in only):
                continue
            if c.get('disabled') and not include_disabled:
                skipped.append((c['name'], 'disabled: ' + str(c['disabled'])))
                continue
            if c.get('slow') and not slow:
                skipped.append((c['name'], 'slow (use --slow)'))
                continue
            c['group'] = c['name'].split('-')[0]
            cases.append(expand_case(c, defaults))
    return cases, skipped


# ---- oracle (imported lazily: --list, --unit host and --verify-only do not need it) ---------------------------------
def oracle():
    import oracle as o
    return o


def oracle_attr(name):
    try:
        return getattr(oracle(), name, None)
    except ImportError:
        return None


def timer_sensitive(case):
    exp = case.get('export') or {}
    return bool(case.get('slow') or case.get('serial') or exp.get('mode') == 'hang')


# ---- running and judging one case ------------------------------------------------------------------------------------
def run_reference(case, root=None):
    """(ts run, baselines or None). Baselines are TS reruns on the case with only a policy's trigger input edited, made by
    oracle.baseline_cases(run) -> {name: case} when oracle.py offers that hook (P4: unsupported timestamp tokens -> invalid date)."""
    run = prepare(case)
    ts = run_side(run, 'ts', reference_command(), root=root)
    base = {}
    hook = oracle_attr('baseline_cases')
    if hook:
        for name, b in (hook(run) or {}).items():
            base[name] = run_side(b, 'ts_' + name, reference_command())
    return ts, base or None


def run_pair(case, side, exe, root=None):
    return run_side(prepare(case), side, [exe], root=root)


def _fields(run):
    """The comparable summary of a stdout envelope."""
    r = run.get('result') or {}
    err = r.get('error') or {}
    rows = r.get('rows')
    if rows is None:
        rows = r.get('turns')
    plan = r.get('lastPlan') if r.get('mode') == 'read' else (r.get('value') if r.get('present') else None)
    return {'status': r.get('status'), 'class': err.get('class'), 'code': err.get('code'), 'stage': err.get('stage'),
            'rowCount': len(rows) if rows is not None else None, 'omitted': r.get('omittedBefore'), 'title': r.get('title'),
            'model': r.get('model'), 'planPresent': plan is not None,
            'value': r.get('value') if r.get('mode') in ('dataDir', 'folderKey') else None}


def check_expected(case, ts):
    """The `expected` block against the pinned TypeScript result (a fixture that does not do what it says is a fixture bug)."""
    exp = case.get('expected')
    if not exp:
        return []
    if ts.get('result') is None:
        return ['no result envelope (returncode %s%s)' % (ts.get('returncode'), ', watchdog' if ts.get('watchdog') else '')]
    got = _fields(ts)
    return ['expected %s=%r but the reference gave %r' % (k, v, got.get(k)) for k, v in exp.items() if got.get(k) != v]


def judge(case, ts, base, impl):
    """-> {ok, policies, notes, problems}: oracle.compare of the case; unexplained differences are problems."""
    if impl.get('returncode') not in (0, None) or impl.get('watchdog') or impl.get('result') is None:
        why = 'watchdog kill' if impl.get('watchdog') else 'exit %s' % impl.get('returncode')
        detail = ' | '.join(impl.get('other', [])[-3:])[:300] or ''.join(impl.get('stdout_lines', [])[:1])[:300]
        return {'ok': False, 'policies': [], 'notes': [], 'problems': ['runner failed (%s): %s' % (why, detail)]}
    o = oracle()
    cmp = o.compare
    diffs = cmp(case, ts, impl, base) if len(inspect.signature(cmp).parameters) >= 4 else cmp(case, ts, impl)
    bad = o.unexplained(diffs)
    return {
        'ok': not bad, 'problems': ['%s: %s' % (d['area'], d['detail'][:300]) for d in bad],
        'policies': sorted({p for d in diffs for p in (d.get('policy') or '').split('+') if p}),
        'notes': ['%s: %s' % (d['area'], d['detail'][:120]) for d in diffs if d.get('severity') != 'diff' or d.get('policy')],
    }


def impl_clean(impl):
    """A runner that exited normally and was not killed by the watchdog (crashes and watchdog kills are never a kill)."""
    return impl.get('returncode') in (0, None) and not impl.get('watchdog') and impl.get('result') is not None


# ---- tools and build -------------------------------------------------------------------------------------------------
def tool(name):
    return shutil.which(name) or str(Path.home() / '.cargo/bin' / name)


def sh(cmd, timeout=3600, log=None):
    """Run a command; raise RuntimeError with its output tail on failure. The command line goes to `log` when given."""
    cmd = [str(x) for x in cmd]
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    if log is not None:
        Path(log).parent.mkdir(parents=True, exist_ok=True)
        with open(log, 'a', encoding='utf-8') as f:
            f.write('$ %s\n[exit %d]\n%s%s\n' % (' '.join(cmd), p.returncode, p.stdout, p.stderr))
    if p.returncode:
        raise RuntimeError('%s: exit %d\n%s\n%s' % (' '.join(cmd[:4]), p.returncode, p.stdout[-1500:], p.stderr[-2500:]))
    return p


def configure():
    """The shared compiler driver: (supervisor common, g2 module with compile_pruv/build_llvm_objects/COMPILERS/RUSTC, run)."""
    spec = importlib.util.spec_from_file_location('super_common', HERE.parent / 'supervisor/common.py')
    c = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(c)
    g, run = c.configure(WORK)
    return c, g, run


def newest(paths):
    paths = list(paths)
    if not paths:
        raise FileNotFoundError('no matching rlib; build the host first')
    return max(paths, key=lambda p: p.stat().st_mtime)


def host_artifacts():
    deps = WORK / 'host/debug/deps'
    natives = [p for p in (WORK / 'host/debug/build').glob('libsqlite3-sys-*/out') if p.is_dir()]
    return deps, newest(deps.glob('libopencode_host-*.rlib')), WORK / 'libpruv_str.rlib', natives


def build_host(rustc=None, log=None):
    """Host crate (cargo, locked + offline: rusqlite =0.40.0 / libsqlite3-sys =0.38.2 bundled SQLite 3.53.2) and the string adapter."""
    sh([tool('cargo'), 'build', '--locked', '--offline', '--manifest-path', HERE / 'host/Cargo.toml', '--target-dir', WORK / 'host'], log=log)
    adapter = WORK / 'libpruv_str.rlib'
    sh([rustc or tool('rustc'), '--crate-name', 'pruv_str', '--crate-type', 'rlib', HERE / 'str_adapter.rs', '-o', adapter], log=log)
    return host_artifacts()


def rustc_link(rustc, main, exe, host, extra=()):
    deps, lib, adapter, natives = host
    cmd = [rustc, '-O', '--edition', '2021', main, '-o', exe, '--extern', f'opencode_host={lib}', '--extern', f'pruv_str={adapter}',
           '-L', f'dependency={deps}']
    for n in natives:
        cmd += ['-L', f'native={n}']
    sh(cmd + list(extra))


def compile_flags(adapter, manifest=None):
    return ['--rust-extern', f'pruv_str={adapter}', '--manifest', str(manifest or HERE / 'pruv.json')]


def link_runner(g, built, out, host):
    harness = out / 'main.rs'
    harness.write_text((HERE / 'harness.rs').read_text().replace('@SOL@', built['rs']))
    extra = []
    if built['ll']:
        extra, err = g.build_llvm_objects(built['ll'], 'opencode', str(out))
        assert not err, err
    exe = out / 'runner'
    rustc_link(g.RUSTC, harness, exe, host, extra)
    return exe


def build_pair(g, compiler, backend, host, reuse=False):
    """Verify + compile root.pruv (verified mode, G4b manifest, nothing passes via --no-verify) for one pair and link the runner."""
    out = WORK / f'{compiler}_{backend}'
    out.mkdir(parents=True, exist_ok=True)
    if reuse:
        built = {'rc': 0, 'rs': str(out / 'root_gen.rs'), 'll': str(out / 'root_gen.ll') if backend == 'llvm' else None}
        assert Path(built['rs']).is_file(), 'run once without --reuse-verified first'
    else:
        built = g.compile_pruv(compiler, str(HERE / 'root.pruv'), str(out), backend, compile_flags(host[2]))
    if built['rc']:
        raise RuntimeError('compile/verify failed: ' + str(built.get('errors') or built.get('stderr') or built))
    print(f'{compiler}/{backend}: ' + ('reusing verified Pruv emission' if reuse else 'verified'), flush=True)
    return link_runner(g, built, out, host)


# ---- verification only -------------------------------------------------------------------------------------------------
def pkg_deps():
    p = HERE / 'pruv.json'
    return json.loads(p.read_text(encoding='utf-8')) if p.exists() else {}


def reachable():
    """Root + the local modules it declares + the manifest packages (what one root verification covers)."""
    root = (HERE / 'root.pruv').read_text(encoding='utf-8') if (HERE / 'root.pruv').exists() else ''
    mods = re.findall(r'^\s*(?:pub\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;', root, re.M)
    deps = pkg_deps().get('dependencies', {})
    out = ['root.pruv']
    for m in mods:
        if m in deps:
            out.append('package %s -> %s' % (m, (HERE / deps[m]['path']).resolve()))
        else:
            out.append('%s.pruv' % m)
    return out


def scope_entry(scope):
    """work/scope_<X>/{scope.pruv, scope.json}: a temporary entry declaring the manifest packages plus only this package's modules."""
    d = WORK / f'scope_{scope}'
    d.mkdir(parents=True, exist_ok=True)
    man = pkg_deps()
    deps = man.get('dependencies', {})
    for dep in deps.values():
        dep['path'] = os.path.relpath((HERE / dep['path']).resolve(), d.resolve())
    man = dict(man, name=man.get('name', 'l3_opencode') + '_scope_' + scope, root='scope.pruv')
    (d / 'scope.json').write_text(json.dumps(man, indent=1) + '\n', encoding='utf-8')
    missing = [m for m in SCOPES[scope] if not (HERE / f'{m}.pruv').exists()]
    if missing:
        raise RuntimeError('scope %s: missing module file(s) %s (not delivered yet)' % (scope, ', '.join(m + '.pruv' for m in missing)))
    lines = ['mod %s;' % n for n in deps] + ['mod %s;' % m for m in SCOPES[scope]]
    (d / 'scope.pruv').write_text('\n'.join(lines) + '\n', encoding='utf-8')
    return d


_VERIFY_FLAG = {}


def verify_one(g, compiler, backend, root_file, manifest, libdir, log):
    """One true verification of root_file with one compiler. Returns (ok, line, output tail)."""
    base = list(g.COMPILERS[compiler])
    flags = compile_flags(WORK / 'libpruv_str.rlib', manifest)
    if libdir:
        flags += ['--lib', str(libdir)]
    out = WORK / 'verify' / f'{compiler}_{backend}'
    out.mkdir(parents=True, exist_ok=True)
    attempts = []
    if _VERIFY_FLAG.get(compiler, True):
        attempts.append(('--verify-only', base + [str(root_file), '--verify-only'] + flags))
    stem = Path(root_file).stem
    emit = out / (stem + '_gen.rs')
    if backend == 'llvm':
        fallback = base + [str(root_file), '-o', str(out / (stem + '_gen.ll')), '--backend', 'llvm', '--rust-bindings', str(emit)] + flags
    else:
        fallback = base + [str(root_file), '-o', str(emit)] + flags
    attempts.append(('verified compile to a throwaway directory', fallback))
    for label, cmd in attempts:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=1800)
        Path(log).parent.mkdir(parents=True, exist_ok=True)
        with open(log, 'a', encoding='utf-8') as f:
            f.write('$ %s   # %s\n[exit %d]\n%s%s\n' % (' '.join(cmd), label, p.returncode, p.stdout, p.stderr))
        if label == '--verify-only' and p.returncode == 2 and 'verify-only' in (p.stdout + p.stderr):
            _VERIFY_FLAG[compiler] = False                  # this compiler has no such flag: fall through to the verified compile
            continue
        return p.returncode == 0, '%s [%s] exit %d' % (' '.join(cmd[-6:]), label, p.returncode), (p.stdout + p.stderr)[-600:]
    return False, 'no attempt ran', ''


def verify_only(pairs, scope=None):
    _, g, _ = configure()
    (WORK / 'logs').mkdir(parents=True, exist_ok=True)
    rc = 0
    if scope:
        d = scope_entry(scope)
        root_file, manifest, libdir, tag = d / 'scope.pruv', d / 'scope.json', HERE, f'scope{scope}'
    else:
        root_file, manifest, libdir, tag = HERE / 'root.pruv', HERE / 'pruv.json', None, 'root'
        print('verifying root and what it reaches: ' + '; '.join(reachable()), flush=True)
    for compiler, backend in pairs:
        log = WORK / 'logs' / f'verify_{compiler}_{backend}_{tag}.log'
        if log.exists():
            log.unlink()
        ok, line, tail = verify_one(g, compiler, backend, root_file, manifest, libdir, log)
        print(f'{compiler}/{backend}: {"VERIFIED" if ok else "FAILED"} {tag}: {line}  (log {log})', flush=True)
        if not ok:
            rc = 1
            print(tail, flush=True)
    return rc


# ---- host unit tests (--unit host) ----------------------------------------------------------------------------------
MOCK_RS = r'''
use std::cell::RefCell;
use std::collections::VecDeque;
// Scripted Sol: HOST_UNIT_SCRIPT is a text file; line "B ints" = the packet opencode_begin returns, each "S ints" line = the next
// opencode_step result. Every step call prints {"unit":{...}} with the event/completion it received and a copy of the six banks
// (as they are when that call starts, i.e. after the patches of the previous packet).
struct M { first: Vec<i64>, steps: RefCell<VecDeque<Vec<i64>>>, n: RefCell<usize> }
fn hx(b: &[u8]) -> String { b.iter().map(|x| format!("{:02x}", x)).collect() }
fn ws(w: &[i64]) -> String { format!("[{}]", w.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",")) }
impl opencode_host::Sol for M {
    fn begin(&self, _c: &[i64], _t: &[u8]) -> Vec<i64> { self.first.clone() }
    fn step(&self, _c: &[i64], _t: &[u8], _ctl: &[i64], b: &opencode_host::Banks, event: &[i64], comp: &[u8]) -> Vec<i64> {
        let mut n = self.n.borrow_mut();
        *n += 1;
        eprintln!("{{\"unit\":{{\"step\":{},\"event\":{},\"completion\":\"{}\",\"banks\":{{\"qw\":{},\"qb\":\"{}\",\"rows\":{},\"text\":\"{}\",\"doc\":\"{}\",\"saved\":{}}}}}}}",
            *n, ws(event), hx(comp), ws(&b.query_words), hx(&b.query_bytes), ws(&b.rows), hx(&b.text), hx(&b.document), ws(&b.saved));
        match self.steps.borrow_mut().pop_front() { Some(p) => p, None => { eprintln!("script exhausted"); std::process::exit(9) } }
    }
}
fn main() {
    let path = std::env::var("HOST_UNIT_SCRIPT").unwrap_or_default();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut first = Vec::new();
    let mut steps = VecDeque::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let tag = it.next().unwrap_or("");
        let v: Vec<i64> = it.map(|x| x.parse().unwrap()).collect();
        if tag == "B" { first = v } else if tag == "S" { steps.push_back(v) }
    }
    opencode_host::run(&M { first, steps: RefCell::new(steps), n: RefCell::new(0) });
}
'''

MAGIC_STEP, MAGIC_RESULT = 0x4f435331, 0x4f435231


def step_packet(action, patches=(), requests=(), words=(), lit=b'', result=(), ctl=None):
    ctl = list(ctl or [0] * 64)
    return ([MAGIC_STEP, 1, action, 64, len(patches), len(requests), len(words), len(lit), len(result), 0, 0, 0] + ctl +
            [x for p in patches for x in p] + [x for r in requests for x in r] + list(words) + list(lit) + list(result))


def request(rid, op, stage=0, flags=0, handle=-1, a=0, arg_start=0, arg_words=0):
    return [rid, op, stage, flags, handle, a, 0, 0, arg_start, arg_words, 0, 0]


def patch(kind, bank, dst, count, source=0, src=0):
    return [kind, bank, dst, count, source, src, 0, 0]


def result_packet(mode, status=0, rows=(), plan=(), text=b'', header=None):
    h = [0] * 40
    h[0], h[1], h[2], h[3] = MAGIC_RESULT, 1, status, mode
    for i in (10, 12, 14, 16, 23, 25, 27):
        h[i] = -1
    for k, v in (header or {}).items():
        h[k] = v
    n = len(rows)
    w = len(rows[0]) if rows else 0
    h[4], h[5], h[6], h[7] = n, w, len(plan) // 3, len(text)
    return h + [x for r in rows for x in r] + list(plan) + list(text)


class Lit:
    """A literal byte pool: add(bytes) -> [start, length] words."""

    def __init__(self):
        self.data = b''

    def add(self, b):
        ref = [len(self.data), len(b)]
        self.data += b
        return ref


def decode_event(ev, comp_hex):
    comp = bytes.fromhex(comp_hex)
    h = ev[:16]
    code = comp[h[8]:h[8] + h[9]].decode() if h[8] >= 0 else None
    return {'id': h[1], 'op': h[2], 'status': h[3], 'class': h[4], 'native': h[5], 'code': code, 'payload': ev[16:16 + h[6]], 'bytes': comp}


def decode_query(d):
    """execute payload [C,R,colRefs...,values...] -> (column names, typed rows)."""
    p, b = d['payload'], d['bytes']
    c, r = p[0], p[1]
    cols = [b[p[2 + 2 * i]:p[2 + 2 * i] + p[3 + 2 * i]].decode() for i in range(c)]
    rows = []
    base = 2 + 2 * c
    for i in range(r):
        row = []
        for j in range(c):
            at = base + 4 * (i * c + j)
            k, a, bb = p[at], p[at + 1], p[at + 2]
            if k == 0:
                row.append({'kind': 'null'})
            elif k == 1:
                row.append({'kind': 'integer', 'decimal': str(a)})
            elif k == 2:
                row.append({'kind': 'real', 'bits': '%08x%08x' % (a, bb)})
            elif k == 3:
                row.append({'kind': 'text', 'text': b[a:a + bb].decode()})
            else:
                row.append({'kind': 'blob', 'hex': b[a:a + bb].hex()})
        rows.append(row)
    return cols, rows


def os_mode(p):
    return os.stat(p).st_mode & 0o777


class Unit:
    """The mock runner and a case runner for the host unit tests."""

    def __init__(self, exe):
        self.exe = exe
        self.fail = []
        self.count = 0

    def run(self, script, cfg=None, root=None, env_extra=None):
        d = Path(root or tempfile.mkdtemp(dir=WORK, prefix='unit-'))
        sp = d / 'script.txt'
        lines = ['B ' + ' '.join(map(str, script[0]))] + ['S ' + ' '.join(map(str, s)) for s in script[1:]]
        sp.write_text('\n'.join(lines) + '\n')
        cfg = dict({'mode': 'dataDir', 'env': {'mode': 'full', 'values': {}}, 'trace': True, 'artifactDir': str(d / 'art')}, **(cfg or {}))
        (d / 'art').mkdir(exist_ok=True)
        env = {k: v for k, v in os.environ.items() if k not in ('XDG_DATA_HOME', 'OPENCODE_DB')}
        env['HOST_UNIT_SCRIPT'] = str(sp)
        env.update(env_extra or {})
        p = subprocess.run([str(self.exe)], input=json.dumps(cfg) + '\n', capture_output=True, text=True, env=env, timeout=120)
        units = [json.loads(l)['unit'] for l in p.stderr.split('\n') if l.startswith('{"unit"')]
        trace = [json.loads(l)['trace'] for l in p.stderr.split('\n') if l.startswith('{"trace"')]
        out = [json.loads(l) for l in p.stdout.split('\n') if l.startswith('{')]
        return {'rc': p.returncode, 'out': out, 'units': units, 'trace': trace, 'stderr': p.stderr}

    def events(self, r):
        return [decode_event(x['event'], x['completion']) for x in r['units']]

    def check(self, name, cond, detail=''):
        self.count += 1
        if cond:
            print('  ok   %s' % name)
        else:
            print('  FAIL %s %s' % (name, str(detail)[:600]))
            self.fail.append(name)


DONE = lambda: step_packet(1, result=result_packet(3, header={14: 0, 15: 0}, text=b''))   # a dataDir result with an empty value


def unit_sqlite(u):
    print('SQLite typed round trip, prepare/all/get, whole-query range failure, read-only open')
    root = Path(tempfile.mkdtemp(dir=WORK, prefix='unit-sql-'))
    tokens = tokens_of(root)
    rows1 = [[{'kind': 'integer', 'decimal': '1'}, {'kind': 'real', 'bits': '3ff0000000000000'}, {'kind': 'text', 'text': 'é'},
              {'kind': 'blob', 'hex': '00ff'}, {'kind': 'null'}],
             [{'kind': 'integer', 'decimal': '9007199254740991'}, {'kind': 'real', 'bits': '3fe0000000000000'}, {'kind': 'text', 'text': ''},
              {'kind': 'blob', 'hex': ''}, {'kind': 'null'}]]
    db = {'path': '$SRC/t.db', 'ddl': ['CREATE TABLE t(a,b,c,d,e)'], 'inserts': [{'table': 't', 'columns': list('abcde'), 'rows': rows1}]}
    path = str(place_db(db, tokens)).encode()
    sql1, sql2 = b'SELECT a,b,c,d,e FROM t ORDER BY rowid', b'SELECT a FROM t WHERE c = ?'
    L = Lit()
    rp, r1, r2, rt = L.add(path), L.add(sql1), L.add(sql2), L.add('é'.encode())
    lit = L.data
    script = [
        step_packet(0, requests=[request(1, 3, 4, arg_words=2)], words=rp, lit=lit),
        step_packet(0, requests=[request(2, 4, 5, flags=0, handle=1, arg_words=2)], words=r1, lit=lit),
        step_packet(0, requests=[request(3, 4, 7, flags=1, handle=2, arg_words=1)], words=[0]),
        step_packet(0, requests=[request(4, 4, 8, flags=0, handle=1, arg_words=2)], words=r2, lit=lit),
        step_packet(0, requests=[request(5, 4, 9, flags=2, handle=3, arg_words=5)], words=[1, 3] + rt + [0], lit=lit),
        step_packet(0, requests=[request(6, 5, 10, handle=1)]),
        step_packet(1, result=result_packet(3, header={14: 0, 15: 2}, text=b'ok')),
    ]
    r = u.run(script, {'mode': 'dataDir'}, root=root)
    u.check('sqlite: exit 0 and a dataDir envelope', r['rc'] == 0 and r['out'] and r['out'][0].get('status') == 'ok', r['stderr'][-400:])
    ev = u.events(r)
    u.check('sqlite: open gives connection 1; prepare gives statement 2 with 5 columns', len(ev) >= 2 and ev[0]['payload'] == [1] and ev[1]['payload'][:2] == [2, 5], ev[:2])
    if len(ev) >= 3:
        cols, rows = decode_query(ev[2])
        u.check('sqlite: columns and typed rows keep storage class and order (REAL 1.0 stays REAL, BLOB raw)', cols == list('abcde') and rows == rows1, (cols, rows))
    if len(ev) >= 5:
        cols, rows = decode_query(ev[4])
        u.check('sqlite: get binds a TEXT parameter and reads at most one row', cols == ['a'] and rows == [[{'kind': 'integer', 'decimal': '1'}]], (cols, rows))
    qtrace = [t for t in r['trace'] if t['op'] == 'db_query' and t['event'] == 'issue']
    u.check('sqlite: trace names prepare/all/prepare/get with the SQL text',
            [t['args']['phase'] for t in qtrace] == ['prepare', 'all', 'prepare', 'get'] and qtrace[0]['args']['sql'] == sql1.decode(), qtrace)
    done_all = [t for t in r['trace'] if t['op'] == 'db_query' and t['event'] == 'complete' and t['result'] and 'rows' in t['result']]
    u.check('sqlite: trace completion carries the typed rows', bool(done_all) and done_all[0]['result']['rows'][0][1] == {'kind': 'real', 'bits': '3ff0000000000000'}, done_all[:1])
    # an unsafe INTEGER fails the whole query: ErrorClass 3 / ERR_OUT_OF_RANGE, no rows
    db2 = dict(db, path='$SRC/u.db', inserts=[{'table': 't', 'columns': list('abcde'), 'rows': [[{'kind': 'integer', 'decimal': '1'}] + [{'kind': 'null'}] * 4,
                                                                                               [{'kind': 'integer', 'decimal': '9007199254740992'}] + [{'kind': 'null'}] * 4]}])
    p2 = str(place_db(db2, tokens)).encode()
    L = Lit()
    rp, r1 = L.add(p2), L.add(sql1)
    script = [
        step_packet(0, requests=[request(1, 3, 4, arg_words=2)], words=rp, lit=L.data),
        step_packet(0, requests=[request(2, 4, 5, flags=0, handle=1, arg_words=2)], words=r1, lit=L.data),
        step_packet(0, requests=[request(3, 4, 7, flags=1, handle=2, arg_words=1)], words=[0]),
        DONE(),
    ]
    ev = u.events(u.run(script, {'mode': 'dataDir'}, root=root))
    u.check('sqlite: unsafe INTEGER fails the whole query as class 3 ERR_OUT_OF_RANGE with no payload',
            len(ev) >= 3 and ev[2]['status'] == 1 and ev[2]['class'] == 3 and ev[2]['code'] == 'ERR_OUT_OF_RANGE' and not ev[2]['payload'], ev[2:3])
    # a missing file opened read-only: SQLite error, extended code 14 (SQLITE_CANTOPEN)
    L = Lit()
    rp = L.add(str(root / 'src' / 'missing.db').encode())
    ev = u.events(u.run([step_packet(0, requests=[request(1, 3, 4, arg_words=2)], words=rp, lit=L.data), DONE()], {'mode': 'dataDir'}, root=root))
    u.check('sqlite: a missing database opens as a SQLite error SQLITE_CANTOPEN (14)',
            bool(ev) and ev[0]['status'] == 1 and ev[0]['class'] == 2 and ev[0]['code'] == 'SQLITE_CANTOPEN' and ev[0]['native'] % 256 == 14, ev)
    v = subprocess.run([str(u.exe), '--sqlite-version'], capture_output=True, text=True, timeout=60)
    u.check('sqlite: --sqlite-version prints the bundled version', v.returncode == 0 and 'sqliteVersion' in v.stdout, v.stdout + v.stderr)
    print('  bundled SQLite: ' + v.stdout.strip())
    clean(root)


def unit_spawn(u):
    print('mkdtemp, wx open, spawn with fd 1 = the open file, observe/kill/close/stat/read, missing program')
    root = Path(tempfile.mkdtemp(dir=WORK, prefix='unit-sp-'))
    tokens = tokens_of(root)
    (root / 'folder').mkdir()
    (root / 'tmp').mkdir()
    data = b'{"messages":[]}' + b'x' * 300000          # > 196608 bytes, no final LF
    case = {'thread': 'T1', 'folder': str(root / 'folder'), 'export': {'bytes': {'hex': data.hex()}, 'exit': 0, 'env': {'OC_SENTINEL': '1', 'XDG_DATA_HOME': None}}}
    exe = root / 'bin' / 'exporter'
    log = place_exporter(case, tokens, exe)
    # mkdtemp: prefix + 6 random characters, mode 0700
    L = Lit()
    pre = L.add(str(root / 'tmp' / 'opencode-').encode())
    r = u.run([step_packet(0, requests=[request(1, 6, 13, arg_words=2)], words=pre, lit=L.data), DONE()], {'mode': 'dataDir'}, root=root)
    ev = u.events(r)
    made = ev[0]['bytes'].decode() if ev and ev[0]['status'] == 0 else ''
    ok = bool(re.fullmatch(re.escape(str(root / 'tmp' / 'opencode-')) + '[A-Za-z0-9]{6}', made)) and Path(made).is_dir() and os_mode(made) == 0o700
    u.check('mkdtemp: prefix + 6 [A-Za-z0-9], directory mode 0700', ok, made)
    try:
        norm = normalize_trace(r['trace'], tokens)
        u.check('mkdtemp: trace normalisation asserts the prefix and maps the random suffix to @1', any('opencode-@1' in json.dumps(e) for e in norm))
    except TraceAssertion as e:
        u.check('mkdtemp: trace normalisation', False, e)
    # open wx, spawn (fd 1 = that file, full env replacement), observe the close, close, stat, read, kill, observe, second wx open
    L = Lit()
    out, binr, cwd, a0, a1, a2, k, v = (L.add(x) for x in (str(root / 'tmp' / 'export.json').encode(), str(exe).encode(), str(root / 'folder').encode(),
                                                          b'--pure', b'export', b'T1', b'OC_SENTINEL', b'1'))
    spawn = binr + cwd + [3, 1, 1, 0, 0, 0, 1, 1] + a0 + a1 + a2 + k + v
    script = [
        step_packet(0, requests=[request(1, 7, 14, flags=1, a=384, arg_words=2)], words=out, lit=L.data),
        step_packet(0, requests=[request(2, 8, 15, flags=0, handle=1, arg_words=len(spawn))], words=spawn, lit=L.data),
        step_packet(0, requests=[request(3, 9, 16, handle=2, a=600000)]),
        step_packet(0, requests=[request(4, 11, 20, handle=1)]),
        step_packet(0, requests=[request(5, 12, 21, arg_words=2)], words=out, lit=L.data),
        step_packet(0, requests=[request(6, 13, 22, arg_words=2)], words=out, lit=L.data),
        step_packet(0, requests=[request(7, 10, 17, flags=1, handle=2, a=15)]),
        step_packet(0, requests=[request(8, 9, 16, handle=2, a=1)]),
        step_packet(0, requests=[request(9, 7, 14, flags=1, a=384, arg_words=2)], words=out, lit=L.data),
        DONE(),
    ]
    r = u.run(script, {'mode': 'dataDir'}, root=root, env_extra={'XDG_DATA_HOME': '/nonexistent-xdg'})
    ev = u.events(r)
    u.check('spawn: the whole sequence runs', r['rc'] == 0 and len(ev) == 9, r['stderr'][-500:])
    if len(ev) == 9:
        u.check('open wx gives file handle 1', ev[0]['status'] == 0 and ev[0]['payload'] == [1], ev[0])
        u.check('spawn gives child handle 2', ev[1]['status'] == 0 and ev[1]['payload'] == [2], ev[1])
        u.check('observe: the first event is the child close with exit code 0 (not the deadline)', ev[2]['payload'][:4] == [1, 1, 0, 0], ev[2]['payload'])
        u.check('close succeeds; stat sees a regular file of the full size', ev[3]['status'] == 0 and ev[4]['payload'] == [1, len(data)], (ev[3], ev[4]['payload']))
        u.check('read returns all bytes to EOF (>196608, no final LF)', ev[5]['status'] == 0 and ev[5]['bytes'] == data)
        u.check('kill of an ended child reports sent=0', ev[6]['payload'] == [0], ev[6]['payload'])
        u.check('observe with an elapsed deadline after close returns the deadline event', ev[7]['payload'][:4] == [0, 0, 0, 0], ev[7]['payload'])
        u.check('a second wx open of the same path fails EEXIST (OS class 1, errno 17)', ev[8]['status'] == 1 and ev[8]['class'] == 1 and ev[8]['code'] == 'EEXIST' and ev[8]['native'] == 17, ev[8])
    f = root / 'tmp' / 'export.json'
    u.check('the export file has mode 0600 and exactly the exporter bytes', f.exists() and os_mode(f) == 0o600 and f.read_bytes() == data)
    lg = read_exporter_log(log)
    u.check('fake exporter: argv, cwd, sentinel present/absent, fd 1 regular, stdin/stderr null, group leader: no violations',
            lg is not None and not lg['violations'] and bool(lg['fd1']) and lg['fd1']['regular'], json.dumps(lg)[:500])
    # a missing program: spawn succeeds, then observe returns an error event (ENOENT; status 0, class OS)
    L = Lit()
    out2, binm, cwd, a0, a1, a2 = (L.add(x) for x in (str(root / 'tmp' / 'out2.json').encode(), str(root / 'bin' / 'missing').encode(), str(root / 'folder').encode(),
                                                      b'--pure', b'export', b'T1'))
    spawn = binm + cwd + [3, 1, 0, 0, 0, 0, 1, 1] + a0 + a1 + a2
    script = [
        step_packet(0, requests=[request(1, 7, 14, flags=1, a=384, arg_words=2)], words=out2, lit=L.data),
        step_packet(0, requests=[request(2, 8, 15, flags=0, handle=1, arg_words=len(spawn))], words=spawn, lit=L.data),
        step_packet(0, requests=[request(3, 9, 16, handle=2, a=600000)]),
        DONE(),
    ]
    ev = u.events(u.run(script, {'mode': 'dataDir'}, root=root))
    u.check('spawn of a missing program: success, then an error event ENOENT (status 0, class 1, errno 2)',
            len(ev) == 3 and ev[1]['status'] == 0 and ev[2]['status'] == 0 and ev[2]['payload'][0] == 2 and ev[2]['class'] == 1 and ev[2]['code'] == 'ENOENT' and ev[2]['native'] == 2, ev)
    clean(root)


def unit_patches(u):
    print('delta patches (resize keeps the prefix and zero-fills; copies from event words / completion bytes / literal words / literal bytes) and rejections')
    root = Path(tempfile.mkdtemp(dir=WORK, prefix='unit-pt-'))
    real = os.path.realpath(root).encode()
    L = Lit()
    rr = L.add(real)
    P = patch
    script = [
        # begin packet: bank 3 (text) = 'hello' from literal bytes; bank 0 (query words) = [7,8,9] from literal words
        step_packet(2, patches=[P(0, 3, 0, 5), P(1, 3, 0, 5, 4, 0), P(0, 0, 0, 3), P(1, 0, 0, 3, 3, 0)], words=[7, 8, 9], lit=b'hello'),
        step_packet(2, patches=[P(0, 3, 0, 8)]),                                             # grow keeps 'hello', zero-fills 3 bytes
        step_packet(0, requests=[request(1, 15, 12, flags=1)], patches=[P(1, 1, 0, 0, 4, 0)]),   # wall clock; a legal empty copy
        step_packet(0, requests=[request(2, 1, 1, arg_words=2)], words=rr, lit=L.data,           # realpath; rows[0] = clock payload (event word 16)
                    patches=[P(0, 2, 0, 1), P(1, 2, 0, 1, 1, 16)]),
        step_packet(2, patches=[P(0, 4, 0, len(real)), P(1, 4, 0, len(real), 2, 0),               # document = the completion bytes
                                P(0, 5, 0, 2), P(1, 5, 0, 2, 1, 16)]),                           # saved = event words 16,17 (the path ref)
        step_packet(1, result=result_packet(3, header={14: 0, 15: 1}, text=b'v')),
    ]
    r = u.run(script, {'mode': 'dataDir', 'nowMs': 1700000000000}, root=root)
    u.check('patches: the run completes with the rendered dataDir', r['rc'] == 0 and r['out'] and r['out'][0].get('value') == 'v', r['stderr'][-400:])
    un = r['units']
    if len(un) == 5:
        u.check('patches: resize + literal-bytes copy fills the text bank', un[0]['banks']['text'] == b'hello'.hex(), un[0]['banks'])
        u.check('patches: literal-words copy fills the words bank', un[0]['banks']['qw'] == [7, 8, 9], un[0]['banks'])
        u.check('patches: resize keeps the prefix and zero-fills new space', un[1]['banks']['text'] == (b'hello' + b'\0\0\0').hex(), un[1]['banks'])
        u.check('patches: rows bank copies the wall-clock payload (frozen nowMs) from the event words', un[3]['banks']['rows'] == [1700000000000], un[3]['banks'])
        u.check('patches: the document bank copies the completion bytes of the previous request', un[4]['banks']['doc'] == real.hex(), un[4]['banks'])
        u.check('patches: the saved bank copies event words at an offset', un[4]['banks']['saved'] == [0, len(real)], un[4]['banks'])
    else:
        u.check('patches: five step calls', False, len(un))
    done = lambda: step_packet(1, result=result_packet(3, text=b''))
    bad = {
        'copy beyond the destination bank': [step_packet(2, patches=[P(0, 3, 0, 2), P(1, 3, 0, 3, 4, 0)], lit=b'abc'), done()],
        'copy beyond the source': [step_packet(2, patches=[P(0, 3, 0, 4), P(1, 3, 0, 4, 4, 0)], lit=b'abc'), done()],
        'resize beyond the 2^31 bank cap': [step_packet(2, patches=[P(0, 3, 0, (1 << 31) + 1)]), done()],
        'copy past the 2^31 bank cap': [step_packet(2, patches=[P(0, 3, 0, 1), P(1, 3, (1 << 31), 1, 4, 0)], lit=b'a'), done()],
        'word 256 copied into a byte bank': [step_packet(2, patches=[P(0, 3, 0, 1), P(1, 3, 0, 1, 3, 0)], words=[256]), done()],
        'unknown bank 6': [step_packet(2, patches=[P(0, 6, 0, 1)]), done()],
        'resize with a source': [step_packet(2, patches=[P(0, 3, 0, 1, 4, 0)], lit=b'a'), done()],
        'a request id that does not increase': [step_packet(0, requests=[request(1, 15, 12, flags=1)]), step_packet(0, requests=[request(1, 15, 12, flags=1)]), done()],
        'a done step that also issues a request': [step_packet(1, requests=[request(1, 15, 12, flags=1)], result=result_packet(3, text=b''))],
        'an await step with no request': [step_packet(0)],
        'opcode 16': [step_packet(0, requests=[request(1, 16)]), done()],
        'clock_now with a stray handle': [step_packet(0, requests=[request(1, 15, 12, flags=1, handle=3)]), done()],
        'spawn argc/word count mismatch': [step_packet(0, requests=[request(1, 8, 15, flags=0, handle=1, arg_words=13)], words=[0] * 13), done()],
        'a result ref outside the text': [step_packet(1, result=result_packet(3, header={14: 0, 15: 5}, text=b'ab'))],
        'result text that is not UTF-8 at scalar boundaries': [step_packet(1, result=result_packet(3, header={14: 0, 15: 1}, text=b'\xc3'))],
        'a present plan with zero items': [step_packet(1, result=result_packet(5, header={9: 1}, text=b''))],
        'an error result carrying a title': [step_packet(1, result=result_packet(0, status=1, header={20: 1, 10: 0, 11: 0}, text=b''))],
    }
    for name, script in bad.items():
        r = u.run(script, {'mode': 'dataDir'}, root=root)
        u.check('reject (exit 3, host-protocol-error): ' + name, r['rc'] == 3 and r['out'] and r['out'][0].get('status') == 'host-protocol-error', 'rc=%s out=%s' % (r['rc'], r['out']))
    clean(root)


def unit_render(u):
    print('result packet rendering (DESIGN section 5)')
    root = Path(tempfile.mkdtemp(dir=WORK, prefix='unit-rd-'))

    def one(pkt, mode):
        r = u.run([step_packet(1, result=pkt)], {'mode': mode}, root=root)
        return r['out'][0] if r['out'] else {'rc': r['rc'], 'stderr': r['stderr'][-200:]}

    got = one(result_packet(0, rows=[[0, 1, 1, 5, 6, 24, 1, 3, 1, 0]], text=b'Atitle2023-11-14T22:13:20.000Z'), 'list')
    u.check('list row: thread, title, updatedAt, messages, maybeOpen', got == {'status': 'ok', 'mode': 'list', 'rows': [
        {'thread': 'A', 'title': 'title', 'updatedAt': '2023-11-14T22:13:20.000Z', 'messages': 3, 'maybeOpen': True}]}, got)
    got = one(result_packet(0, rows=[[0, 0, 0, 0, -1, 0, 0, 0, 0, 0]], text=b''), 'list')
    u.check('(0,0) is a present empty string, (-1,0) is absent; zero messages stay absent', got.get('rows') == [{'thread': '', 'title': ''}], got)
    got = one(result_packet(0, text=b''), 'list')
    u.check('an empty list', got == {'status': 'ok', 'mode': 'list', 'rows': []}, got)
    got = one(result_packet(1, rows=[[0, 1, 1, 1, -1, 0, 2, 3, 2, 1]], header={8: 1, 10: 5, 11: 1, 12: 6, 13: 1}, text=b'PRabcTM'), 'read')
    u.check('read: one turn, omittedBefore, title, model', got == {'status': 'ok', 'mode': 'read', 'turns': [
        {'prompt': 'P', 'reply': 'R', 'completedAt': 'abc', 'tools': 2, 'clipped': True}], 'omittedBefore': True, 'title': 'T', 'model': 'M'}, got)
    got = one(result_packet(2, rows=[[0, 1, 1, 1, 2, 0, 1700000000000]], text=b'AB'), 'rows')
    u.check('rows: id, directory, empty title, updatedMs', got == {'status': 'ok', 'mode': 'rows', 'rows': [
        {'id': 'A', 'directory': 'B', 'title': '', 'updatedMs': 1700000000000}]}, got)
    got = one(result_packet(4, header={14: 0, 15: 0}, text=b''), 'folderKey')
    u.check('folderKey with an empty value', got == {'status': 'ok', 'mode': 'folderKey', 'value': ''}, got)
    got = one(result_packet(5, header={9: 1, 16: 0, 17: 2, 18: 4}, plan=[2, 3, 2, 0, 0, 0], text=b'zzabc'), 'plan')
    u.check('plan: at, an in_progress item, an item without status, more', got == {'status': 'ok', 'mode': 'plan', 'present': True, 'value': {
        'at': 'zz', 'items': [{'text': 'abc', 'status': 'in_progress'}, {'text': ''}], 'more': 4}}, got)
    got = one(result_packet(5), 'plan')
    u.check('an absent plan', got == {'status': 'ok', 'mode': 'plan', 'present': False}, got)
    got = one(result_packet(0, status=1, header={19: 4, 20: 1, 21: 2, 22: 14, 23: 0, 24: 15, 27: 15, 28: 7}, text=b'SQLITE_CANTOPENcolumns'), 'list')
    u.check('an error envelope', got == {'status': 'error', 'mode': 'list', 'error': {
        'class': 'EngineImportError', 'code': 'read-failed', 'name': None, 'detail': 'columns', 'stage': 'db-open', 'rawCode': 'SQLITE_CANTOPEN', 'native': 14}}, got)
    clean(root)


def run_host_unit():
    WORK.mkdir(parents=True, exist_ok=True)
    rustc = tool('rustc')
    log = WORK / 'logs' / 'unit_host_build.log'
    if log.exists():
        log.unlink()
    host = build_host(rustc, log)
    d = WORK / 'unit_host'
    d.mkdir(parents=True, exist_ok=True)
    (d / 'main.rs').write_text(MOCK_RS)
    exe = d / 'runner'
    rustc_link(rustc, d / 'main.rs', exe, host)
    u = Unit(exe)
    for fn in (unit_sqlite, unit_spawn, unit_patches, unit_render):
        fn(u)
    print('host unit: %d checks, %d failed' % (u.count, len(u.fail)))
    return 1 if u.fail else 0


# ---- main -------------------------------------------------------------------------------------------------------------
def parse_pairs(text):
    if not text:
        return PAIRS
    out = []
    for p in text.split(','):
        c, b = p.strip().split('/')
        if (c, b) not in PAIRS:
            raise SystemExit('unknown pair ' + p)
        out.append((c, b))
    return out


def run_batch(cases, fn, jobs):
    """fn(case) for every case: parallel for ordinary cases, then serially for timer-sensitive ones. Order is kept."""
    results = {}
    par = [c for c in cases if not timer_sensitive(c)]
    ser = [c for c in cases if timer_sensitive(c)]
    with ThreadPoolExecutor(jobs) as ex:
        for c, r in zip(par, ex.map(fn, par)):
            results[c['name']] = r
    for c in ser:
        results[c['name']] = fn(c)
    return [(c, results[c['name']]) for c in cases]


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--pairs', help='comma list of py/rust,py/llvm,rs/rust,rs/llvm (default all four)')
    ap.add_argument('--only', action='append', default=[], help='fixtures whose name contains SUBSTRING (repeatable)')
    ap.add_argument('--verify-only', action='store_true', help='true verification of root and what it reaches on the selected compilers; no linking or execution')
    ap.add_argument('--scope', choices=sorted(SCOPES), help="with --verify-only: only that package's modules via a temporary entry")
    ap.add_argument('--unit', choices=['packet', 'host', 'export', 'convert'], help='unit group (host = host unit tests)')
    ap.add_argument('--reference-only', action='store_true', help='run only the pinned TypeScript and check `expected`')
    ap.add_argument('--slow', action='store_true', help='include slow fixtures (the real 20 s deadline)')
    ap.add_argument('--jobs', type=int, default=4)
    ap.add_argument('--keep', action='store_true', help='keep the work trees of passing cases')
    ap.add_argument('--list', action='store_true', help='list the selected fixtures and exit')
    ap.add_argument('--reuse-verified', action='store_true', help='reuse work/<pair>/root_gen.* of an earlier verified run')
    args = ap.parse_args(argv)
    if args.scope and not args.verify_only:
        ap.error('--scope needs --verify-only')
    if args.unit == 'host':
        return run_host_unit()
    only = list(args.only)
    if args.unit:
        only = only or UNIT_SELECT[args.unit]
    pairs = parse_pairs(args.pairs) if args.pairs else ([('rs', 'rust')] if args.unit else PAIRS)
    if args.verify_only:
        return verify_only(pairs, args.scope)
    cases, skipped = load_fixtures(only, args.slow)
    if args.list:
        for c in cases:
            print('%-16s %-44s kills=%s%s' % (c['category'], c['name'], ','.join(c.get('kills', [])), ' slow' if c.get('slow') else ''))
        for name, why in skipped:
            print('skipped %-44s %s' % (name, why))
        return 0
    if not cases:
        print('no fixtures selected (fixtures/F*.json missing, or --only matches nothing)', file=sys.stderr)
        return 2
    if not os.environ.get('GROK_TS_TRANSPILER'):
        print('GROK_TS_TRANSPILER must name the typescript package: the reference transpiles plan.ts with it', file=sys.stderr)
        return 2
    failures, report = [], {'fixtures': len(cases), 'skipped': skipped, 'pairs': {}, 'ts': {}}
    t0 = time.monotonic()
    refs = dict((c['name'], r) for c, r in run_batch(cases, run_reference, args.jobs))
    meta = next((r[0]['meta'] for r in refs.values() if r[0].get('meta')), None)
    print('reference: Node %s, V8 %s, SQLite %s, %d fixtures, %.1f s' % ((meta or {}).get('node'), (meta or {}).get('v8'), (meta or {}).get('sqlite'), len(cases), time.monotonic() - t0), flush=True)
    report['ts'] = {'meta': meta}
    bad = 0
    for case in cases:
        problems = check_expected(case, refs[case['name']][0])
        if problems:
            bad += 1
            failures.append(('ts', 'expected', case['name'], '; '.join(problems)))
            print('ts/expected: FAIL %s: %s' % (case['name'], problems[0]), flush=True)
    print('ts/expected: %d/%d fixtures do what they declare' % (len(cases) - bad, len(cases)), flush=True)
    if args.reference_only:
        (WORK / 'test_impl_report.json').write_text(json.dumps(dict(report, failures=failures), indent=1, default=str))
        return 1 if failures else 0
    _, g, _ = configure()
    host = build_host(g.RUSTC)
    exes = {}
    for compiler, backend in pairs:
        exes[(compiler, backend)] = build_pair(g, compiler, backend, host, args.reuse_verified)
    seen_policies = {}
    for (compiler, backend), exe in exes.items():
        key = f'{compiler}/{backend}'
        side = f'{compiler}_{backend}'

        def one(case, exe=exe, side=side):
            impl = run_pair(case, side, exe)
            ts, base = refs[case['name']]
            return impl, judge(case, ts, base, impl)

        passed, t0 = 0, time.monotonic()
        for case, (impl, v) in run_batch(cases, one, args.jobs):
            if v['ok']:
                passed += 1
                for p in v['policies']:
                    seen_policies.setdefault(p, set()).add(case['name'])
                if not args.keep:
                    clean(impl['root'])
                note = ' [%s]' % ','.join(v['policies']) if v['policies'] else ''
                print(f'{key}: PASS {case["name"]}{note}', flush=True)
            else:
                failures.append((compiler, backend, case['name'], '; '.join(v['problems'])))
                print(f'{key}: FAIL {case["name"]}: {v["problems"][0]}', flush=True)
        report['pairs'][key] = {'passed': passed, 'of': len(cases), 'seconds': round(time.monotonic() - t0, 1)}
        print(f'{key}: {passed}/{len(cases)} passed', flush=True)
    report['policies_observed'] = {p: sorted(v) for p, v in sorted(seen_policies.items())}
    report['failures'] = failures
    if not args.keep:
        for case in cases:
            ts_run = refs[case['name']][0]
            if ts_run.get('root') and not any(f[2] == case['name'] for f in failures):
                clean(ts_run['root'])
    (WORK / 'test_impl_report.json').write_text(json.dumps(report, indent=1, default=str))
    print('policies observed (a difference explained by a policy): ' + (', '.join('%s x%d' % (p, len(v)) for p, v in sorted(seen_policies.items())) or 'none'))
    print('FAILED: %d' % len(failures) if failures else 'all fixtures passed on %d pair(s)' % len(exes))
    return 1 if failures else 0


if __name__ == '__main__':
    raise SystemExit(main())
