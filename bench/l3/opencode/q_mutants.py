"""16 isolated source mutants M01-M16 of the OpenCode import (DESIGN section 7 mutants table), one at a time, on COPIES of the .pruv
modules in work/mutants/<id>/ (the originals are never touched), built on the rs compiler + rust backend (rs/rust) only.

    GROK_TS_TRANSPILER=/path/to/typescript python3 bench/l3/opencode/q_mutants.py [--only M03,M05] [--check] [--list] [--full]

How a mutation is located: EXACT markers only (DESIGN: "Add exact MUT:Mxx markers at the intended decision, not regex guesses").
The module owner adds ONE line at the intended decision, of the form

    <the baseline statement>  // MUT:M07 => <replacement statement>

q_mutants.py replaces that whole line (indent kept) by the replacement statement. The marker text is the literal `MUT:Mxx`. It must
exist EXACTLY ONCE in the .pruv files of this directory, and in one of the target files listed for that mutant. `--check` confirms that
(and that the killing fixtures exist) without building or running anything; it fails until the owners have added the markers.

Kill = a verifier rejection of the mutated source, or a normal run of the mutant runner whose result/trace/trees differ from the pinned
TypeScript beyond the policies (oracle.compare unexplained). NOT kills: compilation failure that is not a verifier rejection, runner crash,
harness error, watchdog kill. The unmutated rs/rust runner (work/rs_rust/runner, built by test_impl.py) must first pass each killing
fixture ("baseline-failed" otherwise), and a baseline trace must never contain a pipe-stdout spawn (the M10-only flag).
Report: work/mutants/report.json.
"""
import sys
sys.dont_write_bytecode = True
import argparse
import json
import os
import re
import time
from pathlib import Path

from common import HERE, WORK, clean, set_transpiler

MUT_DIR = WORK / 'mutants'
FIXTURES = HERE / 'fixtures'
VERIFIER_WORDS = ('ensures', 'invariant', 'unproved', 'verification', 'cannot prove', 'verifier', 'requires', 'timeout')
MARK = re.compile(r'^(?P<indent>[ \t]*)(?P<code>.*?)[ \t]*//[ \t]*MUT:(?P<id>M\d\d)[ \t]*=>[ \t]*(?P<alt>.*?)[ \t]*$')

