"""Shared helpers of the OpenCode import tests (test_impl.py, q.py, oracle.py, reference.mjs callers): fixture trees, SQLite
template images, the fake exporter placement, running a runner or the TypeScript reference on one JSONL case, tree snapshots
and trace normalisation (DESIGN sections 5-7). Synthetic private roots only: never a real home, conversation or credential.

Concrete case (DESIGN section 7), after test_impl.expand_case:
  {name, mode, folder, thread, sourceHome, homedir, env:{mode:inherited|full, values}, binary, tempRoot, nowMs, rpcMs, umask,
   tree:[...], db:{path, ddl, inserts, generate, wal, pragmas, mode}, export:{bytes, exit, mode, term, ...}|null,
   schedule:{rules}, planJson, watchdog, kills, expected, policies, slow}
Path tokens (in tree paths, folder, thread, sourceHome, homedir, binary, tempRoot, env values, db/schedule paths, planJson):
  $ROOT $SRC $HOME $BIN $TMP $FOLDER and the realpath forms $ROOT_REAL $SRC_REAL $HOME_REAL $BIN_REAL $TMP_REAL $FOLDER_REAL.
Byte recipes: {"utf8":S} {"hex":H} {"repeat":RECIPE,"count":N} {"concat":[RECIPE,...]} {"blob":NAME}.
Typed SQL values: {"kind":"null"} {"kind":"integer","decimal":"..."} {"kind":"real","bits":"16 hex"} {"kind":"text","text":..}
  {"kind":"blob","hex":..}.
"""
import sys
sys.dont_write_bytecode = True
import copy
import hashlib
import json
import os
import re
import shutil
import signal
import sqlite3
import stat
import struct
import subprocess
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
WORK = HERE / 'work'
NODE = os.environ.get('OPENCODE_NODE') or os.environ.get('GROK_NODE') or shutil.which('node') or '/opt/homebrew/bin/node'
FAKE_EXPORTER = HERE / 'fake_exporter.py'
TOKENS = ('ROOT', 'SRC', 'HOME', 'BIN', 'TMP', 'FOLDER')
# environment names the importer reads itself: scrubbed from the runner's base environment unless a case sets them
SCRUBBED_ENV = ('XDG_DATA_HOME', 'OPENCODE_DB')
# The environment at import, before the compiler driver points TMPDIR at the work folder: both sides inherit this one.
PARENT_ENV = dict(os.environ)


def set_transpiler(path):
    """Apply a --ts-transpiler override to this process and to the snapshot runners inherit from."""
    os.environ['GROK_TS_TRANSPILER'] = path
    PARENT_ENV['GROK_TS_TRANSPILER'] = path

STAGES = ['input', 'db-parent', 'db-lstat', 'db-realpath', 'db-open', 'schema-session', 'schema-message', 'select', 'count-prepare',
          'count', 'db-close', 'read-admission', 'list-clock', 'temp-create', 'output-open', 'spawn', 'wait', 'term', 'kill', 'give-up',
          'output-close', 'output-stat', 'output-read', 'cleanup', 'parse', 'shape', 'convert', 'plan', 'result']
CLASSES = ['none', 'EngineImportError', 'RawOsError', 'CoreError']
CODES = ['none', 'not-found', 'read-failed', 'engine-missing', 'timeout', 'InputTooLarge', 'InternalInvariant']
OPS = ['realpath', 'lstat', 'db_open_readonly', 'db_query', 'db_close', 'mkdtemp', 'open', 'spawn', 'observe', 'kill', 'close', 'stat', 'read',
       'rm', 'clock_now']


class TraceAssertion(AssertionError):
    """A trace normalisation precondition failed (fixture root prefix, temp prefix/location, handle naming)."""


# ---- fixture recipes ---------------------------------------------------------------------------------------------
def bytes_of(recipe, blobs=None):
    """Raw bytes of a recipe; JSON text such as 1e400, escaped surrogates or invalid UTF-8 is never round-tripped."""
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


