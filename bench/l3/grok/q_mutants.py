"""16 source mutants M01-M16 (DESIGN section 8 risk list / GROK_IMPORT_SPEC "Q mutants"), one realistic bug each, applied to COPIES of the
.pruv files in work/mutants/<id>/ (the originals are never touched) and judged on the rs compiler + rust backend pair.

    GROK_TS_TRANSPILER=/path/to/typescript python3 bench/l3/grok/q_mutants.py [--only M03,M05] [--check] [--full] [--list]
        [--intended-only] [--jobs N]

Kill = a verifier rejection of the changed contract, or at least one fixture whose result / ACP transcript / primitive trace / final
tree differs from the pinned TypeScript beyond the policies (oracle.compare unexplained) on a normal run of the mutant runner.
Not kills: build errors, runner crashes, watchdog kills, mutants whose edit did not apply (`pattern-missing`). Fixtures that already
fail on the unmutated rs/rust runner are excluded (they would "kill" everything). Report: work/mutants/report.json.

How a mutation is located (variants are tried in order, the first whose every operation applies is used):
  marker   a source line ending in  `// MUT:M07 => <replacement code>`  is replaced (indent kept) by the replacement code. The module
           owner adds ONE such marker per mutant at the exact statement (see Q_README.md for the 16 intended edits); this is robust to
           refactors. It is searched in every copied .pruv file.
  inject   code inserted right after the opening brace of `fn NAME(` (DESIGN section 3 fixes these names)
  sub      regex substitution in the file (a guess against the unseen source; `--check` shows how often it matches)
"""
import sys
sys.dont_write_bytecode = True
import argparse
import json
import os
import re
import shutil
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from common import HERE, WORK, clean
import test_impl as T
from oracle import unexplained

MUT_DIR = WORK / 'mutants'
VERIFIER_WORDS = ('ensures', 'invariant', 'unproved', 'verification', 'timeout', 'requires', 'cannot prove', 'verifier')

# id, name, file (the file the edit lives in), what the bug is, variants (see the module docstring)
MUTANTS = [
    dict(id='M01', name='encode-folder-as-path', file='path.pruv',
         bug='the folder is used as a path (no percent-encoding), so "/a b" becomes nested directories',
         variants=[[('marker',)], [('inject', 'encode_folder', 'if true { return crate::util::to_bytes(folder); }')]]),
    dict(id='M02', name='follow-chain-link', file='path.pruv',
         bug='plainDir accepts a symlink (lstat kind 3) for sessions/folder/thread components',
         variants=[[('marker',)], [('sub', r'(\b\w+)\[7\]\s*==\s*2\b', r'(\1[7] == 2 || \1[7] == 3)', 'all')]]),
    dict(id='M03', name='ignore-open-identity', file='read.pruv',
         bug='the dev/ino comparison of the identity fstat is skipped, so a file replaced between lstat and open is read',
         variants=[[('marker',)], [('sub', r'\bif\s+[^{;]*\[9\][^{;]*!=[^{;]*\{', 'if false {', 1)]]),
    dict(id='M04', name='trust-summary-id-cwd', file='path.pruv',
         bug='the summary cwd/folderKey check is dropped (folder_same is always true), so a wrong cwd is trusted',
         variants=[[('marker',)], [('inject', 'folder_same', 'if true { return true; }')]]),
    dict(id='M05', name='cap-before-sort', file='list.pruv',
         bug='the scheduler stops examining candidates once 50 are done, so the 50-row cap is applied before the sort',
         variants=[[('marker',)], [('sub', r'\b(\w+)\[5\]\s*<\s*\1\[6\]', r'(\1[5] < \1[6] && \1[7] < 50)', 'all')]]),
    dict(id='M06', name='missing-date-is-oldest', file='list.pruv',
         bug='the sort comparator treats a missing/unparsable date as older than every parsed date instead of equal to everything',
         variants=[[('marker',)]]),
    dict(id='M07', name='active-id-truthy-fallback', file='summary.pruv',
         bug='an empty `id` falls through to sessionId/session_id (JS `||` instead of `??`)',
         variants=[[('marker',)]]),
    dict(id='M08', name='trim-query-interior', file='entry.pruv',
         bug='the unwrapped <user_query> text is trimmed (interior spaces / extra newlines lost) instead of removing exactly one LF',
         variants=[[('marker',)]]),
    dict(id='M09', name='synthetic-truthiness', file='entry.pruv',
         bug='synthetic_reason is tested for truthiness: false, 0 and "" no longer skip the user entry',
         variants=[[('marker',)]]),
    dict(id='M10', name='drop-without-detach', file='read.pruv',
         bug='a dropped oversized user line does not call skipUser: the next assistant attaches to the old prompt, the next prompt is not clipped',
         variants=[[('marker',)]]),
    dict(id='M11', name='tool-count-one', file='entry.pruv',
         bug='an assistant tool_calls array adds one tool instead of its length',
         variants=[[('marker',)], [('inject', 'add_tools', 'let n: i64 = 1;')]]),
    dict(id='M12', name='omit-budget-flag', file='read.pruv',
         bug='omittedBefore ignores the collector trimmed flag (only the window cut sets it)',
         variants=[[('marker',)]]),
    dict(id='M13', name='allow-all-files', file='copy.pruv',
         bug='the copy allow-list accepts every name (auth, lock, unknown files are copied and not counted as skipped)',
         variants=[[('marker',)], [('inject', 'allowed', 'if true { return true; }')]]),
    dict(id='M14', name='omit-second-exists', file='copy.pruv',
         bug='the second target lstat before rename is not acted on (a target that appeared meanwhile is renamed over)',
         variants=[[('marker',)]]),
    dict(id='M15', name='keep-copy-after-verify-error', file='copy.pruv',
         bug='after a failed verify the renamed target is not removed (the error is still grok-load-failed)',
         variants=[[('marker',)]]),
    dict(id='M16', name='ignore-load-capability', file='rpc.pruv',
         bug='initialize.agentCapabilities.loadSession is not required to be strictly true: session/load is sent anyway',
         variants=[[('marker',)]]),
]
MARK = re.compile(r'^(?P<indent>[ \t]*)(?P<code>.*?)[ \t]*//[ \t]*MUT:(?P<id>M\d\d)[ \t]*=>[ \t]*(?P<alt>.*?)[ \t]*$')