# id, name, target files (the marker must be in one of them), killing fixtures (DESIGN section 7), the edit the replacement performs.
# `reconcile`: no module file existed when this table was written, so each marker position was chosen from the DESIGN-described logic.
MUTANTS = [
    dict(id='M01', name='memory-means-default-db', files=['path.pruv'], kills=['F01-db-memory'],
         edit='in the OPENCODE_DB resolution: exactly `:memory:` takes the default `join(home,"opencode.db")` branch instead of "no database"'),
    dict(id='M02', name='follow-final-db-link', files=['admit.pruv'], kills=['F02-db-final-symlink'],
         edit='the lstat kind test accepts kind 3 (symlink) as well as 1 (regular file) before the realpath(DB) step'),
    dict(id='M03', name='skip-required-column-check', files=['sql.pruv'], kills=['F03-missing-title', 'F03-missing-id'],
         edit='the session required-name gate (all five session names present) is bypassed: treated as satisfied (the message.session_id gate stays)'),
    dict(id='M04', name='admit-child-archived-empty', files=['sql.pruv'], kills=['F04-exclude-child-archived-empty'],
         edit='the selection prefix drops the `parent_id IS NULL` / archive / EXISTS-message predicates (one of them; the marker line removes the appended fragments)'),
    dict(id='M05', name='folderkey-for-list-sql', files=['sql.pruv'], kills=['F04-list-slash-asymmetry'],
         edit='the list selection parameters use folderKey-normalised stored-directory semantics: the second IN parameter becomes the folder with ALL trailing slashes removed plus `/` of the raw input'),
    dict(id='M06', name='validate-before-limit', files=['sql.pruv', 'list.pruv'], kills=['F04-limit-51-invalid-newest'],
         edit='the SELECT is issued with a limit larger than 50 (e.g. no LIMIT parameter 50 -> 51+) and rows::decode output is cut to 50 valid rows afterwards (backfill)'),
    dict(id='M07', name='resort-converted-updated', files=['rows.pruv', 'list.pruv'], kills=['F04-raw-storage-order'],
         edit='validated rows are stably sorted by the converted (P9) updated number, descending, before the list is assembled'),
    dict(id='M08', name='non-strict-open-window', files=['list.pruv'], kills=['F05-age-120000'],
         edit='maybeOpen test `now - updatedMs < 120000` becomes `<= 120000`'),
    dict(id='M09', name='export-before-admission', files=['admit.pruv', 'export.pruv', 'list.pruv'], kills=['F06-wrong-folder'],
         edit='the folderKey(row.directory) == folderKey(folder) comparison no longer gates the export: a folder mismatch still proceeds to mkdtemp/spawn'),
    dict(id='M10', name='pipe-export-stdout', files=['export.pruv'], kills=['F07-stdout-file-large'],
         edit='the spawn request uses flags 1 (host drains a pipe to a discard sink) instead of flags 0 (the open output file as fd 1)'),
    dict(id='M11', name='size-before-exit-result', files=['export.pruv'], kills=['F09-nonzero-oversized'],
         edit='an oversized stat result overrides a pending failed-wait result: the failure is dropped and the oversized success is returned'),
    dict(id='M12', name='skip-malformed-message', files=['convert.pruv'], kills=['F10-late-malformed'],
         edit='a message that fails the object/info/parts shape check is skipped (continue) instead of failing the read with detail `shape`'),
    dict(id='M13', name='attach-after-empty-user', files=['entry.pruv'], kills=['F11-synthetic-detach'],
         edit='a user message without human text leaves `open` unchanged instead of setting it false (the next assistant attaches to the old turn)'),
    dict(id='M14', name='metadata-only-when-open', files=['convert.pruv', 'entry.pruv'], kills=['F11-orphan-metadata'],
         edit='the model pair and plan input of a nonsummary assistant are recorded only while `open` is true'),
    dict(id='M15', name='empty-plan-falls-back', files=['plan.pruv'], kills=['F14-empty-clears'],
         edit='when the newest candidate fits to zero items the search continues to older messages instead of ending with an absent plan'),
    dict(id='M16', name='omit-collector-trim-flag', files=['convert.pruv'], kills=['F15-turns-201', 'F15-bytes-over'],
         edit='omittedBefore is forced to false: the collector trimmed flag is ignored (except the explicit oversized branch)'),
]
BY_ID = {m['id']: m for m in MUTANTS}


def fixture_names():
    names = set()
    for f in sorted(FIXTURES.glob('F*.json')):
        raw = json.loads(f.read_text(encoding='utf-8'))
        for c in (raw['cases'] if isinstance(raw, dict) else raw):
            names.add(c['name'])
    return names


def scan_markers():
    """{id: [(file, line number, line)]} over every .pruv of this directory."""
    found = {}
    for p in sorted(HERE.glob('*.pruv')):
        for n, line in enumerate(p.read_text(encoding='utf-8').split('\n'), 1):
            for mid in re.findall(r'MUT:(M\d\d)', line):
                found.setdefault(mid, []).append((p.name, n, line))
    return found


def check():
    """-> (ok, rows): for every mutant: file present?, marker count, marker location, marker syntax, killing fixtures present."""
    found = scan_markers()
    have = fixture_names()
    rows, ok = [], True
    for m in MUTANTS:
        hits = found.get(m['id'], [])
        row = dict(id=m['id'], name=m['name'], files=m['files'], kills=m['kills'], markers=len(hits))
        missing_fixtures = [k for k in m['kills'] if k not in have]
        row['fixtures_missing'] = missing_fixtures
        present = [f for f in m['files'] if (HERE / f).exists()]
        if len(hits) == 0:
            row['status'] = 'marker-missing' + ('' if present else ' (target file not written yet: %s)' % ', '.join(m['files']))
        elif len(hits) > 1:
            row['status'] = 'marker-duplicated x%d (%s)' % (len(hits), ', '.join('%s:%d' % (h[0], h[1]) for h in hits))
        elif hits[0][0] not in m['files']:
            row['status'] = 'marker-in-wrong-file %s (expected %s)' % (hits[0][0], ' or '.join(m['files']))
        elif not MARK.match(hits[0][2]):
            row['status'] = 'marker-malformed (need `<statement> // MUT:%s => <replacement>`) at %s:%d' % (m['id'], hits[0][0], hits[0][1])
        else:
            row['status'] = 'ok %s:%d' % (hits[0][0], hits[0][1])
        if missing_fixtures:
            row['status'] += '; fixture missing: ' + ', '.join(missing_fixtures)
        row['ok'] = row['status'].startswith('ok') and not missing_fixtures
        ok = ok and row['ok']
        rows.append(row)
    return ok, rows