def tokens_of(root):
    root = Path(root)
    base = {'ROOT': root, 'SRC': root / 'src', 'HOME': root / 'home', 'BIN': root / 'bin', 'TMP': root / 'tmp', 'FOLDER': root / 'folder'}
    out = {}
    for k, p in base.items():
        out['$' + k] = str(p)
        out['$' + k + '_REAL'] = str(Path(os.path.realpath(root)) / p.relative_to(root)) if p != root else str(Path(os.path.realpath(root)))
    return out


_TOKEN_RE = re.compile(r'\$(' + '|'.join(sorted((t + '_REAL' for t in TOKENS), key=len, reverse=True)) + '|' + '|'.join(sorted(TOKENS, key=len, reverse=True)) + r')')


def expand_tokens(value, tokens):
    if isinstance(value, str):
        return _TOKEN_RE.sub(lambda m: tokens['$' + m.group(1)], value)
    return value


def expand_deep(value, tokens):
    """Tokens in every string of a JSON value (dict keys included)."""
    if isinstance(value, str):
        return expand_tokens(value, tokens)
    if isinstance(value, list):
        return [expand_deep(v, tokens) for v in value]
    if isinstance(value, dict):
        return {expand_tokens(k, tokens): expand_deep(v, tokens) for k, v in value.items()}
    return value


def build_tree(entries, tokens, blobs=None):
    """Create dir/file/symlink/hardlink/fifo entries; modes are applied last, deepest first (nothing is locked early).
    Entry: {path, kind, mode?, bytes? (file), target? (symlink), of? (hardlink)}."""
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
        elif kind in ('fifo', 'FIFO'):
            os.mkfifo(path)
        else:
            raise ValueError('bad tree kind ' + kind)
        if 'mode' in e and kind != 'symlink':
            modes.append((path, e['mode']))
    for path, mode in sorted(modes, key=lambda m: -len(m[0].parts)):
        os.chmod(path, mode)


def logical_path(path, roots):
    """Real fixture roots -> tokens, longest root first (a side that already prints tokens is unchanged)."""
    if not isinstance(path, str):
        return path
    for token, root in sorted(roots.items(), key=lambda kv: -len(kv[1])):
        if root and root in path:
            path = path.replace(root, token)
    return path


def snapshot(root, roots=None):
    """{relative path: {kind, mode, size, sha256 | target}} of everything below root ('.' is root itself); symlink targets inside
    the case become logical so the two sides' trees compare. Sidecars (-wal, -shm) are ordinary files."""
    root = Path(root)
    out = {}
    if not os.path.lexists(root):
        return out
    if os.path.islink(root):
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
                if data[:16] == b'SQLite format 3\x00' and len(data) >= 100:
                    # Bytes 96..100 (SQLITE_VERSION_NUMBER) name the library that last wrote the file; the reference
                    # (Node) and the host (rusqlite) link different SQLite patch releases. Bytes 92..96
                    # (version-valid-for, a copy of the change counter) stay compared.
                    data = data[:96] + bytes(4) + data[100:]
                out[rel] = {'kind': 'file', 'mode': mode, 'size': len(data), 'sha256': hashlib.sha256(data).hexdigest(), 'nlink': st.st_nlink}
            except PermissionError:
                out[rel] = {'kind': 'file', 'mode': mode, 'unreadable': True}
        else:
            out[rel] = {'kind': 'fifo' if stat.S_ISFIFO(st.st_mode) else 'other', 'mode': mode}
    visit(root, '.')
    return out


def clean(path):
    """Remove a work directory even when the importer or a fixture left unreadable modes behind."""
    path = Path(path)
    if not os.path.lexists(path):
        return
    if os.path.islink(path):
        os.unlink(path)
        return
    for dirpath, dirnames, _ in os.walk(path):
        for d in dirnames:
            p = os.path.join(dirpath, d)
            if not os.path.islink(p):
                try:
                    os.chmod(p, 0o700)
                except OSError:
                    pass
    shutil.rmtree(path, ignore_errors=True)