# ---- applying a mutation ----------------------------------------------------------------------------------------------
class NoMatch(Exception):
    pass


def fn_body_open(src, name):
    """Index just after the `{` that opens the body of `fn NAME(...)` (None if there is no such function)."""
    m = re.search(r'\bfn\s+' + re.escape(name) + r'\s*\(', src)
    if not m:
        return None
    depth, i = 1, m.end()
    while i < len(src) and depth:
        depth += {'(': 1, ')': -1}.get(src[i], 0)
        i += 1
    j = src.find('{', i)
    return None if j < 0 else j + 1


def apply_variant(files, mid, variant, home):
    """files: {name: text} (modified in place). Returns a description of what was changed; raises NoMatch."""
    notes = []
    for op in variant:
        kind = op[0]
        if kind == 'marker':
            hits = 0
            for name, text in files.items():
                out = []
                for line in text.split('\n'):
                    m = MARK.match(line)
                    if m and m.group('id') == mid:
                        out.append(m.group('indent') + m.group('alt'))
                        hits += 1
                    else:
                        out.append(line)
                files[name] = '\n'.join(out)
            if hits != 1:
                raise NoMatch('marker `// MUT:%s => ...` found %d times (need exactly 1)' % (mid, hits))
            notes.append('marker')
        elif kind == 'inject':
            _, fn, code = op
            for name in home:
                at = fn_body_open(files[name], fn)
                if at is not None:
                    files[name] = files[name][:at] + '\n    ' + code + files[name][at:]
                    notes.append('inject into %s::%s' % (name, fn))
                    break
            else:
                raise NoMatch('fn %s not found' % fn)
        elif kind == 'sub':
            _, rx, repl, count = op
            n_total, done = 0, False
            for name in home:
                new, n = re.subn(rx, repl, files[name])
                if n:
                    if count != 'all' and n != count:
                        raise NoMatch('regex %r matches %d times in %s (need %s)' % (rx, n, name, count))
                    files[name], n_total, done = new, n, True
                    notes.append('sub x%d in %s' % (n, name))
                    break
            if not done:
                raise NoMatch('regex %r does not match in %s' % (rx, ','.join(home)))
        else:
            raise ValueError(kind)
    return '; '.join(notes)