class NoMatch(Exception):
    pass


def mutate(mutant, out_dir):
    """Copy the .pruv files and the manifest to out_dir with the marker line of this mutant replaced. Raises NoMatch."""
    clean(out_dir)
    out_dir.mkdir(parents=True)
    names = sorted(p.name for p in HERE.glob('*.pruv'))
    hits = 0
    changed = []
    for n in names:
        text = (HERE / n).read_text(encoding='utf-8')
        lines = []
        for line in text.split('\n'):
            m = MARK.match(line)
            if m and m.group('id') == mutant['id']:
                lines.append(m.group('indent') + m.group('alt'))
                hits += 1
                changed.append(n)
            else:
                lines.append(line)
        (out_dir / n).write_text('\n'.join(lines), encoding='utf-8')
    if hits != 1:
        raise NoMatch('marker `// MUT:%s => ...` found %d times (need exactly 1)' % (mutant['id'], hits))
    if changed[0] not in mutant['files']:
        raise NoMatch('marker of %s is in %s, not in %s' % (mutant['id'], changed[0], mutant['files']))
    manifest = json.loads((HERE / 'pruv.json').read_text())
    for dep in manifest.get('dependencies', {}).values():       # paths are relative to the manifest: point them at the real packages
        dep['path'] = os.path.relpath((HERE / dep['path']).resolve(), out_dir.resolve())
    (out_dir / 'pruv.json').write_text(json.dumps(manifest) + '\n')
    return changed[0]