# ---- SQLite database images ----------------------------------------------------------------------------------------
def typed_py(v):
    """Python value of a typed SQL value (REAL from its IEEE-754 bits, INTEGER from its decimal string)."""
    k = v['kind']
    if k == 'null':
        return None
    if k == 'integer':
        return int(v['decimal'])
    if k == 'real':
        return struct.unpack('>d', bytes.fromhex(v['bits']))[0]
    if k == 'text':
        return v['text']
    if k == 'blob':
        return bytes.fromhex(v['hex'])
    raise ValueError('bad typed value %r' % (v,))


def typed_json(x):
    """Typed SQL value of a Python value (inverse of typed_py for what sqlite3 returns)."""
    if x is None:
        return {'kind': 'null'}
    if isinstance(x, bool):
        raise ValueError('bool is not a SQL storage class here')
    if isinstance(x, int):
        return {'kind': 'integer', 'decimal': str(x)}
    if isinstance(x, float):
        return {'kind': 'real', 'bits': struct.pack('>d', x).hex()}
    if isinstance(x, str):
        return {'kind': 'text', 'text': x}
    return {'kind': 'blob', 'hex': bytes(x).hex()}


def generated_rows(db, cols_of):
    """Rows of the `generate` block: {table: [[typed...]...]}. sessions ids S0000.. (parent NULL, directory generate.directory,
    time_updated 1700000000000 + 1000*i); messagesPerSession rows per session; invalidNewestId replaces the id of the newest
    session (last ordinal). Only columns the DDL has are filled; the others are NULL."""
    g = db.get('generate')
    if not g:
        return {}
    n = int(g.get('sessions', 0))
    per = int(g.get('messagesPerSession', 0))
    ids = ['S%04d' % i for i in range(n)]
    if n and g.get('invalidNewestId') is not None:
        ids[-1] = g['invalidNewestId']
    sess = []
    for i, sid in enumerate(ids):
        vals = {'id': {'kind': 'text', 'text': sid}, 'directory': {'kind': 'text', 'text': g.get('directory', '/work')},
                'title': {'kind': 'text', 'text': 'Title %d' % i},
                'time_updated': {'kind': 'integer', 'decimal': str(1700000000000 + 1000 * i)}}
        sess.append([vals.get(c, {'kind': 'null'}) for c in cols_of('session')])
    msgs = []
    for i, sid in enumerate(ids):
        for j in range(per):
            vals = {'session_id': {'kind': 'text', 'text': sid}, 'id': {'kind': 'text', 'text': 'M%04d-%d' % (i, j)},
                    'time_created': {'kind': 'integer', 'decimal': str(1700000000000 + 1000 * i + j)}}
            msgs.append([vals.get(c, {'kind': 'null'}) for c in cols_of('message')])
    return {'session': sess, 'message': msgs}


def _db_key(db):
    spec = {k: v for k, v in db.items() if k not in ('path', 'mode')}
    return hashlib.sha256(json.dumps(spec, sort_keys=True).encode()).hexdigest()[:20]