def mutate(mutant, src_dir, out_dir):
    """Copy the .pruv files and the manifest to out_dir and apply the first variant that fits. Returns (variant index, note) or raises NoMatch."""
    clean(out_dir)
    out_dir.mkdir(parents=True)
    names = sorted(p.name for p in Path(src_dir).glob('*.pruv'))
    texts = {n: (Path(src_dir) / n).read_text(encoding='utf-8') for n in names}
    manifest = json.loads((Path(src_dir) / 'pruv.json').read_text())
    for dep in manifest.get('dependencies', {}).values():       # paths are relative to the manifest: point them at the real packages
        dep['path'] = os.path.relpath((Path(src_dir) / dep['path']).resolve(), out_dir.resolve())
    (out_dir / 'pruv.json').write_text(json.dumps(manifest) + '\n')
    home = [mutant['file']] + [n for n in names if n != mutant['file']]
    home = [n for n in home if n in texts]
    errors = []
    for k, variant in enumerate(mutant['variants']):
        files = dict(texts)
        try:
            note = apply_variant(files, mutant['id'], variant, home)
        except NoMatch as exc:
            errors.append('variant %d: %s' % (k, exc))
            continue
        for n in names:
            (out_dir / n).write_text(files[n], encoding='utf-8')
        changed = [n for n in names if files[n] != texts[n]]
        assert changed, 'a mutation that changes nothing'
        return k, '%s (%s)' % (note, ','.join(changed))
    raise NoMatch(' | '.join(errors))


