"""Shared helpers of the Grok import tests (test_impl.py, q.py, oracle.py): fixture trees, fake ACP placement,
running a runner or the TypeScript reference on one JSONL case, tree snapshots, trace normalisation (DESIGN §5/§6).

Fixture (DESIGN §6): {name, mode, umask, source:{engine,home}, folder, thread, targetHome, tree:[...], blobs:{name:RECIPE},
  schedule:{readdir,entropyHex,rules}, acp:{<method>:ACTION}, acp_options:{lifetime,ignore_term,allow_no_reply},
  smallPath, smallHomeReal, cleanupPath, rpcMs, binary, watchdog, concurrency, policies, expected}.
Path tokens in tree paths, folder, thread, targetHome, smallPath, smallHomeReal, cleanupPath: $SRC $DST $ROOT $ACP
and $SRC_REAL $DST_REAL (realpath). The ACP cases need a folder that exists (the child runs with cwd=folder), e.g. "/".
Byte recipes: {"utf8":S} {"hex":H} {"repeat":RECIPE,"count":N} {"concat":[RECIPE,...]} {"blob":NAME}.
"""
import sys
sys.dont_write_bytecode = True
import hashlib
import json
import os
import re
import shutil
import signal
import stat
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
WORK = HERE / 'work'
NODE = os.environ.get('GROK_NODE', '/opt/homebrew/bin/node')
sys.path.insert(0, str(HERE))
from q_server import Num, expand, load_raw  # noqa: E402  (macro expansion and raw-token numbers shared with the fake)

STAGES = ['input', 'realHome', 'chain', 'readdir', 'candidate-dir', 'file-lstat', 'containment', 'open',
          'identity-fstat', 'size-fstat', 'read', 'close', 'parse', 'summary', 'sort', 'active',
          'first-target-lstat', 'entropy', 'parent-mkdir', 'temp-mkdir', 'exclusive-create', 'write', 'second-target-lstat', 'rename',
          'spawn', 'initialize', 'session/load', 'temp-cleanup', 'target-cleanup', 'caller-cleanup', 'result']
CLASSES = ['none', 'EngineImportError', 'AbortError', 'RpcResponseError', 'RawOsError', 'URIError', 'CoreError', 'OtherError']
ERRNO_CLASSES = ['none', 'ENOENT', 'EACCES', 'EEXIST', 'other']
CODES = ['none', 'InputTooLarge', 'InternalInvariant', 'not-found', 'grok-exists', 'grok-load-failed', 'timeout', 'read-failed', 'engine-missing']


# ---- fixture recipes ---------------------------------------------------------------------------------------------
def bytes_of(recipe, blobs=None):
    """Raw bytes of a recipe; JSON text such as 1e400, -0 or invalid UTF-8 is never round-tripped through Python values."""
    if isinstance(recipe, str):
        return recipe.encode('utf-8', 'surrogatepass')
    if 'utf8' in recipe:
        return recipe['utf8'].encode('utf-8', 'surrogatepass')
    if 'hex' in recipe:
        return bytes.fromhex(recipe['hex'])
    if 'repeat' in recipe:
        return bytes_of(recipe['repeat'], blobs) * recipe['count']
    if 'concat' in recipe:
        return b''.join(bytes_of(r, blobs) for r in recipe['concat'])
    if 'blob' in recipe:
        return bytes_of((blobs or {})[recipe['blob']], blobs)
    raise ValueError('bad bytes recipe %r' % (recipe,))


def tokens_of(case, root):
    root = Path(root)
    acp = root / 'acp'
    return {'$SRC_REAL': str((root / 'src').resolve()), '$DST_REAL': str((root / 'dst').resolve()),
            '$SRC': str(root / 'src'), '$DST': str(root / 'dst'), '$ROOT': str(root), '$ACP': str(acp)}


def expand_tokens(value, tokens):
    if isinstance(value, str):
        return re.sub(r'\$(SRC_REAL|DST_REAL|SRC|DST|ROOT|ACP)', lambda m: tokens['$' + m.group(1)], value)
    return value