def build_template(db):
    """The closed template image of a db spec (cached by content, built once) -> directory holding opencode.db (+ -wal/-shm for WAL
    cases). One image is copied identically to every side (place_db); nothing is rebuilt per side, so physical insertion order is
    shared. WAL: the writer stays open with autocheckpoint disabled while db/-wal/-shm are copied; the WAL-only row is asserted
    visible from a copy."""
    out = WORK / 'templates' / _db_key(db)
    if (out / 'opencode.db').exists():
        return out
    WORK.mkdir(parents=True, exist_ok=True)
    tmp = Path(tempfile.mkdtemp(dir=WORK, prefix='tpl-'))
    path = tmp / 'opencode.db'
    conn = sqlite3.connect(str(path), isolation_level=None)
    wal = bool(db.get('wal'))
    try:
        if wal:
            conn.execute('PRAGMA journal_mode=WAL')
            conn.execute('PRAGMA wal_autocheckpoint=0')
        for p in db.get('pragmas', []):
            conn.execute(p)
        for ddl in db.get('ddl', []):
            conn.execute(ddl)
        cols_of = lambda t: [r[1] for r in conn.execute('PRAGMA table_info(%s)' % t)]
        gen = generated_rows(db, cols_of)
        explicit = {}
        counts = {}
        conn.execute('BEGIN')
        for ins in db.get('inserts', []):
            explicit.setdefault(ins['table'], []).append(ins)
        tables = list(dict.fromkeys(list(gen) + list(explicit)))
        for table in tables:
            rows_gen = list(gen.get(table, []))
            cols_gen = cols_of(table)
            plan = [(cols_gen, r) for r in rows_gen]
            for ins in explicit.get(table, []):
                cols = ins['columns']
                for j, r in enumerate(ins['rows']):
                    if ins.get('overrideGenerated') and j < len(plan):
                        plan[j] = (cols, r)            # explicit rows override generated ones by ordinal, not by id
                    else:
                        plan.append((cols, r))
            for cols, r in plan:
                conn.execute('INSERT INTO %s(%s) VALUES (%s)' % (table, ','.join(cols), ','.join('?' * len(cols))), [typed_py(v) for v in r])
            counts[table] = len(plan)
        conn.execute('COMMIT')
        out.mkdir(parents=True, exist_ok=True)
        if wal:
            wal_file = Path(str(path) + '-wal')
            assert wal_file.exists() and wal_file.stat().st_size > 0, 'WAL template has no WAL content'
            for suffix in ('', '-wal', '-shm'):
                src = Path(str(path) + suffix)
                if src.exists():
                    shutil.copyfile(src, out / ('opencode.db' + suffix))
            probe = Path(tempfile.mkdtemp(dir=WORK, prefix='tplchk-'))
            try:
                for suffix in ('', '-wal', '-shm'):
                    src = out / ('opencode.db' + suffix)
                    if src.exists():
                        shutil.copyfile(src, probe / ('opencode.db' + suffix))
                chk = sqlite3.connect(str(probe / 'opencode.db'))
                table = next(iter(counts)) if counts else None
                if table:
                    seen = chk.execute('SELECT COUNT(*) FROM %s' % table).fetchone()[0]
                    assert seen == counts[table], 'WAL-only rows are not visible from the copied image (%d != %d)' % (seen, counts[table])
                chk.close()
            finally:
                clean(probe)
        else:
            conn.close()
            conn = None
            shutil.copyfile(path, out / 'opencode.db')
    finally:
        if conn is not None:
            conn.close()
        clean(tmp)
    return out


def place_db(db, tokens):
    """Copy the template image (and its WAL sidecars) to db.path; every side gets byte-identical files."""
    tpl = build_template(db)
    dest = Path(expand_tokens(db['path'], tokens))
    dest.parent.mkdir(parents=True, exist_ok=True)
    mode = db.get('mode', 0o644)
    for suffix in ('', '-wal', '-shm'):
        src = tpl / ('opencode.db' + suffix)
        if src.exists():
            shutil.copyfile(src, str(dest) + suffix)
            os.chmod(str(dest) + suffix, mode)
    return dest