# ---- judging ----------------------------------------------------------------------------------------------------------
def kills_of(case, ts, base, impl):
    """'kill' | 'crash' | 'same' for one fixture on a mutant runner."""
    if not T.impl_clean(impl):
        return 'crash'
    v = T.judge(case, ts, base, impl)
    return 'same' if v['ok'] else 'kill'


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--only', help='comma list of mutant ids (M01,...)')
    ap.add_argument('--check', action='store_true', help='only report which variant matches; build and run nothing')
    ap.add_argument('--list', action='store_true', help='list the mutants and their mapped fixtures')
    ap.add_argument('--full', action='store_true', help='run every fixture for every mutant (default: stop at the first kill)')
    ap.add_argument('--intended-only', action='store_true', help='only the fixtures mapped to the mutant (`kills` in fixtures/*.json)')
    ap.add_argument('--jobs', type=int, default=4)
    ap.add_argument('--ts-transpiler')
    args = ap.parse_args(argv)
    cases, _ = T.load_fixtures()
    by_name = {c['name']: c for c in cases}
    intended = {m['id']: [c['name'] for c in cases if m['id'] in c.get('kills', [])] for m in MUTANTS}
    chosen = [m for m in MUTANTS if not args.only or m['id'] in args.only.split(',')]
    if args.list:
        for m in MUTANTS:
            print('%s %-30s %-12s %d fixtures: %s' % (m['id'], m['name'], m['file'], len(intended[m['id']]), ', '.join(intended[m['id']][:4])))
        return 1 if any(not intended[m['id']] for m in MUTANTS) else 0
    unmapped = [m['id'] for m in MUTANTS if not intended[m['id']]]
    if unmapped:
        print('mutants without a fixed fixture: %s' % ', '.join(unmapped), file=sys.stderr)
    MUT_DIR.mkdir(parents=True, exist_ok=True)
    report = []
    applied = {}
    for m in chosen:
        row = {'id': m['id'], 'name': m['name'], 'file': m['file'], 'bug': m['bug'], 'fixtures': intended[m['id']]}
        try:
            k, note = mutate(m, HERE, MUT_DIR / m['id'])
            row.update(variant=k, edit=note)
            applied[m['id']] = True
        except NoMatch as exc:
            row.update(result='pattern-missing', detail=str(exc))
            applied[m['id']] = False
        report.append(row)
        if args.check:
            print('%s %-30s %s' % (m['id'], m['name'], ('variant %d: %s' % (row['variant'], row['edit'])) if applied[m['id']] else 'PATTERN MISSING: ' + row['detail']))
    if args.check:
        (MUT_DIR / 'check.json').write_text(json.dumps(report, indent=1))
        return 0 if all(applied.values()) else 1
    if args.ts_transpiler:
        os.environ['GROK_TS_TRANSPILER'] = args.ts_transpiler
    if not os.environ.get('GROK_TS_TRANSPILER'):
        print('GROK_TS_TRANSPILER (or --ts-transpiler) must name the typescript package', file=sys.stderr)
        return 2
    c, g, run = T.configure()
    host = T.host_artifacts()
    baseline_exe = WORK / 'rs_rust' / 'runner'
    if not baseline_exe.exists():
        print('no work/rs_rust/runner: run test_impl.py first', file=sys.stderr)
        return 2
    t0 = time.monotonic()
    with ThreadPoolExecutor(args.jobs) as ex:
        refs = dict(zip([c['name'] for c in cases], ex.map(T.run_reference, cases)))
        base_runs = dict(zip([c['name'] for c in cases], ex.map(lambda c: T.run_pair(c, 'mut_baseline', baseline_exe), cases)))
    baseline_bad = {n for n, impl in base_runs.items() if not T.judge(by_name[n], refs[n][0], refs[n][1], impl)['ok'] or T.check_expected(by_name[n], refs[n][0])}
    for n in by_name:
        for side in (refs[n][0], base_runs[n]):
            if n not in baseline_bad and side.get('root'):
                clean(side['root'])
    print('reference + baseline: %d fixtures, %d already failing on the unmutated rs/rust runner (excluded), %.0f s' % (len(cases), len(baseline_bad), time.monotonic() - t0), flush=True)
    usable = [c for c in cases if c['name'] not in baseline_bad]
    for row in report:
        if row.get('result') == 'pattern-missing':
            print('%s pattern-missing' % row['id'], flush=True)
            continue
        m = next(x for x in MUTANTS if x['id'] == row['id'])
        w = MUT_DIR / m['id']
        built = g.compile_pruv('rs', str(w / 'root.pruv'), str(w), 'rust', T.compile_flags(host[3], w / 'pruv.json'))
        (w / 'compile.json').write_text(json.dumps(built, default=str, indent=1))
        if built['rc']:
            detail = str(built.get('errors') or built.get('stderr'))
            row.update(result='verifier-rejected' if any(x in detail.lower() for x in VERIFIER_WORDS) else 'build-error', detail=detail[-2000:])
            print('%s %s' % (m['id'], row['result']), flush=True)
            continue
        exe = T.link_runner(g, run, built, w, host, 'rust')
        order = [by_name[n] for n in intended[m['id']] if n in by_name and n not in baseline_bad]
        if not args.intended_only:
            seen = {c['name'] for c in order}
            order += [c for c in usable if c['name'] not in seen]
        killed, crashes, same = [], [], 0
        for case in order:
            impl = T.run_pair(case, 'mut_' + m['id'], exe)
            verdict = kills_of(case, refs[case['name']][0], refs[case['name']][1], impl)
            if verdict == 'kill':
                killed.append(case['name'])
                if not args.full:
                    break
            elif verdict == 'crash':
                crashes.append(case['name'])
            else:
                same += 1
            if verdict != 'kill' and impl.get('root'):
                clean(impl['root'])
        mapped_killed = [n for n in killed if n in intended[m['id']]]
        row.update(result='differential-killed' if killed else 'survived', killed_by=killed[:20], killed_by_intended=bool(mapped_killed),
                   crashes=crashes[:20], unchanged_fixtures=same)
        if not killed:
            row['note'] = 'no fixed fixture distinguishes it (crashes are not kills); not a claim of equivalence'
        print('%s %-30s %s%s' % (m['id'], m['name'], row['result'], (' by ' + ', '.join(killed[:3])) if killed else ''), flush=True)
        (MUT_DIR / 'report.json').write_text(json.dumps(report, indent=1))
    (MUT_DIR / 'report.json').write_text(json.dumps(report, indent=1))
    ok = sum(1 for r in report if r.get('result') in ('differential-killed', 'verifier-rejected'))
    print('killed %d/%d (differential %d, verifier %d); survived %d; build errors %d; pattern missing %d' % (
        ok, len(report), sum(r.get('result') == 'differential-killed' for r in report), sum(r.get('result') == 'verifier-rejected' for r in report),
        sum(r.get('result') == 'survived' for r in report), sum(r.get('result') == 'build-error' for r in report), sum(r.get('result') == 'pattern-missing' for r in report)))
    return 0 if ok == len(report) and not unmapped else 1


if __name__ == '__main__':
    raise SystemExit(main())