def build_tree(entries, tokens, blobs):
    """Create dir/file/symlink/hardlink/fifo entries; modes are applied last, deepest first (nothing is locked early)."""
    modes = []
    for e in entries:
        path = Path(expand_tokens(e['path'], tokens))
        kind = e['kind']
        path.parent.mkdir(parents=True, exist_ok=True)
        if kind == 'dir':
            path.mkdir(exist_ok=True)
        elif kind == 'file':
            path.write_bytes(bytes_of(e.get('bytes', {'utf8': ''}), blobs))
        elif kind == 'symlink':
            os.symlink(expand_tokens(e['target'], tokens), path)
        elif kind == 'hardlink':
            os.link(expand_tokens(e['of'], tokens), path)
        elif kind == 'fifo':
            os.mkfifo(path)
        else:
            raise ValueError('bad tree kind ' + kind)
        if 'mode' in e and kind != 'symlink':
            modes.append((path, e['mode']))
    for path, mode in sorted(modes, key=lambda m: -len(m[0].parts)):
        os.chmod(path, mode)


def snapshot(root, roots=None):
    """{relative path: {kind, mode, size, sha256 | target}} of everything below root (root itself is '.').
    Absolute symlink targets inside the case root become logical ($SRC/$DST) so the two sides' trees compare."""
    root = Path(root)
    out = {}
    if not os.path.lexists(root):
        return out
    if os.path.islink(root):                            # a symlinked home: snapshot what it points at (links inside stay links)
        target = Path(os.path.realpath(root))
        if os.path.lexists(target):
            root = target

    def visit(path, rel):
        st = os.lstat(path)
        mode = stat.S_IMODE(st.st_mode)
        if stat.S_ISLNK(st.st_mode):
            out[rel] = {'kind': 'symlink', 'target': logical_path(os.readlink(path), roots or {})}
        elif stat.S_ISDIR(st.st_mode):
            out[rel] = {'kind': 'dir', 'mode': mode}
            try:
                names = sorted(os.listdir(path))
            except PermissionError:
                out[rel]['unreadable'] = True
                return
            for n in names:
                visit(path / n, n if rel == '.' else rel + '/' + n)
        elif stat.S_ISREG(st.st_mode):
            try:
                data = Path(path).read_bytes()
                out[rel] = {'kind': 'file', 'mode': mode, 'size': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
            except PermissionError:
                out[rel] = {'kind': 'file', 'mode': mode, 'unreadable': True}
        else:
            out[rel] = {'kind': 'fifo' if stat.S_ISFIFO(st.st_mode) else 'other', 'mode': mode}
    visit(root, '.')
    return out


def tree_files(case):
    """(logical path, bytes) of every file recipe in the case (used by the oracle's trigger detectors)."""
    blobs = case.get('blobs', {})
    return [(e['path'], bytes_of(e.get('bytes', {'utf8': ''}), blobs)) for e in case.get('tree', []) if e['kind'] == 'file']


def clean(path):
    """Remove a work directory even when the importer left unreadable modes behind."""
    path = Path(path)
    if not os.path.lexists(path):
        return
    for dirpath, dirnames, _ in os.walk(path):
        for d in dirnames:
            p = os.path.join(dirpath, d)
            if not os.path.islink(p):
                os.chmod(p, 0o700)
    shutil.rmtree(path, ignore_errors=True)


# ---- fake ACP ------------------------------------------------------------------------------------------------------
def place_acp(acp_dir, case, tokens, mark):
    """Executable `grok` (+ sibling scenario and log paths). Returns (exe, log path). The wrapper adds nothing to argv."""
    acp_dir = Path(acp_dir)
    acp_dir.mkdir(parents=True, exist_ok=True)
    exe = acp_dir / 'grok'
    exe.write_text('#!/bin/sh\nQ_ACP_EXE="$0" exec \'%s\' -B \'%s\' "$@"\n' % (sys.executable, HERE / 'q_server.py'))
    exe.chmod(exe.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    folder = expand_tokens(case.get('folder', '/'), tokens)
    scenario = {
        'methods': case.get('acp', {}),
        'expect': {'cwd': folder, 'grokHome': expand_tokens(case.get('targetHome', ''), tokens), 'thread': expand_tokens(case.get('thread', ''), tokens),
                   'folder': folder, 'env': {'LAIR_Q_MARK': mark}},
    }
    scenario.update(case.get('acp_options', {}))
    scenario = json.loads(json.dumps(scenario))
    (acp_dir / 'grok.scenario.json').write_text(json.dumps(scenario), encoding='utf-8')
    log = acp_dir / 'grok.log.json'
    if log.exists():
        log.unlink()
    return str(exe), log


def read_acp_log(path):
    p = Path(path)
    if not p.exists():
        return None
    try:
        return json.loads(p.read_text(encoding='utf-8'))
    except ValueError:
        return {'unreadable': True, 'violations': ['transcript is not valid JSON']}


# ---- running a runner ----------------------------------------------------------------------------------------------
def reference_command():
    return [NODE, '--experimental-strip-types', '--import', HERE / 'reference-hooks.mjs', HERE / 'reference.mjs']


def make_config(case, tokens, exe, trace=True):
    """The JSONL config both the Rust host and the TS reference read (DESIGN §1 / task F2)."""
    t = lambda k: expand_tokens(case[k], tokens) if k in case else None
    cfg = {
        'mode': case['mode'], 'sourceHome': expand_tokens(case.get('source', {}).get('home', '$SRC'), tokens), 'folder': t('folder'),
        'thread': t('thread'), 'targetHome': t('targetHome'), 'binary': case.get('binary') or exe,
        'sourceEngine': case.get('source', {}).get('engine', 'grok'), 'smallPath': t('smallPath'), 'smallHomeReal': t('smallHomeReal'),
        'cleanupPath': t('cleanupPath'), 'rpcMs': case.get('rpcMs'), 'schedule': dict(case.get('schedule', {}), blobs=case.get('blobs', {})),
        'trace': trace,
    }
    return {k: v for k, v in cfg.items() if v is not None}


def _kill_group(pid):
    try:
        os.killpg(pid, signal.SIGKILL)
    except (ProcessLookupError, PermissionError):
        pass


def run_side(case, side, command, *, trace=True, watchdog=None, root=None):
    """Build the trees and the fake ACP of `case` under WORK/cases/<side>/<name>, run `command` on its config,
    snapshot before/after. Never raises on a runner failure: see the keys `returncode`, `watchdog`.
    `side` names the runner ('ts', 'py_rust', ...). Returns a dict:
      result, meta, trace, stderr, acp, before, after, roots, returncode, watchdog, stdout_lines."""
    root = Path(root) if root else WORK / 'cases' / side / re.sub(r'[^A-Za-z0-9_.-]', '_', case['name'])
    clean(root)
    root.mkdir(parents=True)
    root = root.resolve()
    tokens = tokens_of(case, root)
    build_tree(case.get('tree', []), tokens, case.get('blobs', {}))
    mark = hashlib.sha256((side + case['name']).encode()).hexdigest()[:16]
    exe, log = place_acp(root / 'acp', case, tokens, mark)
    cfg = make_config(case, tokens, exe, trace)
    sroots = {'src': str(root / 'src'), 'dst': str(root / 'dst'), 'src_real': str((root / 'src').resolve()), 'dst_real': str((root / 'dst').resolve())}
    before = {'src': snapshot(root / 'src', sroots), 'dst': snapshot(root / 'dst', sroots)}
    umask = case.get('umask', 0o022)
    cmd = ['/bin/sh', '-c', 'umask "$0"; exec "$@"', '%o' % umask] + [str(x) for x in command]
    env = dict(os.environ, LAIR_Q_MARK=mark)
    limit = watchdog or case.get('watchdog') or 120
    proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, cwd=str(HERE), start_new_session=True)
    killed = False
    try:
        out, err = proc.communicate(json.dumps(cfg).encode() + b'\n', timeout=limit)
    except subprocess.TimeoutExpired:
        killed = True                                   # a watchdog kill is never a success
        _kill_group(proc.pid)
        acp = read_acp_log(log) or {}
        for pid in [acp.get('pid')] + [int(m.split()[-1]) for m in acp.get('notes', []) if m.startswith('descendant pid')]:
            if pid:
                _kill_group(pid)
                try:
                    os.kill(pid, signal.SIGKILL)
                except (ProcessLookupError, PermissionError):
                    pass
        try:
            out, err = proc.communicate(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
            out, err = b'', b''
    after = {'src': snapshot(root / 'src', sroots), 'dst': snapshot(root / 'dst', sroots)}
    lines = [l for l in out.decode('utf-8', 'replace').split('\n') if l]   # split on LF only: U+2028 stays in a string
    rows = [load_raw(l) for l in lines]
    meta = next((r['meta'] for r in rows if isinstance(r, dict) and 'meta' in r), None)
    results = [r for r in rows if not (isinstance(r, dict) and 'meta' in r)]
    trace_lines, other = [], []
    for l in err.decode('utf-8', 'replace').split('\n'):
        if l.startswith('{"trace"'):
            trace_lines.append(json.loads(l)['trace'])
        elif l:
            other.append(l)
    return {
        'result': results[0] if results else None, 'meta': meta, 'trace': trace_lines, 'stderr': '\n'.join(other[-40:]),
        'acp': read_acp_log(log), 'before': before, 'after': after, 'roots': {'src': str(root / 'src'), 'dst': str(root / 'dst'), 'src_real': str((root / 'src').resolve()), 'dst_real': str((root / 'dst').resolve())},
        'returncode': proc.returncode, 'watchdog': killed, 'stdout_lines': lines, 'root': str(root),
    }   # the trees stay for diagnosis; the caller removes them with clean(root) once the case passes


# ---- trace normalisation (DESIGN §5 "Normalized") ------------------------------------------------------------------
RW = ('read', 'write')


def logical_path(path, roots):
    """Real fixture root -> $SRC/$DST (a side that already prints logical paths is unchanged)."""
    if not isinstance(path, str):
        return path
    for key, name in (('src', '$SRC'), ('dst', '$DST')):
        for r in {roots.get(key), roots.get(key + '_real')}:
            if r and (path == r or path.startswith(r + '/')):
                return name + path[len(r):]
    return path


def pair_trace(events, roots):
    """Issue/complete lines -> one record per primitive, in issue order; barriers become separators."""
    recs, by_id = [], {}
    for e in events:
        ev = e.get('event')
        if ev == 'issue':
            r = {k: e.get(k) for k in ('op', 'path', 'handle', 'offset', 'count', 'mode')}
            r.update(errno=None, kind=None, dev=None, ino=None, bytes=None, digest=None, id=e.get('id'))
            r['path'] = logical_path(r['path'], roots)
            recs.append(r)
            by_id[e.get('id')] = r
        elif ev == 'complete':
            r = by_id.get(e.get('id'))
            if r is not None:
                for k in ('errno', 'kind', 'dev', 'ino', 'bytes', 'digest'):
                    r[k] = e.get(k)
                if e.get('handle') is not None:
                    r['handle'] = e['handle']
                if r['op'] in ('open', 'create'):
                    r['mode'] = r['mode'] if r['mode'] is not None else e.get('mode')
        elif isinstance(ev, str) and ev.startswith('barrier'):
            recs.append({'op': 'barrier', 'event': ev, 'path': logical_path(e.get('path'), roots), 'target': e.get('op'), 'errno': None})
    return recs


def _name_handles(recs):
    """Handle ids -> '<opened path>#<k>' (independent of timing); fills the path of handle operations."""
    names, count = {}, {}
    for r in recs:
        if r['op'] in ('open', 'create') and r.get('handle') is not None:
            k = count[(r['op'], r['path'])] = count.get((r['op'], r['path']), 0) + 1
            names[r['handle']] = '%s#%d' % (r['path'], k)
            r['handle'] = names[r['handle']]
            r['_hpath'] = r['path']
        elif r['op'] in ('fstat', 'read', 'write', 'close') and r.get('handle') in names:
            r['handle'] = names[r['handle']]
            r['path'] = r['handle'].rsplit('#', 1)[0]
        elif r['op'] in RW + ('fstat', 'close'):
            r['handle'] = None if r.get('handle') is None else 'unknown'


def _candidate(path, mode_list):
    m = re.match(r'^\$SRC/sessions/[^/]+/([^/]+)(?:/|$)', path or '')
    return m.group(1) if (m and mode_list) else None


def _merge(run):
    """A run of consecutive successful read/write records -> one record per (op, handle) with contiguous ranges."""
    order, groups = [], {}
    for r in run:
        if not r['bytes']:
            continue                                   # EOF probes carry no data
        key = (r['op'], r['handle'])
        if key not in groups:
            groups[key] = {'op': r['op'], 'path': r['path'], 'handle': r['handle'], 'ranges': [], 'bytes': 0, 'digest': None, 'n': 0}
            order.append(key)
        g = groups[key]
        start = r['offset']
        if g['ranges'] and g['ranges'][-1][1] == start:
            g['ranges'][-1][1] = start + r['bytes']
        else:
            g['ranges'].append([start, start + r['bytes']])
        g['bytes'] += r['bytes']
        g['n'] += 1
        g['digest'] = r['digest'] if g['n'] == 1 else None
    out = []
    for key in order:
        g = groups[key]
        g.pop('n')
        out.append(g)
    return out


def _merge_ranges(recs):
    out, run = [], []
    for r in recs:
        if r['op'] in RW and not r['errno']:
            run.append(r)
            continue
        out.extend(_merge(run))
        run = []
        out.append(r)
    out.extend(_merge(run))
    return out


def _transfers(recs, reference=False):
    """Copy transfers in a canonical order. The reference streams each allowed file with `pipeline(readStream,
    writeStream)`: the read stream starts reading (and closes its handle at end of file or on teardown) while the
    exclusive create and the writes are still in flight, so the relative order of source-handle and destination-handle
    primitives inside one transfer is timing. Each transfer window (from the destination create until both handles are
    closed, or a primitive on another handle) becomes: create, the source-handle primitives, then the
    destination-handle primitives, each side in its own order. Barriers travel with their target primitive (a
    both sides trace a barrier after the issue of its target primitive). `reference` marks the TS trace: there, when
    the create failed, source reads that raced it are dropped (a stream torn down before its first read is the same
    schedule). The implementation's trace keeps them, so an implementation that reads after a failed create differs."""
    out, i, n = [], 0, len(recs)
    while i < n:
        r = recs[i]
        if r['op'] != 'create':
            out.append(r)
            i += 1
            continue
        src = None
        for q in reversed(out):
            if q['op'] == 'open' and q.get('handle') is not None:
                src = q['handle']
                break
        dst = r.get('handle')
        side = lambda u: u[0].get('handle')
        head, units, j, closed = [r], [], i + 1, set()
        while j < n:
            q = recs[j]
            if q['op'] == 'barrier':                    # barriers follow the issue of their target: attach to it
                owner = next((u for u in reversed(units) if u[0]['op'] == q['target'] and u[0]['path'] == q['path']), None)
                if owner is None and r['op'] == q['target'] and r['path'] == q['path']:
                    owner = head
                if owner is None:
                    break
                owner.append(q)
                j += 1
                continue
            h = q.get('handle')
            if h is None or h not in (src, dst):
                break
            units.append([q])
            j += 1
            if q['op'] == 'close':
                closed.add(h)
                if closed >= {x for x in (src, dst) if x is not None}:
                    break
        src_units = [u for u in units if side(u) == src]
        if reference and r.get('errno'):
            src_units = [u for u in src_units if u[0]['op'] != 'read']
        dst_units = [u for u in units if side(u) == dst]
        out.extend(head)
        for u in src_units + dst_units:
            out.extend(u)
        i = j
    return out


def normalize_trace(events, roots, mode=None, reference=False):
    """Normalised primitive trace: logical paths, logical handles, dev/ino as same/different relations, contiguous
    ranges per handle, listing candidates grouped per candidate (timing of independent slots is not compared)."""
    recs = pair_trace(events, roots)
    _name_handles(recs)
    if mode == 'copy':
        recs = _transfers(recs, reference)
    is_list = mode == 'list'
    prefix, groups, suffix, seen_candidate = [], {}, [], False
    for r in recs:
        c = _candidate(r['path'], is_list)
        if c is None:
            (suffix if seen_candidate else prefix).append(r)
        else:
            seen_candidate = True
            groups.setdefault(c, []).append(r)
    seq = _merge_ranges(prefix)
    for c in groups:                                    # dict order = first issue = raw enumeration order
        seq += _merge_ranges(groups[c])
    seq += _merge_ranges(suffix)
    idents = {}
    out = []
    for r in seq:
        r = dict(r)
        r.pop('id', None)
        r.pop('_hpath', None)
        dev, ino = r.pop('dev', None), r.pop('ino', None)
        if dev is not None and ino is not None:
            r['ident'] = idents.setdefault((dev, ino), 'i%d' % len(idents))
        if r.get('op') in ('readdir',):
            r['digest'] = None                          # directory digests depend on the sides' own encodings
        if r.get('op') in ('lstat', 'fstat', 'realpath', 'mkdir', 'rename', 'rm', 'open', 'close', 'spawn', 'readdir'):
            r['digest'] = r['digest'] if r.get('op') == 'spawn' else None
            r['offset'] = None
        out.append(r)
    return out


def concurrency_info(events, roots):
    """Per-candidate first issue / last completion and the maximum number of simultaneously outstanding requests."""
    outstanding, peak = 0, 0
    first, last = {}, {}
    pos = {}
    for i, e in enumerate(events):
        if e.get('event') == 'issue':
            outstanding += 1
            peak = max(peak, outstanding)
            pos[e['id']] = e
            c = _candidate(logical_path(e.get('path'), roots), True)
            if c:
                first.setdefault(c, i)
        elif e.get('event') == 'complete':
            outstanding -= 1
            src = pos.get(e['id'])
            c = _candidate(logical_path((src or e).get('path'), roots), True)
            if c:
                last[c] = i
    return {'max_outstanding': peak, 'first_issue': first, 'last_complete': last}


def count_ops(events, op):
    return sum(1 for e in events if e.get('event') == 'issue' and e.get('op') == op)