# ---- fake exporter -------------------------------------------------------------------------------------------------
def place_exporter(case, tokens, binary):
    """Executable `binary` (a copy of fake_exporter.py with an absolute python shebang), its sibling scenario and output bytes.
    Returns the log path. The scenario is case['export'] minus `bytes`/`expect`, plus expect{thread, cwd, env, detached}."""
    exe = Path(binary)
    exe.parent.mkdir(parents=True, exist_ok=True)
    src = FAKE_EXPORTER.read_text(encoding='utf-8').split('\n', 1)
    exe.write_text('#!%s\n%s' % (sys.executable, src[1]), encoding='utf-8')
    exe.chmod(0o755)
    exp = case['export']
    scenario = {k: v for k, v in exp.items() if k not in ('bytes', 'expect', 'env')}
    expect = {'thread': expand_tokens(case.get('thread', ''), tokens), 'cwd': expand_tokens(case.get('folder', ''), tokens)}
    expect.update(exp.get('expect', {}))
    if 'env' in exp:
        expect['env'] = dict(expect.get('env', {}), **exp['env'])
    scenario['expect'] = expect
    Path(str(exe) + '.scenario.json').write_text(json.dumps(expand_deep(scenario, tokens)), encoding='utf-8')
    out = Path(str(exe) + '.out')
    out.write_bytes(bytes_of(exp.get('bytes', {'utf8': ''}), case.get('blobs')))
    log = Path(str(exe) + '.log')
    if log.exists():
        log.unlink()
    return log


def read_exporter_log(path):
    p = Path(path) if path else None
    if not p or not p.exists():
        return None
    try:
        return json.loads(p.read_text(encoding='utf-8'))
    except ValueError:
        return {'unreadable': True, 'violations': ['exporter log is not valid JSON']}


# ---- running a runner ----------------------------------------------------------------------------------------------
def reference_command():
    return [NODE, '--import', HERE / 'reference-hooks.mjs', HERE / 'reference.mjs']


def make_config(case, tokens, trace=True, artifact_dir=None):
    """The JSON line both the Rust host and the TS reference read (host.rs header)."""
    t = lambda k, default=None: expand_tokens(case.get(k, default), tokens) if (k in case or default is not None) else None
    env = case.get('env') or {'mode': 'inherited', 'values': {}}
    cfg = {
        'mode': case['mode'], 'folder': t('folder'), 'thread': t('thread'), 'sourceHome': t('sourceHome', '$SRC'),
        'homedir': t('homedir', '$HOME'), 'env': {'mode': env.get('mode', 'inherited'), 'values': expand_deep(env.get('values', {}), tokens)},
        'binary': t('binary', '$BIN/exporter'), 'tempRoot': t('tempRoot', '$TMP'), 'nowMs': case.get('nowMs'), 'rpcMs': case.get('rpcMs'),
        'schedule': expand_deep(case.get('schedule', {'rules': []}), tokens), 'planJson': t('planJson'), 'trace': trace,
        'artifactDir': artifact_dir,
    }
    return {k: v for k, v in cfg.items() if v is not None}


def runner_env(case, tokens):
    """Process environment of a runner: the parent's minus the importer's own variables, plus the case values when the case
    inherits (the effective table the child sees then is exactly this environment)."""
    env = {k: v for k, v in PARENT_ENV.items() if k not in SCRUBBED_ENV}
    spec = case.get('env') or {}
    if spec.get('mode', 'inherited') == 'inherited':
        env.update(expand_deep(spec.get('values', {}), tokens))
    return env


def _kill_group(pid):
    try:
        os.killpg(pid, signal.SIGKILL)
    except (ProcessLookupError, PermissionError):
        pass


def parse_trace(stderr_text):
    """(events, other lines): every stderr line starting with {"trace" is one event; anything else is diagnostics."""
    events, other = [], []
    for line in stderr_text.split('\n'):
        if line.startswith('{"trace"'):
            events.append(json.loads(line)['trace'])
        elif line:
            other.append(line)
    return events, other