def pipe_spawns(run):
    return [e for e in run.get('trace', []) if e.get('op') == 'spawn' and e.get('event') == 'issue'
            and ((e.get('args') or {}).get('stdout') or {}).get('kind') == 'pipe']


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--only', help='comma list of mutant ids (M01,...)')
    ap.add_argument('--check', action='store_true', help='confirm every marker text exists exactly once in its target file; build and run nothing')
    ap.add_argument('--list', action='store_true', help='list the mutants with target files and killing fixtures')
    ap.add_argument('--full', action='store_true', help='also run every other fixture for a mutant not killed by its own (default: stop after the named fixtures)')
    ap.add_argument('--jobs', type=int, default=4)
    ap.add_argument('--ts-transpiler')
    args = ap.parse_args(argv)
    chosen = [m for m in MUTANTS if not args.only or m['id'] in args.only.split(',')]
    if args.list:
        for m in MUTANTS:
            print('%s %-28s %-24s kills: %s\n      edit: %s' % (m['id'], m['name'], '/'.join(m['files']), ', '.join(m['kills']), m['edit']))
        return 0
    ok, rows = check()
    if args.check:
        for r in rows:
            if args.only and r['id'] not in args.only.split(','):
                continue
            print('%s %-28s %s' % (r['id'], r['name'], r['status']))
        MUT_DIR.mkdir(parents=True, exist_ok=True)
        (MUT_DIR / 'check.json').write_text(json.dumps(rows, indent=1))
        print('all %d markers present exactly once' % len(rows) if ok else 'NOT READY: %d of %d mutants lack a unique marker or fixture' % (sum(not r['ok'] for r in rows), len(rows)))
        return 0 if ok else 1
    if args.ts_transpiler:
        set_transpiler(args.ts_transpiler)
    if not os.environ.get('GROK_TS_TRANSPILER'):
        print('GROK_TS_TRANSPILER (or --ts-transpiler) must name the typescript package', file=sys.stderr)
        return 2
    import test_impl as T                                   # heavy imports only for a real run
    c, g, _ = T.configure()
    host = T.host_artifacts()
    baseline_exe = WORK / 'rs_rust' / 'runner'
    if not baseline_exe.exists():
        print('no work/rs_rust/runner: run test_impl.py (verified build) first', file=sys.stderr)
        return 2
    cases, _ = T.load_fixtures(include_disabled=False, slow=True)
    by_name = {c_['name']: c_ for c_ in cases}
    MUT_DIR.mkdir(parents=True, exist_ok=True)
    report = []
    refs, base_cache = {}, {}
    t0 = time.monotonic()

    def reference(case):
        if case['name'] not in refs:
            refs[case['name']] = T.run_reference(case)
        return refs[case['name']]

    def baseline(case):
        if case['name'] not in base_cache:
            run = T.run_pair(case, 'mut_baseline', baseline_exe)
            ts, base = reference(case)
            verdict = T.judge(case, ts, base, run)
            base_cache[case['name']] = (run, verdict, pipe_spawns(run))
        return base_cache[case['name']]

    for m in chosen:
        row = dict(id=m['id'], name=m['name'], files=m['files'], fixtures=m['kills'])
        report.append(row)
        crow = next(r for r in rows if r['id'] == m['id'])
        if not crow['ok']:
            row.update(result='pattern-missing', detail=crow['status'])
            print('%s pattern-missing: %s' % (m['id'], crow['status']), flush=True)
            continue
        named = [by_name[n] for n in m['kills'] if n in by_name]
        bad_base = []
        for case in named:
            run, verdict, pipes = baseline(case)
            if not verdict['ok'] or pipes:
                bad_base.append('%s%s' % (case['name'], ' (baseline spawned with a pipe: the M10-only flag leaked)' if pipes else ''))
        if bad_base or len(named) != len(m['kills']):
            row.update(result='baseline-failed', detail='; '.join(bad_base) or 'killing fixture not selectable (slow/disabled?)')
            print('%s baseline-failed: %s' % (m['id'], row['detail']), flush=True)
            continue
        w = MUT_DIR / m['id']
        try:
            row['edit'] = 'marker in ' + mutate(m, w)
        except NoMatch as exc:
            row.update(result='pattern-missing', detail=str(exc))
            print('%s pattern-missing: %s' % (m['id'], exc), flush=True)
            continue
        built = g.compile_pruv('rs', str(w / 'root.pruv'), str(w), 'rust', T.compile_flags(host[2], w / 'pruv.json'))
        (w / 'compile.json').write_text(json.dumps(built, default=str, indent=1))
        if built['rc']:
            detail = str(built.get('errors') or built.get('stderr'))
            row.update(result='verifier-rejected' if any(x in detail.lower() for x in VERIFIER_WORDS) else 'build-error', detail=detail[-2000:])
            print('%s %s' % (m['id'], row['result']), flush=True)
            continue
        exe = T.link_runner(g, built, w, host)
        order = list(named)
        if args.full:
            order += [c_ for c_ in cases if c_['name'] not in set(m['kills']) and not c_.get('slow')]
        killed, crashes, same = [], [], 0
        for case in order:
            impl = T.run_pair(case, 'mut_' + m['id'], exe)
            if not T.impl_clean(impl):
                crashes.append(case['name'])                 # a crash / watchdog kill / harness error is never a kill
                continue
            ts, base = reference(case)
            verdict = T.judge(case, ts, base, impl)
            if verdict['ok']:
                same += 1
                if impl.get('root'):
                    clean(impl['root'])
            else:
                killed.append(case['name'])
                if not args.full:
                    break
        by_named = [n for n in killed if n in m['kills']]
        row.update(result='differential-killed' if killed else 'survived', killed_by=killed[:20], killed_by_named_fixture=bool(by_named),
                   crashes=crashes[:20], unchanged_fixtures=same)
        if not killed:
            row['note'] = 'no run of the named fixtures distinguishes it (crashes are not kills); not a claim of equivalence'
        print('%s %-28s %s%s' % (m['id'], m['name'], row['result'], (' by ' + ', '.join(killed[:3])) if killed else ''), flush=True)
        (MUT_DIR / 'report.json').write_text(json.dumps(report, indent=1))
    (MUT_DIR / 'report.json').write_text(json.dumps(report, indent=1))
    res = [r.get('result') for r in report]
    killed_n = sum(r in ('differential-killed', 'verifier-rejected') for r in res)
    print('killed %d/%d (differential %d, verifier %d); survived %d; build errors %d; pattern missing %d; baseline failed %d; %.0f s' % (
        killed_n, len(report), res.count('differential-killed'), res.count('verifier-rejected'), res.count('survived'), res.count('build-error'),
        res.count('pattern-missing'), res.count('baseline-failed'), time.monotonic() - t0))
    return 0 if killed_n == len(report) and len(report) == len(MUTANTS) else 1


if __name__ == '__main__':
    raise SystemExit(main())