def run_side(case, side, command, *, trace=True, watchdog=None, root=None):
    """Build the trees, the SQLite image and the fake exporter of `case` under WORK/cases/<side>/<name>, run `command` on its
    config, snapshot before/after. Never raises on a runner failure: see the keys `returncode`, `watchdog`.
    `side` names the runner ('ts', 'py_rust', ...). Returns a dict:
      result (the stdout envelope), meta, trace (raw events), other (stderr diagnostics), exporter (fake log), before, after, roots,
      returncode, watchdog, stdout_lines, root, stderr."""
    root = Path(root) if root else WORK / 'cases' / side / re.sub(r'[^A-Za-z0-9_.-]', '_', case['name'])
    clean(root)
    root.mkdir(parents=True)
    root = root.resolve()
    tokens = tokens_of(root)
    (root / 'folder').mkdir(exist_ok=True)
    build_tree(case.get('tree', []), tokens, case.get('blobs', {}))
    if case.get('db'):
        place_db(case['db'], tokens)
    cfg = make_config(case, tokens, trace, str(root / 'artifacts'))
    (root / 'artifacts').mkdir(exist_ok=True)
    log = None
    if case.get('export') is not None:
        binary = Path(cfg.get('binary', ''))
        if str(binary).startswith(str(root)):
            log = place_exporter(case, tokens, binary)
    roots = {k: v for k, v in tokens.items()}
    before = {n: snapshot(root / n, roots) for n in ('src', 'home', 'tmp', 'folder')}
    umask = case.get('umask', 0o022)
    cmd = ['/bin/sh', '-c', 'umask "$0"; exec "$@"', '%o' % umask] + [str(x) for x in command]
    lines_in = json.dumps(cfg, ensure_ascii=False).encode('utf-8') + b'\n'
    limit = watchdog or case.get('watchdog') or 120
    proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=runner_env(case, tokens),
                            cwd=str(HERE), start_new_session=True)
    killed = False
    try:
        out, err = proc.communicate(lines_in, timeout=limit)
    except subprocess.TimeoutExpired:
        killed = True                                   # a watchdog kill is never a passing timeout
        _kill_group(proc.pid)
        lg = read_exporter_log(log) or {}
        if lg.get('pid'):                               # only PIDs this run spawned
            try:
                os.kill(lg['pid'], signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                pass
        try:
            out, err = proc.communicate(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
            out, err = b'', b''
    after = {n: snapshot(root / n, roots) for n in ('src', 'home', 'tmp', 'folder')}
    lines = [l for l in out.decode('utf-8', 'replace').split('\n') if l]
    rows = []
    for l in lines:
        try:
            rows.append(json.loads(l))
        except ValueError:
            rows.append({'status': 'unparsable-stdout', 'line': l[:200]})
    meta = next((r['meta'] for r in rows if isinstance(r, dict) and 'meta' in r), None)
    results = [r for r in rows if not (isinstance(r, dict) and 'meta' in r)]
    err_text = err.decode('utf-8', 'replace')
    events, other = parse_trace(err_text)
    return {
        'result': results[0] if results else None, 'meta': meta, 'trace': events, 'other': other[-40:], 'stderr': '\n'.join(other[-40:]),
        'exporter': read_exporter_log(log), 'before': before, 'after': after, 'roots': roots, 'tokens': tokens, 'returncode': proc.returncode,
        'watchdog': killed, 'stdout_lines': lines, 'root': str(root),
    }   # the trees stay for diagnosis; the caller removes them with clean(root) once the case passes


def sqlite_version(command):
    """`SELECT sqlite_version()` from the runner's metadata mode (a separate connection; never an importer statement)."""
    p = subprocess.run([str(x) for x in command] + ['--sqlite-version'], capture_output=True, text=True, timeout=60)
    return json.loads(p.stdout.strip().splitlines()[-1]).get('sqliteVersion') if p.returncode == 0 and p.stdout.strip() else None


# ---- trace normalisation (DESIGN section 5) -----------------------------------------------------------------------
def _walk(v, fn):
    if isinstance(v, str):
        return fn(v)
    if isinstance(v, list):
        return [_walk(x, fn) for x in v]
    if isinstance(v, dict):
        return {k: _walk(x, fn) for k, x in v.items()}
    return v


def normalize_trace(events, roots, *, clocks=True):
    """Normalised trace: fixture roots -> tokens, request ids dropped, opaque handles -> '<kind>#<k>' in creation order, random
    mkdtemp suffixes -> '@<k>' (after asserting the result is `<issued prefix>` + exactly 6 [A-Za-z0-9]), clock values to
    semantic form (monotonic readings dropped; observe deadlines kept as budgets relative to the last monotonic reading).
    Never sorts, never drops failed calls, never reorders cleanup; slash differences in inputs stay. Raises TraceAssertion."""
    names = {}
    counts = {}
    temp_suffix = {}
    prefix_of = {}                                       # request id -> the mkdtemp prefix it issued
    issued = {}                                          # request id -> {phase, clock} of its issue event
    last_mono = [None]
    out = []

    def logical(s):
        return logical_path(s, roots)

    for e in events:
        e = copy.deepcopy(e)
        ev = e.get('event')
        eid = e.get('id')
        e = _walk(e, logical)
        if e.get('op') == 'mkdtemp':
            if ev == 'issue':
                prefix_of[eid] = (e.get('args') or {}).get('prefix')
                if not str(prefix_of[eid]).endswith('opencode-'):
                    raise TraceAssertion('mkdtemp prefix %r does not end with opencode-' % prefix_of[eid])
            elif ev == 'complete' and not e.get('error'):
                path = (e.get('result') or {}).get('path')
                pre = prefix_of.get(eid)
                m = re.fullmatch(r'(.*)([A-Za-z0-9]{6})', path or '')
                if not m or m.group(1) != pre:
                    raise TraceAssertion('mkdtemp result %r is not prefix %r + 6 random characters' % (path, pre))
                temp_suffix[m.group(2)] = '@%d' % (len(temp_suffix) + 1)
        for suffix, tag in temp_suffix.items():
            e = _walk(e, lambda s, suffix=suffix, tag=tag: s.replace('opencode-' + suffix, 'opencode-' + tag))
        op = e.get('op')
        args = e.get('args') if isinstance(e.get('args'), dict) else {}
        if ev == 'issue':
            issued[eid] = {'phase': args.get('phase'), 'clock': args.get('clock')}
        info = issued.get(eid, {})
        # opaque handles: created by the result of db_open_readonly / prepare / open / spawn, named in creation order
        res = e.get('result')
        create = {'db_open_readonly': ('connection', 'conn'), 'open': ('handle', 'file'), 'spawn': ('child', 'child')}.get(op)
        if op == 'db_query' and info.get('phase') == 'prepare':
            create = ('statement', 'stmt')
        if ev == 'complete' and create and isinstance(res, dict) and not e.get('error') and isinstance(res.get(create[0]), int):
            field, kind = create
            counts[kind] = counts.get(kind, 0) + 1
            names[(kind, res[field])] = '%s#%d' % (kind, counts[kind])
            res[field] = names[(kind, res[field])]
        h = e.get('handle')
        if h is not None:
            if op == 'db_query':
                kind = 'conn' if info.get('phase') == 'prepare' else 'stmt'
            else:
                kind = {'db_close': 'conn', 'close': 'file', 'spawn': 'file', 'observe': 'child', 'kill': 'child'}.get(op)
            e['handle'] = names.get((kind, h), 'unknown') if kind else 'unknown'
        # clocks: monotonic readings are per-run origins; wall readings (frozen by nowMs) stay exact
        if clocks:
            if op == 'clock_now' and ev == 'complete' and isinstance(res, dict) and 'ms' in res and info.get('clock') == 'monotonic':
                last_mono[0] = res['ms']
                res['ms'] = None
            if op == 'observe':
                if ev == 'issue' and 'deadlineMs' in args:
                    d = e['args'].pop('deadlineMs')
                    e['args']['budgetMs'] = (d - last_mono[0]) if last_mono[0] is not None else None
                if ev == 'complete' and isinstance(res, dict):
                    res['monoMs'] = None
        e.pop('seq', None)
        e.pop('id', None)
        out.append(e)
    return out


def count_ops(events, op):
    return sum(1 for e in events if e.get('event') == 'issue' and e.get('op') == op)
