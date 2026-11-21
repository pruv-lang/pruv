"""Generates fixtures/*.json (the 150 named cases of spec section 7 plus the five wC audit cases S11 C11 C12 H11 V11) and their expect blocks.

Every non-probe expectation is derived by running the pinned-TypeScript oracle (oracle.py) over the case, then each case's
check function asserts the observation the spec names (so a fixture really exercises that behavior, not just any result).
Probe cases (P06-P11) record the expected boundary result of the new packet interface. Exact-limit probes are also
oracle-compared. The I group (integration) carries route/op and an assertion description for integrate.py.

  python3 gen_fixtures.py            # rewrite fixtures/*.json
  python3 gen_fixtures.py --check    # derive again and fail when a fixture's expect differs (no file written)

Synthetic data only.
"""
import sys
sys.dont_write_bytecode = True
import json

from common import FIXTURES, expand_case, jv_string, jv_u16, load_defaults, merge_case, probe_expect, strip_case
from oracle import run_oracle

A1 = 'acc_0123456789abcdef'
A2 = 'acc_fedcba9876543210'
DD = '/data'
CL = '/data/engine-homes/claude/'
CX = '/data/engine-homes/codex/'
HOST_EXEC = 'wireEngine.account.folder.codexHostExecution'
INVALID_ID = 'wireEngine.account.invalidId'
PLAIN_MSG = 'Codex 계정 폴더를 확인하지 못해 쓰지 않았어.'
CODEX_ACC = CX + A1  # the expected folder of account A1
CODES = ['not-found', 'codex-unsupported', 'grok-exists', 'grok-load-failed', 'timeout', 'read-failed', 'engine-missing']
GROUPS = ['S', 'E', 'A', 'C', 'H', 'F', 'D', 'T', 'R', 'U', 'V', 'X', 'B', 'P', 'I']
MUTANT_KILLERS = {
    'E09': 'M01', 'S08': 'M02', 'E02': 'M03', 'A07': 'M04', 'A08': 'M05', 'C04': 'M06', 'H03': 'M07', 'F05': 'M08',
    'D06': 'M09', 'T07': 'M10', 'R04': 'M11', 'R09': 'M12', 'U09': 'M13', 'V03': 'M14', 'X06': 'M15',
    'X01': 'M15', 'B03': 'M16', 'P11': 'M17',
}

ENTRIES = {g: [] for g in GROUPS}  # group -> list of fixture entries (without expect), with 'checks' kept aside
CHECKS = {}  # sub id -> check function
FAILS = []
PROBES = {}  # sub id -> probe transform info


def add(cid, name, op, subs=None, args=None, script=None, check=None, **extra):
    """One fixture entry. Without subs the entry is itself the case (check applies to it); with subs each sub is
    (suffix, {args:..,script:..}, check) and gets the id '<cid>/<suffix>'."""
    entry = {'id': cid, 'name': name, 'op': op}
    if args is not None:
        entry['args'] = args
    if script is not None:
        entry['script'] = script
    entry.update(extra)
    if subs is None:
        CHECKS[cid] = check
    else:
        entry.pop('id')
        entry['id'] = cid
        entry['subs'] = []
        for suffix, over, sub_check in subs:
            sid = f'{cid}/{suffix}'
            sub = dict(over)
            sub['id'] = sid
            entry['subs'].append(sub)
            CHECKS[sid] = sub_check
    ENTRIES[cid[0]].append(entry)


# ---- small builders / assertions ----
def sess(sid='s', **fields):
    out = {'id': sid}
    out.update(fields)
    return out


def row(ref, title, **extra):
    out = {'ref': ref, 'title': title}
    out.update(extra)
    return out


def eng(rows=None, record=None, reject=None, sync_throw=None):
    if rows is not None:
        return {'rows': rows}
    if record is not None:
        return {'record': record}
    if reject is not None:
        return {'reject': reject}
    return {'sync_throw': sync_throw}


def ok_rows(n=2):
    return eng(rows=[row(f'r{i + 1}', f't{i + 1}') for i in range(n)])


def acc(ident, name=None):
    return {'id': ident, 'name': name if name is not None else 'n'}


def err(ref, cls='Error', **kw):
    out = {'class': cls, 'ref': ref}
    out.update(kw)
    return out


def need(cond, what):
    if not cond:
        raise AssertionError(what)


def term(r, t):
    need(r['terminal'] == t, f"terminal {r['terminal']} != {t}: {json.dumps(r['value'], ensure_ascii=False)[:200]}")


def homes(r):
    term(r, 'ok')
    return {v['source']: v['home'] for v in r['value']}


def sources(r):
    term(r, 'ok')
    return [v['source'] for v in r['value']]


def kinds(trace):
    return [t[0] for t in trace]


def imp(env=None, ambient=None, **script):
    out = {'args': {'env': env or {}}}
    scr = dict(script)
    if ambient is not None:
        scr['ambient'] = [{'ok': ambient}]
    if scr:
        out['script'] = scr
    return out


HOME_DEFAULTS = {'claude:home': '/home/test/.claude', 'codex:home': '/home/test/.codex', 'grok:home': '/home/test/.grok',
                 'opencode:home': '/home/test/.local/share/opencode', 'qwen:home': '/home/test/.qwen'}


def home_is(source, expected):
    def check(r):
        h = homes(r)
        need(h[source] == expected, f'{source} home {h[source]!r} != {expected!r}')
        return None
    return check


def codex_args(home=None, data_dir=DD, ident=A1):
    return {'dataDir': data_dir, 'account': {'id': ident, 'home': home if home is not None else CX + ident}}


def fsr(**kw):
    """realpath/stat script helper: {path: [entries]}."""
    return kw


def ok_s(path):
    return {'ok': path}


def errno(code, ref=None):
    out = {'errno': code}
    if ref:
        out['ref'] = ref
    return out


def st(dev, ino):
    return {'ok': {'dev': str(dev), 'ino': str(ino)}}


def is_wire_host(r):
    term(r, 'throw_account_wire')
    need(r['value']['key'] == HOST_EXEC, 'wire key')


# =========================================================================================================== S
add('S01', 'homes-default-order', 'import_sources', **imp(), check=lambda r: (
    need(sources(r) == list(HOME_DEFAULTS), 'order'),
    need(homes(r) == HOME_DEFAULTS, 'homes'),
    need([v['engine'] for v in r['value']] == ['claude', 'codex', 'grok', 'opencode', 'qwen'], 'engines')))
add('S02', 'claude-trimmed-override', 'import_sources', subs=[
    ('ascii', imp(env={'CLAUDE_CONFIG_DIR': ' \t/cl/home\n '}), home_is('claude:home', '/cl/home')),
    ('unicode', imp(env={'CLAUDE_CONFIG_DIR': ' ﻿/cl/home '}), home_is('claude:home', '/cl/home'))])
add('S03', 'claude-empty-fallback', 'import_sources', **imp(env={'CLAUDE_CONFIG_DIR': ''}),
    check=home_is('claude:home', '/home/test/.claude'))
add('S04', 'claude-whitespace-fallback', 'import_sources', **imp(env={'CLAUDE_CONFIG_DIR': ' \t\n  '}),
    check=home_is('claude:home', '/home/test/.claude'))
add('S05', 'claude-relative-retained', 'import_sources', **imp(env={'CLAUDE_CONFIG_DIR': 'rel/claude'}),
    check=home_is('claude:home', 'rel/claude'))
add('S06', 'grok-trimmed-override', 'import_sources', **imp(env={'GROK_HOME': '  /g/home\t'}),
    check=home_is('grok:home', '/g/home'))
add('S07', 'grok-empty-fallback', 'import_sources', subs=[
    ('empty', imp(env={'GROK_HOME': ''}), home_is('grok:home', '/home/test/.grok')),
    ('blank', imp(env={'GROK_HOME': '   '}), home_is('grok:home', '/home/test/.grok'))])
add('S08', 'codex-ambient-wins', 'import_sources', **imp(env={'CODEX_HOME': '/deps'}, ambient='/amb'),
    check=lambda r: (home_is('codex:home', '/amb')(r), need(['ambient', 'CODEX_HOME'] in r['trace'], 'ambient request')))
add('S09', 'codex-ambient-blank', 'import_sources', **imp(env={'CODEX_HOME': '/deps'}, ambient='   '),
    check=home_is('codex:home', '/home/test/.codex'))
add('S10', 'deps-home-not-homedir', 'import_sources', **imp(env={'HOME': '/other/home'}),
    check=lambda r: (need(homes(r) == HOME_DEFAULTS, 'homes follow homedir, not deps.env.HOME'),
                     need(all('/other' not in h for h in homes(r).values()), 'no /other')))

# =========================================================================================================== E
add('E01', 'xdg-absolute-join', 'import_sources', **imp(env={'XDG_DATA_HOME': '/xdg/share'}),
    check=home_is('opencode:home', '/xdg/share/opencode'))
add('E02', 'xdg-relative-fallback', 'import_sources', **imp(env={'XDG_DATA_HOME': 'xdg/rel'}),
    check=home_is('opencode:home', '/home/test/.local/share/opencode'))
add('E03', 'xdg-blank-fallback', 'import_sources', subs=[
    ('empty', imp(env={'XDG_DATA_HOME': ''}), home_is('opencode:home', '/home/test/.local/share/opencode')),
    ('trimmed-empty', imp(env={'XDG_DATA_HOME': ' \t '}), home_is('opencode:home', '/home/test/.local/share/opencode'))])
add('E04', 'xdg-root-and-dot-normalization', 'import_sources', subs=[
    ('root', imp(env={'XDG_DATA_HOME': '/'}), home_is('opencode:home', '/opencode')),
    ('dots', imp(env={'XDG_DATA_HOME': '/a/./b/../c'}), home_is('opencode:home', '/a/c/opencode')),
    ('double-slash', imp(env={'XDG_DATA_HOME': '/a//b/'}), home_is('opencode:home', '/a/b/opencode'))])
add('E05', 'qwen-runtime-precedence', 'import_sources', **imp(env={'QWEN_RUNTIME_DIR': '/rt', 'QWEN_HOME': '/glob'}),
    check=home_is('qwen:home', '/rt'))
add('E06', 'qwen-runtime-blank-global', 'import_sources', **imp(env={'QWEN_RUNTIME_DIR': '  ', 'QWEN_HOME': ' /glob '}),
    check=home_is('qwen:home', '/glob'))
add('E07', 'qwen-both-blank-default', 'import_sources', **imp(env={'QWEN_RUNTIME_DIR': ' ', 'QWEN_HOME': '\t'}),
    check=home_is('qwen:home', '/home/test/.qwen'))
add('E08', 'qwen-relative-verbatim', 'import_sources', **imp(env={'QWEN_RUNTIME_DIR': 'rel/./rt/../x'}),
    check=home_is('qwen:home', 'rel/./rt/../x'))
add('E09', 'trim-unicode-membership', 'import_sources', subs=[
    ('trimmed', imp(env={'CLAUDE_CONFIG_DIR': '﻿ /c ﻿'}), home_is('claude:home', '/c')),
    ('nel-retained', imp(env={'CLAUDE_CONFIG_DIR': '\u0085/c'}), home_is('claude:home', '\u0085/c')),
    ('zwsp-retained', imp(env={'CLAUDE_CONFIG_DIR': '/c​'}), home_is('claude:home', '/c​')),
    ('all-trimmed', imp(env={'GROK_HOME': '       　\u000b\u000c/g'}),
     home_is('grok:home', '/g'))])
add('E10', 'qwen-eager-home-observation', 'import_sources', subs=[
    ('override', imp(env={'CLAUDE_CONFIG_DIR': '/c', 'GROK_HOME': '/g', 'XDG_DATA_HOME': '/x', 'QWEN_RUNTIME_DIR': '/rt'},
                    ambient='/amb'),
     lambda r: (home_is('qwen:home', '/rt')(r),
                need(r['trace'].count(['homedir']) == 1, 'exactly one homedir observation (qwen eager)'),
                need(r['trace'][-1] == ['homedir'], 'it is the last request'))),
    ('throw', imp(env={'CLAUDE_CONFIG_DIR': '/c', 'GROK_HOME': '/g', 'XDG_DATA_HOME': '/x', 'QWEN_RUNTIME_DIR': '/rt'},
                  ambient='/amb', homedir=[{'throw': err('hd_err', message='boom')}]),
     lambda r: (term(r, 'throw_existing'), need(r['value'] == {'ref': 'hd_err'}, 'homedir failure escapes')))])

# =========================================================================================================== A
def acc_script(claude=None, codex=None):
    out = {}
    if claude is not None:
        out['claude'] = claude if 'throw' in claude else {'ok': claude['ok']}
    if codex is not None:
        out['codex'] = codex if 'throw' in codex else {'ok': codex['ok']}
    return {'accounts': out}


def acct_rows(r, kind='claude'):
    term(r, 'ok')
    return [v for v in r['value'] if v['engine'] == kind and v['kind'] == 'account']


add('A01', 'account-valid-lower-hex', 'import_sources', args={'env': {}}, script=acc_script({'ok': [acc(A1, 'One')]}, {'ok': [acc(A2, 'Two')]}),
    check=lambda r: (need(sources(r) == ['claude:home', f'claude:{A1}', 'codex:home', f'codex:{A2}', 'grok:home', 'opencode:home', 'qwen:home'], 'sources'),
                     need(acct_rows(r)[0]['home'] == CL + A1, 'claude home'),
                     need(acct_rows(r, 'codex')[0]['home'] == CX + A2, 'codex home'),
                     need(acct_rows(r)[0]['account'] == A1 and acct_rows(r)[0]['accountName'] == 'One', 'fields')))
UPPER = 'acc_0123456789ABCDEF'
add('A02', 'account-uppercase-rejected', 'import_sources', subs=[
    ('import', {'args': {'env': {}}, 'script': acc_script({'ok': [acc(UPPER), acc(A1)]}, {'ok': [acc(UPPER)]})},
     lambda r: (need([v['account'] for v in acct_rows(r)] == [A1], 'upper skipped'), need(acct_rows(r, 'codex') == [], 'codex upper skipped'))),
    ('home', {'op': 'account_home', 'args': {'engine': 'claude', 'id': UPPER}},
     lambda r: (term(r, 'throw_account_wire'), need(r['value']['key'] == INVALID_ID, 'key'))),
    ('mixed-case', {'op': 'account_home', 'args': {'engine': 'codex', 'id': 'acc_0123456789abcdeF'}},
     lambda r: term(r, 'throw_account_wire'))])
H15, H16, H17 = 'acc_0123456789abcde', A1, 'acc_0123456789abcdef0'
add('A03', 'account-length-boundaries', 'import_sources', subs=[
    ('import', {'args': {'env': {}}, 'script': acc_script({'ok': [acc(H15), acc(H16), acc(H17)]})},
     lambda r: need([v['account'] for v in acct_rows(r)] == [H16], 'only 16 accepted')),
    ('home15', {'op': 'account_home', 'args': {'engine': 'claude', 'id': H15}}, lambda r: term(r, 'throw_account_wire')),
    ('home17', {'op': 'account_home', 'args': {'engine': 'claude', 'id': H17}}, lambda r: term(r, 'throw_account_wire')),
    ('home16', {'op': 'account_home', 'args': {'engine': 'claude', 'id': H16}},
     lambda r: (term(r, 'ok'), need(r['value'] == CL + A1, 'joined')))])
BAD4 = [('newline', A1 + '\n'), ('slash', 'acc_0123456789/abcdef'), ('traversal', '../' + A1), ('embedded-dotdot', 'acc_../../etc/passw'),
        ('prefix-slash', '/' + A1), ('leading-newline', '\n' + A1)]
add('A04', 'account-newline-and-path-rejected', 'account_home',
    subs=[(n, {'args': {'engine': 'claude', 'id': i}}, lambda r: (term(r, 'throw_account_wire'), need(r['value']['key'] == INVALID_ID, 'key')))
          for n, i in BAD4] + [('import', {'op': 'import_sources', 'args': {'env': {}},
                                          'script': acc_script({'ok': [acc(i) for _, i in BAD4]}, {'ok': [acc(i) for _, i in BAD4]})},
                                lambda r: (need(acct_rows(r) == [] and acct_rows(r, 'codex') == [], 'all skipped'),
                                           need(sources(r) == list(HOME_DEFAULTS), 'homes only')))])
add('A05', 'account-name-verbatim', 'import_sources', args={'env': {}},
    script=acc_script({'ok': [acc(A1, ''), acc(A2, '  pad \n')]}, {'ok': [acc(A1, '')]}),
    check=lambda r: (need([v['accountName'] for v in acct_rows(r)] == ['', '  pad \n'], 'claude names'),
                     need([v['accountName'] for v in acct_rows(r, 'codex')] == [''], 'codex name')))
add('A06', 'account-callback-order', 'import_sources', args={'env': {}}, script=acc_script({'ok': [acc(A1)]}, {'ok': [acc(A1)]}),
    check=lambda r: (need(['accounts', 'claude'] in r['trace'] and ['accounts', 'codex'] in r['trace'], 'both requested'),
                     need(r['trace'].index(['accounts', 'claude']) < r['trace'].index(['accounts', 'codex']), 'claude first'),
                     need(r['trace'].count(['accounts', 'claude']) == 1 and r['trace'].count(['accounts', 'codex']) == 1, 'once each')))
add('A07', 'account-array-order-duplicates', 'import_sources', args={'env': {}},
    script=acc_script({'ok': [acc(A1, 'a'), acc(A2, 'b'), acc(A1, 'c')]}, {'ok': [acc(A2, 'x'), acc(A2, 'y')]}),
    check=lambda r: (need([(v['account'], v['accountName']) for v in acct_rows(r)] == [(A1, 'a'), (A2, 'b'), (A1, 'c')], 'claude order/dups'),
                     need([(v['account'], v['accountName']) for v in acct_rows(r, 'codex')] == [(A2, 'x'), (A2, 'y')], 'codex dups')))
add('A08', 'account-bad-middle-skipped', 'import_sources', args={'env': {}},
    script=acc_script({'ok': [acc(A1), acc('not-valid'), acc(A2)]}, {'ok': [acc(A1), acc('BAD'), acc(A2)]}),
    check=lambda r: (need([v['account'] for v in acct_rows(r)] == [A1, A2], 'claude later retained'),
                     need([v['account'] for v in acct_rows(r, 'codex')] == [A1, A2], 'codex later retained'),
                     term(r, 'ok')))
add('A09', 'account-callback-throw-escapes', 'import_sources', subs=[
    ('claude', {'args': {'env': {}}, 'script': acc_script({'throw': err('cb_err', message='cb failed')}, {'ok': [acc(A1)]})},
     lambda r: (term(r, 'throw_existing'), need(r['value'] == {'ref': 'cb_err'}, 'ref'),
                need(['accounts', 'codex'] not in r['trace'], 'codex callback never ran'))),
    ('codex', {'args': {'env': {}}, 'script': acc_script({'ok': [acc(A1)]}, {'throw': err('cb_err2', message='cb2 failed')})},
     lambda r: (term(r, 'throw_existing'), need(r['value'] == {'ref': 'cb_err2'}, 'ref'),
                need(['accounts', 'claude'] in r['trace'], 'claude ran first'))),
    ('after-rows', {'args': {'env': {}}, 'script': acc_script({'ok': [acc(A1)]}, {'throw': err('cb_err3', cls='TypeError', message='t')})},
     lambda r: need(r['terminal'] != 'ok', 'no partial output'))])
NINE = [acc(f'acc_{i:016x}', f'n{i}') for i in range(1, 10)]
add('A10', 'account-no-eight-cap', 'import_sources', args={'env': {}}, script=acc_script({'ok': NINE}, {'ok': NINE}),
    check=lambda r: (need(len(acct_rows(r)) == 9 and len(acct_rows(r, 'codex')) == 9, 'nine each'),
                     need([v['account'] for v in acct_rows(r)] == [a['id'] for a in NINE], 'order')))

# R7: homedir results that are {"u16":[...]} (a lone surrogate, or a valid pair spelled as units) are decoded by the oracle.
def u16_text(units):
    """The expected rendering of a unit string: a JSON string when well-formed UTF-16, otherwise {"u16":[...]}."""
    try:
        return b''.join(u.to_bytes(2, 'little') for u in units).decode('utf-16-le')
    except UnicodeDecodeError:
        return jv_u16(units)


HD_PAIR = [0x2F, 0x68, 0xD83D, 0xDE00]
HD_LONE = [0x2F, 0x68, 0xD800]
HD_LOW = [0x2F, 0xDC00, 0x68]


def home_u16(units):
    return lambda r: (need(homes(r)['claude:home'] == u16_text(units + [0x2F, 0x2E, 0x63, 0x6C, 0x61, 0x75, 0x64, 0x65]),
                           'claude home follows the decoded homedir'),
                      need(['homedir'] in r['trace'], 'homedir requested'))


add('S11', 'homedir-u16-result', 'import_sources', subs=[
    ('pair', imp(homedir=[{'ok': jv_u16(HD_PAIR)}]), home_u16(HD_PAIR)),
    ('lone-high', imp(homedir=[{'ok': jv_u16(HD_LONE)}]), home_u16(HD_LONE)),
    ('lone-low', imp(homedir=[{'ok': jv_u16(HD_LOW)}]), home_u16(HD_LOW))])

# =========================================================================================================== C
def cf(realpath=None, stat=None, ambient=None, args=None, cwd=None, **extra):
    out = {'op': 'codex_account_folder', 'args': args or codex_args()}
    scr = {}
    if realpath:
        scr['realpath'] = realpath
    if stat:
        scr['stat'] = stat
    if ambient is not None:
        scr['ambient'] = [{'ok': ambient}]
    if cwd is not None:
        scr['cwd'] = [{'ok': cwd}]
    scr.update(extra)
    if scr:
        out['script'] = scr
    return out


def rp(r):
    return [t[1] for t in r['trace'] if t[0] == 'realpath']


def stp(r):
    return [t[1] for t in r['trace'] if t[0] == 'stat']


add('C01', 'canonical-existing-realpath', 'codex_account_folder',
    **{k: v for k, v in cf(realpath={CODEX_ACC: [ok_s('/real/shared')], '/amb': [ok_s('/real/shared')]}, ambient='/amb').items() if k != 'op'},
    check=lambda r: (is_wire_host(r), need(r['trace'][0] == ['realpath', CODEX_ACC], 'target realpath first'),
                     need(rp(r)[:2] == [CODEX_ACC, '/amb'], 'no climbing after a successful realpath')))
add('C02', 'canonical-one-missing-leaf', 'codex_account_folder',
    **{k: v for k, v in cf(realpath={CX.rstrip('/'): [ok_s('/real/codex')], '/amb': [ok_s('/real/codex/' + A1)]}, ambient='/amb').items() if k != 'op'},
    check=lambda r: (is_wire_host(r), need(rp(r)[:3] == [CODEX_ACC, CX.rstrip('/'), '/amb'], 'leaf then parent then host')))
add('C03', 'canonical-multiple-missing-components', 'codex_account_folder',
    **{k: v for k, v in cf(realpath={'/data': [ok_s('/real/data')], '/amb': [ok_s('/real/data/engine-homes/codex/' + A1)]}, ambient='/amb').items() if k != 'op'},
    check=lambda r: (is_wire_host(r), need(rp(r)[:5] == [CODEX_ACC, CX.rstrip('/'), '/data/engine-homes', '/data', '/amb'], 'climb order')))
add('C04', 'canonical-enoent-skips-stat', 'codex_account_folder',
    **{k: v for k, v in cf(ambient='/amb', realpath={'/data': [ok_s('/real/data')]}).items() if k != 'op'},
    check=lambda r: (term(r, 'ok'),
                     need(CX.rstrip('/') in rp(r) and '/data/engine-homes' in rp(r), 'target climbed through ENOENT components'),
                     need(set(stp(r)) == {CODEX_ACC, '/amb', '/home/test/.codex'} or set(stp(r)) <= {CODEX_ACC}, 'only sameFile stats'),
                     need(all(p == CODEX_ACC for p in stp(r)[:1]), 'first stat is the sameFile left'),
                     need(CX.rstrip('/') not in stp(r) and '/data/engine-homes' not in stp(r) and '/data' not in stp(r), 'no stat at ENOENT components')))
add('C05', 'canonical-eacces-stat-identity', 'codex_account_folder',
    **{k: v for k, v in cf(realpath={CODEX_ACC: [errno('EACCES')], '/amb': [errno('EACCES')]},
                           stat={CODEX_ACC: [st(11, 22)], '/amb': [st(11, 22)]}, ambient='/amb').items() if k != 'op'},
    check=lambda r: (is_wire_host(r), need(stp(r)[:2] == [CODEX_ACC, '/amb'], 'identity stats'),
                     need(kinds(r['trace']).index('stat') > kinds(r['trace']).index('realpath'), 'stat after realpath')))
add('C06', 'canonical-eloop-stat-identity', 'codex_account_folder',
    **{k: v for k, v in cf(realpath={CODEX_ACC: [errno('ELOOP')], '/amb': [errno('ELOOP')]},
                           stat={CODEX_ACC: [st(5, 6)], '/amb': [st(5, 6)]}, ambient='/amb').items() if k != 'op'},
    check=lambda r: (is_wire_host(r), need(stp(r)[:2] == [CODEX_ACC, '/amb'], 'identity stats via ELOOP')))
add('C07', 'canonical-stat-failure-climbs', 'codex_account_folder',
    **{k: v for k, v in cf(realpath={CODEX_ACC: [errno('EACCES')], CX.rstrip('/'): [ok_s('/real/codex')], '/amb': [ok_s('/real/codex/' + A1)]},
                           stat={CODEX_ACC: [errno('EIO')]}, ambient='/amb').items() if k != 'op'},
    check=lambda r: (is_wire_host(r), need(r['trace'][:3] == [['realpath', CODEX_ACC], ['stat', CODEX_ACC], ['realpath', CX.rstrip('/')]], 'climbs after failed stat')))
add('C08', 'canonical-root-exhaustion', 'codex_account_folder', subs=[
    ('exact', cf(ambient=CODEX_ACC), lambda r: (is_wire_host(r), need('/' in rp(r), 'climbed to the root'))),
    ('normalized', cf(ambient='/data/x/../engine-homes/./codex/' + A1), lambda r: is_wire_host(r))])
add('C09', 'canonical-relative-cwd', 'codex_account_folder',
    subs=[('relative', cf(args=codex_args(data_dir='rel/data', home='rel/data/engine-homes/codex/' + A1), cwd='/cwdx',
                          ambient='/cwdx/rel/data/engine-homes/codex/' + A1),
           lambda r: (is_wire_host(r), need(['cwd'] in r['trace'], 'cwd observed'),
                      need(rp(r)[0] == '/cwdx/rel/data/engine-homes/codex/' + A1, 'resolved against cwd'))),
          ('other-cwd', cf(args=codex_args(data_dir='rel/data', home='rel/data/engine-homes/codex/' + A1), cwd='/elsewhere',
                           ambient='/cwdx/rel/data/engine-homes/codex/' + A1),
           lambda r: (term(r, 'ok'), need(rp(r)[0] == '/elsewhere/rel/data/engine-homes/codex/' + A1, 'cwd changes the path')))])
add('C10', 'canonical-identity-suffix', 'codex_account_folder', subs=[
    ('equal', cf(realpath={CX.rstrip('/'): [errno('EACCES')], '/amb': [errno('EACCES')], '/amb/' + A1: [errno('ENOENT')]},
                 stat={CX.rstrip('/'): [st(3, 4)], '/amb': [st(3, 4)]}, ambient='/amb/' + A1),
     lambda r: (is_wire_host(r), need(['stat', CX.rstrip('/')] in r['trace'], 'identity of the parent'))),
    ('other', cf(realpath={CX.rstrip('/'): [errno('EACCES')], '/amb': [errno('EACCES')], '/amb/B': [errno('ENOENT')]},
                 stat={CX.rstrip('/'): [st(3, 4)], '/amb': [st(3, 4)]}, ambient='/amb/B'),
     lambda r: term(r, 'ok'))])

# R7: cwd and realpath success values that are {"u16":[...]} are decoded by the oracle.
CWD_PAIR = [0x2F, 0x77, 0xD83D, 0xDE00]
CWD_LONE = [0x2F, 0x77, 0xD800]
RELH = '/rel/data/engine-homes/codex/' + A1
RELARGS = codex_args(data_dir='rel/data', home='rel/data/engine-homes/codex/' + A1)
add('C11', 'cwd-u16-result', 'codex_account_folder', subs=[
    ('pair', cf(args=RELARGS, cwd=jv_u16(CWD_PAIR)),
     lambda r: (need(['cwd'] in r['trace'], 'cwd requested'), need(rp(r)[0] == u16_text(CWD_PAIR + [ord(c) for c in RELH]), 'resolved against the decoded cwd'))),
    ('lone', cf(args=RELARGS, cwd=jv_u16(CWD_LONE)),
     lambda r: (need(['cwd'] in r['trace'], 'cwd requested'), need(rp(r)[0] == u16_text(CWD_LONE + [ord(c) for c in RELH]), 'resolved against the decoded cwd')))])
RP_PAIR = [0x2F, 0x72, 0xD83D, 0xDE00]
RP_LONE = [0x2F, 0x72, 0xD800]
add('C12', 'realpath-u16-result', 'codex_account_folder', subs=[
    ('pair-equal', cf(realpath={CODEX_ACC: [ok_s(jv_u16(RP_PAIR))], '/amb': [ok_s(u16_text(RP_PAIR))]}, ambient='/amb'),
     lambda r: (is_wire_host(r), need(rp(r)[:2] == [CODEX_ACC, '/amb'], 'no climbing'))),
    ('lone-equal', cf(realpath={CODEX_ACC: [ok_s(jv_u16(RP_LONE))], '/amb': [ok_s(jv_u16(RP_LONE))]}, ambient='/amb'),
     lambda r: (is_wire_host(r), need(rp(r)[:2] == [CODEX_ACC, '/amb'], 'no climbing'))),
    ('lone-differs', cf(realpath={CODEX_ACC: [ok_s(jv_u16(RP_LONE))], '/amb': [ok_s(jv_u16([0x2F, 0x72, 0xD801]))]}, ambient='/amb'),
     lambda r: term(r, 'ok'))])

# =========================================================================================================== H
add('H01', 'codex-home-exact-mismatch', 'codex_account_folder',
    **{k: v for k, v in cf(args=codex_args(home='/elsewhere/' + A1)).items() if k != 'op'},
    check=lambda r: (term(r, 'throw_plain_error'), need(r['value']['message'] == PLAIN_MSG, 'message'),
                     need(r['trace'] == [], 'no filesystem request')))
add('H02', 'codex-canonical-ambient-alias', 'codex_account_folder',
    **{k: v for k, v in cf(realpath={CODEX_ACC: [ok_s('/real/x')], '/amb': [ok_s('/real/x')]}, ambient='/amb').items() if k != 'op'},
    check=lambda r: (is_wire_host(r), need(rp(r) == [CODEX_ACC, '/amb', '/home/test/.codex'] or rp(r)[:2] == [CODEX_ACC, '/amb'], 'sequence'),
                     need(stp(r) == [], 'no same-file call needed')))
add('H03', 'codex-canonical-dot-alias', 'codex_account_folder',
    **{k: v for k, v in cf(realpath={CODEX_ACC: [ok_s('/real/x')], '/home/test/.codex': [ok_s('/real/x')]}, ambient='/amb').items() if k != 'op'},
    check=lambda r: (is_wire_host(r), need(['realpath', '/home/test/.codex'] in r['trace'], 'dot canonicalized'),
                     need(stp(r) == [], 'rejected by canonical equality, not same-file')))
add('H04', 'codex-canonical-all-three', 'codex_account_folder',
    **{k: v for k, v in cf(realpath={CODEX_ACC: [ok_s('/real/x')], '/amb': [ok_s('/real/x')]}, ambient='/amb').items() if k != 'op'},
    check=lambda r: (is_wire_host(r), need(r['trace'].index(['realpath', '/home/test/.codex']) > r['trace'].index(['realpath', '/amb']), 'dot after host'),
                     need(['realpath', CODEX_ACC] == r['trace'][0], 'target first')))
add('H05', 'codex-samefile-ambient', 'codex_account_folder',
    **{k: v for k, v in cf(stat={CODEX_ACC: [st(1, 2)], '/amb': [st(1, 2)]}, ambient='/amb').items() if k != 'op'},
    check=lambda r: (is_wire_host(r), need(r['trace'][-2:] == [['stat', CODEX_ACC], ['stat', '/amb']], 'same-file stats last'),
                     need(['stat', '/home/test/.codex'] not in r['trace'], 'dot same-file not reached')))
add('H06', 'codex-samefile-dot', 'codex_account_folder',
    **{k: v for k, v in cf(stat={CODEX_ACC: [st(1, 2), st(1, 2)], '/amb': [st(9, 9)], '/home/test/.codex': [st(1, 2)]}, ambient='/amb').items() if k != 'op'},
    check=lambda r: (is_wire_host(r), need(stp(r) == [CODEX_ACC, '/amb', CODEX_ACC, '/home/test/.codex'], 'both comparisons, the dot one after the first fails')))
add('H07', 'codex-samefile-short-circuit', 'codex_account_folder',
    **{k: v for k, v in cf(stat={CODEX_ACC: [st(1, 2), st(1, 2)], '/amb': [st(1, 2)], '/home/test/.codex': [st(1, 2)]}, ambient='/amb').items() if k != 'op'},
    check=lambda r: (is_wire_host(r), need(stp(r) == [CODEX_ACC, '/amb'], 'second same-file call suppressed')))
add('H08', 'codex-samefile-stat-failure', 'codex_account_folder',
    **{k: v for k, v in cf(stat={CODEX_ACC: [errno('EACCES'), st(1, 2)], '/home/test/.codex': [st(1, 2)]}, ambient='/amb').items() if k != 'op'},
    check=lambda r: (is_wire_host(r), need(stp(r) == [CODEX_ACC, CODEX_ACC, '/home/test/.codex'], 'failed first call (left only), then later comparison')))
add('H09', 'codex-distinct-accepted', 'codex_account_folder',
    **{k: v for k, v in cf(realpath={CODEX_ACC: [ok_s('/real/x')], '/amb': [ok_s('/real/y')], '/home/test/.codex': [ok_s('/real/z')]},
                           stat={CODEX_ACC: [st(1, 2)], '/amb': [st(3, 4)], '/home/test/.codex': [st(5, 6)]}, ambient='/amb').items() if k != 'op'},
    check=lambda r: (term(r, 'ok'), need(r['value'] == CODEX_ACC, 'expected folder returned'),
                     need(len(stp(r)) == 4, 'both same-file comparisons ran')))
def stn(dev, ino):
    """A stat entry whose dev/ino are given verbatim (strings or integers)."""
    return {'ok': {'dev': dev, 'ino': ino}}


# R5: dev/ino are canonical decimals: leading zeros do not make two identities different.
add('H11', 'codex-samefile-leading-zeros', 'codex_account_folder', subs=[
    ('dev-zero-padded', cf(stat={CODEX_ACC: [stn('01', '2')], '/amb': [stn('1', '2')]}, ambient='/amb'), is_wire_host),
    ('ino-zero-padded', cf(stat={CODEX_ACC: [stn('1', '2')], '/amb': [stn('1', '0002')]}, ambient='/amb'), is_wire_host),
    ('both-padded', cf(stat={CODEX_ACC: [stn('007', '0010')], '/amb': [stn('7', '10')]}, ambient='/amb'), is_wire_host),
    ('zero-spellings', cf(stat={CODEX_ACC: [stn('00', '000')], '/amb': [stn('0', '0')]}, ambient='/amb'), is_wire_host),
    ('integer-vs-padded', cf(stat={CODEX_ACC: [stn(5, 6)], '/amb': [stn('005', '06')]}, ambient='/amb'), is_wire_host),
    ('still-different', cf(stat={CODEX_ACC: [stn('01', '2')], '/amb': [stn('1', '3')], '/home/test/.codex': [stn('9', '9')]}, ambient='/amb'),
     lambda r: (term(r, 'ok'), need(len(stp(r)) == 4, 'both comparisons ran')))])
add('H10', 'codex-account-failure-continues', 'import_sources', args={'env': {}},
    script={**acc_script({'ok': []}, {'ok': [acc(A1), acc(A2)]}), 'ambient': [{'ok': '/amb'}],
            'realpath': {CODEX_ACC: [ok_s('/real/x')], '/amb': [ok_s('/real/x')]}},
    check=lambda r: (need([v['account'] for v in acct_rows(r, 'codex')] == [A2], 'rejected skipped, later retained'),
                     need(sources(r)[-3:] == ['grok:home', 'opencode:home', 'qwen:home'], 'final homes retained'),
                     need('codex:home' in sources(r), 'codex home retained')))

# =========================================================================================================== F
def fs(sources_, query, **kw):
    return {'op': 'find_source', 'args': {'sources': [{'source': s} for s in sources_], 'query': query}}


UNDEF_V = {'undef': 1}
add('F01', 'find-empty-array', 'find_source', **{k: v for k, v in fs([], 'a').items() if k != 'op'},
    check=lambda r: need(r['value'] == UNDEF_V, 'undefined'))
add('F02', 'find-first-entry', 'find_source', **{k: v for k, v in fs(['a', 'b', 'c'], 'a').items() if k != 'op'}, check=lambda r: need(r['value'] == 0, 'index 0'))
add('F03', 'find-last-entry', 'find_source', **{k: v for k, v in fs(['a', 'b', 'c'], 'c').items() if k != 'op'}, check=lambda r: need(r['value'] == 2, 'last'))
add('F04', 'find-missing', 'find_source', **{k: v for k, v in fs(['a', 'b'], 'zz').items() if k != 'op'},
    check=lambda r: (term(r, 'ok'), need(r['value'] == UNDEF_V, 'undefined')))
add('F05', 'find-duplicate-first-wins', 'find_source', **{k: v for k, v in fs(['x', 'y', 'x', 'x'], 'x').items() if k != 'op'},
    check=lambda r: need(r['value'] == 0, 'first duplicate'))
add('F06', 'find-case-sensitive', 'find_source', **{k: v for k, v in fs(['claude:home', 'Claude:Home'], 'CLAUDE:home').items() if k != 'op'},
    check=lambda r: need(r['value'] == UNDEF_V, 'case variant does not match'))
add('F07', 'find-no-trim', 'find_source', subs=[
    ('query', fs(['claude:home'], ' claude:home '), lambda r: need(r['value'] == UNDEF_V, 'padded query')),
    ('source', fs([' claude:home', 'claude:home'], 'claude:home'), lambda r: need(r['value'] == 1, 'padded source differs'))])
add('F08', 'find-empty-id', 'find_source', **{k: v for k, v in fs(['a', '', 'b'], '').items() if k != 'op'}, check=lambda r: need(r['value'] == 1, 'empty id matches'))
add('F09', 'find-colons-verbatim', 'find_source', **{k: v for k, v in fs(['a', 'a:b', 'a:b:c', 'a:b:c:d'], 'a:b:c').items() if k != 'op'},
    check=lambda r: need(r['value'] == 2, 'whole string'))
LONE = jv_u16([0xD800])
add('F10', 'find-surrogate-identity', 'find_source', subs=[
    ('lone', fs([LONE, '�'], LONE), lambda r: need(r['value'] == 0, 'lone surrogate matches itself, not FFFD')),
    ('fffd', fs([LONE, '�'], '�'), lambda r: need(r['value'] == 1, 'FFFD is not the lone surrogate')),
    ('low', fs([jv_u16([0xDC00]), LONE], LONE), lambda r: need(r['value'] == 1, 'high vs low'))])

# =========================================================================================================== D
def dsrc(engine, **extra):
    out = {'ref': 's1', 'engine': engine}
    out.update(extra)
    return out


def dcase(engine, rows=None, **extra):
    src = dsrc(engine, **extra)
    return [('list', {'op': 'list_external', 'args': {'source': src, 'folder': '/f', 'options': {'ref': 'o1'}},
                      'script': {'engine': rows or ok_rows()}},
             lambda r, e=engine: (term(r, 'ok'), need(r['trace'] == [['engine', e, 'list', {'source': {'ref': 's1'}, 'folder': '/f', 'options': 'o1'}]] or
                                                       (len(r['trace']) == 1 and r['trace'][0][1] == e), f'list routed to {e}'))),
            ('read', {'op': 'read_external', 'args': {'source': src, 'folder': '/f', 'thread': 't1', 'options': {'ref': 'o1'}},
                      'script': {'engine': eng(record='rec')}},
             lambda r, e=engine: (term(r, 'ok'), need(len(r['trace']) == 1 and r['trace'][0][:3] == ['engine', e, 'read'], f'read routed to {e}'),
                                  need(r['value'] == {'ref': 'rec'}, 'record')))]


for cid, name, engine, route in [('D01', 'dispatch-claude', 'claude', 'claude'), ('D02', 'dispatch-codex', 'codex', 'codex'),
                                 ('D03', 'dispatch-opencode', 'opencode', 'opencode'), ('D04', 'dispatch-qwen', 'qwen', 'qwen'),
                                 ('D05', 'dispatch-grok', 'grok', 'grok')]:
    add(cid, name, 'list_external', subs=dcase(engine))
add('D06', 'dispatch-unknown-falls-grok', 'list_external', subs=[
    (suffix, over, (lambda r, s=suffix: (term(r, 'ok'), need(r['trace'][0][1] == 'grok', 'unknown engine selects grok'),
                                         need(len(r['trace']) == 1, 'once'))))
    for suffix, over, _ in dcase('gemini')] + [
    ('list-unknown2', {'op': 'list_external', 'args': {'source': dsrc('Claude'), 'folder': '/f', 'options': {'ref': 'o1'}},
                       'script': {'engine': ok_rows()}}, lambda r: need(r['trace'][0][1] == 'grok', 'case-variant name is unknown'))])
add('D07', 'dispatch-empty-falls-grok', 'list_external', subs=[
    (suffix, over, (lambda r: (term(r, 'ok'), need(r['trace'][0][1] == 'grok', 'empty engine selects grok'))))
    for suffix, over, _ in dcase('')])
add('D08', 'dispatch-arguments-identical', 'list_external', subs=[
    ('list', {'op': 'list_external', 'args': {'source': dsrc('codex', source='codex:home', home='/h', kind='home'), 'folder': jv_string('/f\ud800x'), 'options': {'ref': 'o9'}},
              'script': {'engine': ok_rows()}},
     lambda r: need(r['trace'] == [['engine', 'codex', 'list', {'source': {'ref': 's1'}, 'folder': jv_string('/f\ud800x'), 'options': {'ref': 'o9'}}]], 'list args')),
    ('read', {'op': 'read_external', 'args': {'source': dsrc('qwen', source='qwen:home'), 'folder': '/folder', 'thread': jv_u16([0x74, 0xD800]), 'options': {'ref': 'o9'}},
               'script': {'engine': eng(record='rec')}},
     lambda r: need(r['trace'] == [['engine', 'qwen', 'read', {'source': {'ref': 's1'}, 'folder': '/folder', 'thread': jv_u16([0x74, 0xD800]), 'options': {'ref': 'o9'}}]], 'read args'))])
add('D09', 'dispatch-exactly-once', 'list_external', subs=[
    ('list', {'op': 'list_external', 'args': {'source': dsrc('claude'), 'folder': '/f', 'options': {'ref': 'o1'}},
               'script': {'engine': eng(reject=err('e1', cls='EngineImportError', code='not-found'))}},
     lambda r: (term(r, 'throw_existing'), need(len(r['trace']) == 1, 'single invocation, no fallback'))),
    ('read', {'op': 'read_external', 'args': {'source': dsrc('claude'), 'folder': '/f', 'thread': 't', 'options': {'ref': 'o1'}},
               'script': {'engine': eng(reject=err('e1', cls='EngineImportError', code='not-found'))}},
     lambda r: (term(r, 'throw_existing'), need(len(r['trace']) == 1, 'single invocation'))),
    ('sync', {'op': 'list_external', 'args': {'source': dsrc('opencode'), 'folder': '/f', 'options': {'ref': 'o1'}},
               'script': {'engine': eng(sync_throw=err('e2', message='sync'))}},
     lambda r: (need(len(r['trace']) == 1, 'single invocation after a synchronous throw'),))])
add('D10', 'dispatch-read-record-identity', 'read_external', subs=[
    (e, {'op': 'read_external', 'args': {'source': dsrc(e), 'folder': '/f', 'thread': 't', 'options': {'ref': 'o1'}},
         'script': {'engine': eng(record=f'rec_{e}')}},
     lambda r, e=e: (term(r, 'ok'), need(r['value'] == {'ref': f'rec_{e}'}, 'same record'))) for e in ['claude', 'codex', 'opencode', 'qwen', 'grok']])

# =========================================================================================================== T
def tlist(*titles, extra=None):
    rows = []
    for i, t in enumerate(titles):
        rows.append(row(f'r{i + 1}', t, **(extra or {})))
    return {'op': 'list_external', 'args': {'source': dsrc('claude'), 'folder': '/f', 'options': {'ref': 'o1'}}, 'script': {'engine': eng(rows=rows)}}


def units(s):
    raw = s.encode('utf-16-le', 'surrogatepass')
    return [int.from_bytes(raw[i:i + 2], 'little') for i in range(0, len(raw), 2)]


def title_units(entry):
    t = entry[2]
    return t['u16'] if isinstance(t, dict) else units(t)


def keep(i):
    return ['keep', {'ref': f'r{i}'}]


def cut(i, expected_units):
    def check(entry):
        need(entry[0] == 'copy_title' and entry[1] == {'ref': f'r{i}'}, f'row {i} copied')
        need(title_units(entry) == expected_units, f'row {i} title units')
    return check


add('T01', 'title-empty', 'list_external', **{k: v for k, v in tlist('').items() if k != 'op'},
    check=lambda r: need(r['value'] == [keep(1)], 'empty title, same row'))
add('T02', 'title-199', 'list_external', **{k: v for k, v in tlist('x' * 199).items() if k != 'op'}, check=lambda r: need(r['value'] == [keep(1)], 'unchanged'))
add('T03', 'title-200', 'list_external', **{k: v for k, v in tlist('x' * 200).items() if k != 'op'}, check=lambda r: need(r['value'] == [keep(1)], 'unchanged same row'))
add('T04', 'title-201', 'list_external', **{k: v for k, v in tlist('x' * 200 + 'y').items() if k != 'op'},
    check=lambda r: cut(1, units('x' * 200))(r['value'][0]))
BIG = ''.join(chr(0x61 + i % 26) for i in range(5000))
add('T05', 'title-many-kilobytes', 'list_external', **{k: v for k, v in tlist(BIG).items() if k != 'op'},
    check=lambda r: cut(1, units(BIG[:200]))(r['value'][0]))
PAIR = '\U0001F600'
add('T06', 'title-pair-fits', 'list_external', **{k: v for k, v in tlist('x' * 198 + PAIR).items() if k != 'op'},
    check=lambda r: need(r['value'] == [keep(1)], 'exactly 200 units retained'))
add('T07', 'title-pair-split', 'list_external', **{k: v for k, v in tlist('x' * 199 + PAIR).items() if k != 'op'},
    check=lambda r: (cut(1, [0x78] * 199 + [0xD83D])(r['value'][0]), need(title_units(r['value'][0])[-1] == 0xD83D, 'trailing high surrogate retained')))
LH = jv_u16([0x61, 0xD800, 0x62, 0xDC00, 0xDC01, 0xD800])
LONG_LONE = jv_u16([0xD800, 0xDC00 - 0x400 + 0x400] + [0x78] * 250)
add('T08', 'title-lone-surrogates', 'list_external', subs=[
    ('short', tlist(LH), lambda r: need(r['value'] == [keep(1)], 'short title with lone surrogates unchanged')),
    ('long', tlist(jv_u16([0xDC00, 0xD800] + [0x78] * 250)), lambda r: cut(1, [0xDC00, 0xD800] + [0x78] * 198)(r['value'][0])),
    ('cut-between-pair', tlist(jv_u16([0x78] * 199 + [0xD83D, 0xDE00, 0x79])), lambda r: cut(1, [0x78] * 199 + [0xD83D])(r['value'][0]))])
add('T09', 'title-whitespace-not-cleaned', 'list_external', subs=[
    ('short', tlist('\n  pad \t\n'), lambda r: need(r['value'] == [keep(1)], 'unchanged')),
    ('long', tlist('\n' + ' ' * 10 + 'x' * 300), lambda r: cut(1, units('\n' + ' ' * 10 + 'x' * 189))(r['value'][0])),
    ('trailing', tlist('x' * 190 + ' ' * 20 + 'tail'), lambda r: cut(1, units('x' * 190 + ' ' * 10))(r['value'][0]))])
META = {'a': 1, 'o': {'ref': 'obj1'}, 'n': {'num': '8000000000000000'}, 'arr': {'arr': [1, {'undef': 1}, None]}, 'u': {'undef': 1}, 'z': 'last'}
add('T10', 'title-metadata-and-order', 'list_external', **{k: v for k, v in {
    'op': 'list_external', 'args': {'source': dsrc('claude'), 'folder': '/f', 'options': {'ref': 'o1'}},
    'script': {'engine': eng(rows=[
        {'ref': 'r1', 'title': 'x' * 250, **META},
        {'ref': 'r2', 'title': 'short', 'b': 2},
        {'ref': 'r3', **{'a': 1}, 'title': 'y' * 201, 'c': {'ref': 'obj2'}},
        {'ref': 'r4', 'title': ''}])}}.items() if k != 'op'},
    check=lambda r: (need([e[1]['ref'] for e in r['value']] == ['r1', 'r2', 'r3', 'r4'], 'list order'),
                     need([e[0] for e in r['value']] == ['copy_title', 'keep', 'copy_title', 'keep'], 'copy only where cut'),
                     need(r['terminal'] == 'ok', 'oracle verified every other field and key order')))

# =========================================================================================================== R
def rl(err_spec, op='list_external', mode='reject'):
    engine = eng(reject=err_spec) if mode == 'reject' else eng(sync_throw=err_spec)
    if op == 'list_external':
        return {'op': op, 'args': {'source': dsrc('claude'), 'folder': '/f', 'options': {'ref': 'o1'}}, 'script': {'engine': engine}}
    return {'op': op, 'args': {'source': dsrc('claude'), 'folder': '/f', 'thread': 't', 'options': {'ref': 'o1'}}, 'script': {'engine': engine}}


def passes(ref):
    return lambda r: (term(r, 'throw_existing'), need(r['value'] == {'ref': ref}, 'passes through by identity'))


def wraps(cause_ref, message='read-failed'):
    def check(r):
        term(r, 'throw_import')
        need(r['value']['code'] == 'read-failed', 'code')
        need(r['value']['message'] == message, f"message {r['value']['message']!r} != {message!r}")
        need(r['value']['hasCause'] is True and r['value'].get('cause') == {'ref': cause_ref}, 'cause identity')
    return check


add('R01', 'rethrow-import-all-codes', 'list_external',
    subs=[(c, rl(err('e1', cls='EngineImportError', code=c, message='m')), passes('e1')) for c in CODES] +
         [(c + '-read', rl(err('e1', cls='EngineImportError', code=c), op='read_external'), passes('e1')) for c in CODES[:2]])
add('R02', 'rethrow-import-renamed', 'list_external', subs=[
    ('list', rl(err('e1', cls='EngineImportError', code='timeout', rename='SomethingElse')), passes('e1')),
    ('read', rl(err('e1', cls='EngineImportError', code='not-found', rename=''), op='read_external'), passes('e1'))])
add('R03', 'rethrow-abort-error', 'list_external', subs=[
    ('reject', rl(err('e1', name='AbortError', message='aborted')), passes('e1')),
    ('sync', rl(err('e1', name='AbortError'), mode='sync'), passes('e1')),
    ('read', rl(err('e1', name='AbortError'), op='read_external'), passes('e1'))])
add('R04', 'rethrow-abort-object', 'list_external', subs=[
    ('reject', rl(err('e1', cls='plain', name='AbortError')), passes('e1')),
    ('read', rl(err('e1', cls='plain', name='AbortError'), op='read_external'), passes('e1'))])
add('R05', 'rethrow-fake-import-name', 'list_external', subs=[
    ('plain', rl(err('e1', cls='plain', name='EngineImportError', code='not-found', message='fake')), wraps('e1')),
    ('error', rl(err('e1', name='EngineImportError', message='fake-err')), wraps('e1', 'read-failed: fake-err'))])
add('R06', 'rethrow-error-detail', 'list_external', subs=[
    ('ascii', rl(err('e1', message='the exact detail')), wraps('e1', 'read-failed: the exact detail')),
    ('unicode', rl(err('e1', message=jv_u16(units('d\u00e9tail ') + [0xD800] + units(' \U0001F600 end')))), wraps('e1', jv_u16(units('read-failed: d\u00e9tail ') + [0xD800] + units(' \U0001F600 end')))),
    ('type-error', rl(err('e1', cls='TypeError', message='tt')), wraps('e1', 'read-failed: tt')),
    ('read', rl(err('e1', message='r d'), op='read_external'), wraps('e1', 'read-failed: r d'))])
add('R07', 'rethrow-empty-error-detail', 'list_external', subs=[
    ('empty', rl(err('e1', message='')), wraps('e1', 'read-failed')),
    ('read', rl(err('e1', message=''), op='read_external'), wraps('e1', 'read-failed'))])
add('R08', 'rethrow-null-undefined', 'list_external', subs=[
    ('null', rl({'class': 'primitive', 'ref': 'p_null', 'value': None}), wraps('p_null')),
    ('undefined', rl({'class': 'primitive', 'ref': 'p_undef', 'value': {'undef': 1}}), wraps('p_undef')),
    ('null-read', rl({'class': 'primitive', 'ref': 'p_null', 'value': None}, op='read_external'), wraps('p_null'))])
add('R09', 'rethrow-primitive-values', 'list_external', subs=[
    ('string', rl({'class': 'primitive', 'ref': 'p_str', 'value': 'oops'}), wraps('p_str')),
    ('number', rl({'class': 'primitive', 'ref': 'p_num', 'value': 42}), wraps('p_num')),
    ('boolean', rl({'class': 'primitive', 'ref': 'p_bool', 'value': False}), wraps('p_bool')),
    ('string-read', rl({'class': 'primitive', 'ref': 'p_str', 'value': 'oops'}, op='read_external'), wraps('p_str')),
    ('object', rl(err('e1', cls='plain', message='ignored')), wraps('e1'))])
add('R10', 'rethrow-name-near-misses', 'list_external', subs=[
    ('timeout-error', rl(err('e1', name='TimeoutError', message='slow')), wraps('e1', 'read-failed: slow')),
    ('lowercase', rl(err('e1', cls='plain', name='aborterror')), wraps('e1')),
    ('padded', rl(err('e1', cls='plain', name='AbortError ')), wraps('e1')),
    ('import-lookalike-error', rl(err('e1', name='engineimporterror')), wraps('e1', 'read-failed'))])

# =========================================================================================================== U
def thr(session, **kw):
    out = {'op': 'session_threads', 'args': {'session': session}}
    out.update(kw)
    return out


def pairs(*p):
    return [list(x) for x in p]


def tv(want):
    return lambda r: (term(r, 'ok'), need(r['value'] == want, f"threads {json.dumps(r['value'])} != {json.dumps(want)}"))


add('U01', 'threads-empty-session', 'session_threads', subs=[
    ('bare', thr(sess()), tv([])), ('explicit-nulls', thr(sess(externalThread=None, engineThreads=None, terminal=None)), tv([]))])
add('U02', 'threads-origin-only', 'session_threads', **{k: v for k, v in thr(sess(externalThread={'engine': 'claude', 'thread': 't1'})).items() if k != 'op'},
    check=tv([['claude', 't1']]))
add('U03', 'threads-forks-order', 'session_threads',
    **{k: v for k, v in thr(sess(externalThread={'engine': 'claude', 'thread': 't0', 'forks': {'arr': ['f3', 'f1', 'f2']}})).items() if k != 'op'},
    check=tv([['claude', 't0'], ['claude', 'f3'], ['claude', 'f1'], ['claude', 'f2']]))
add('U04', 'threads-nullish-forks', 'session_threads', subs=[
    ('absent', thr(sess(externalThread={'engine': 'claude', 'thread': 't'})), tv([['claude', 't']])),
    ('null', thr(sess(externalThread={'engine': 'claude', 'thread': 't', 'forks': None})), tv([['claude', 't']])),
    ('undefined', thr(sess(externalThread={'engine': 'claude', 'thread': 't', 'forks': {'undef': 1}})), tv([['claude', 't']])),
    ('empty-array', thr(sess(externalThread={'engine': 'claude', 'thread': 't', 'forks': {'arr': []}})), tv([['claude', 't']]))])
add('U05', 'threads-empty-origin-fork', 'session_threads',
    **{k: v for k, v in thr(sess(externalThread={'engine': '', 'thread': '', 'forks': {'arr': ['', 'f']}})).items() if k != 'op'},
    check=tv([['', ''], ['', ''], ['', 'f']]))
add('U06', 'threads-duplicate-forks', 'session_threads',
    **{k: v for k, v in thr(sess(externalThread={'engine': 'codex', 'thread': 't', 'forks': {'arr': ['t', 'f', 'f', 't']}})).items() if k != 'op'},
    check=tv([['codex', 't'], ['codex', 't'], ['codex', 'f'], ['codex', 'f'], ['codex', 't']]))
add('U07', 'threads-terminal-default', 'session_threads', subs=[
    ('absent', thr(sess(terminal={'cliSession': 'cli1'})), tv([['codex', 'cli1']])),
    ('undefined', thr(sess(engine={'undef': 1}, terminal={'cliSession': 'cli1'})), tv([['codex', 'cli1']]))])
add('U08', 'threads-terminal-null-engine', 'session_threads', **{k: v for k, v in thr(sess(engine=None, terminal={'cliSession': 'cli1'})).items() if k != 'op'},
    check=tv([['codex', 'cli1']]))
add('U09', 'threads-terminal-empty-engine', 'session_threads', subs=[
    ('empty', thr(sess(engine='', terminal={'cliSession': 'cli1'})), tv([['', 'cli1']])),
    ('named', thr(sess(engine='qwen', terminal={'cliSession': 'cli1'})), tv([['qwen', 'cli1']]))])
add('U10', 'threads-terminal-truthiness', 'session_threads', subs=[
    ('empty-cli', thr(sess(terminal={'cliSession': '', 'started': True})), tv([])),
    ('space-cli', thr(sess(terminal={'cliSession': ' ', 'started': False})), tv([['codex', ' ']])),
    ('started-ignored', thr(sess(terminal={'cliSession': 'c', 'started': False})), tv([['codex', 'c']])),
    ('missing-cli', thr(sess(terminal={'started': True})), tv([])),
    ('null-cli', thr(sess(terminal={'cliSession': None})), tv([])),
    ('null-terminal', thr(sess(terminal=None)), tv([]))])

# =========================================================================================================== V
def slots(*entries, **kw):
    return thr(sess(engineThreads={'obj': [[k, b] for k, b in entries]}, **kw))


def tid(x):
    return {'threadId': x}


add('V01', 'slot-no-colon', 'session_threads', **{k: v for k, v in slots(('claude', tid('t')), ).items() if k != 'op'}, check=tv([['claude', 't']]))
add('V02', 'slot-one-colon', 'session_threads', **{k: v for k, v in slots(('codex:abc', tid('t')), ).items() if k != 'op'}, check=tv([['codex', 't']]))
add('V03', 'slot-many-colons', 'session_threads', **{k: v for k, v in slots(('a:b:c:d', tid('t')), ).items() if k != 'op'}, check=tv([['a', 't']]))
add('V04', 'slot-leading-colon', 'session_threads', **{k: v for k, v in slots((':x', tid('t')), ).items() if k != 'op'}, check=tv([['', 't']]))
add('V05', 'slot-empty-name', 'session_threads', **{k: v for k, v in slots(('', tid('t')), ).items() if k != 'op'}, check=tv([['', 't']]))
add('V06', 'slot-unknown-engine', 'session_threads', **{k: v for k, v in slots(('mystery:q', tid('t')), ).items() if k != 'op'}, check=tv([['mystery', 't']]))
add('V07', 'slot-empty-thread', 'session_threads', subs=[
    ('empty', slots(('claude:a', tid(''))), tv([])),
    ('empty-keeps-later', slots(('claude:a', tid('')), ('codex:b', tid('x'))), tv([['codex', 'x']])),
    ('missing-thread-id', slots(('claude:a', {'other': 1})), tv([]))])
add('V08', 'slot-nullish-bound', 'session_threads', subs=[
    ('null', slots(('claude:a', None), ('codex:b', tid('x'))), tv([['codex', 'x']])),
    ('undefined', slots(('claude:a', {'undef': 1}), ('codex:b', tid('x'))), tv([['codex', 'x']])),
    ('absent', slots(('claude:a', {'absent': 1}), ('codex:b', tid('x'))), tv([['codex', 'x']])),
    ('null-threadid', slots(('claude:a', {'threadId': None}), ('codex:b', tid('x'))), tv([['codex', 'x']]))])
add('V09', 'slot-object-entry-order', 'session_threads', subs=[
    ('integer-first', slots(('b', tid('tb')), ('2', tid('t2')), ('a', tid('ta')), ('1', tid('t1')), ('10', tid('t10')), ('0', tid('t0'))),
     tv([['0', 't0'], ['1', 't1'], ['2', 't2'], ['10', 't10'], ['b', 'tb'], ['a', 'ta']])),
    ('non-canonical-int', slots(('01', tid('x1')), ('1', tid('x2')), ('-1', tid('x3')), ('4294967295', tid('x4')), ('4294967294', tid('x5'))),
     lambda r: (term(r, 'ok'), need(len(r['value']) == 5, 'five'))),
    ('repeat-key', slots(('a', tid('1')), ('b', tid('2')), ('a', tid('3'))), tv([['a', '3'], ['b', '2']]))])
add('V10', 'slot-after-origin-terminal', 'session_threads',
    **{k: v for k, v in slots(('claude:s', tid('slot1')), ('codex', tid('slot2')),
                              engine='qwen', externalThread={'engine': 'claude', 'thread': 'o', 'forks': {'arr': ['f']}}, terminal={'cliSession': 'cli'}).items() if k != 'op'},
    check=tv([['claude', 'o'], ['claude', 'f'], ['qwen', 'cli'], ['claude', 'slot1'], ['codex', 'slot2']]))

# R6: a "__proto__" key is an own data property (JSON.parse semantics), never the prototype setter.
add('V11', 'slot-proto-key', 'session_threads', subs=[
    ('slot-name', slots(('__proto__', tid('t')), ('a', tid('u'))), tv([['__proto__', 't'], ['a', 'u']])),
    ('slot-name-only', slots(('__proto__', tid('t'))), tv([['__proto__', 't']])),
    ('record-key-not-inherited', slots(('x', {'__proto__': {'threadId': 'p'}})), tv([])),
    ('record-key-beside-own', slots(('x', {'threadId': 'o', '__proto__': {'threadId': 'p'}})), tv([['x', 'o']]))])

# =========================================================================================================== X
def exb(session, callback=None, **kw):
    out = {'op': 'excluded_from_bound', 'args': {'session': session}}
    if callback is not None:
        out['script'] = {'isTemporary': callback}
    out.update(kw)
    return out


def calls_of(r):
    return [t for t in r['trace'] if t[0] == 'isTemporary']


add('X01', 'temporary-literal-true', 'excluded_from_bound', subs=[
    ('callback-true', exb(sess(temporary=True), [{'ok': True}]), lambda r: (need(r['value'] is True, 'excluded'), need(r['trace'] == [], 'no callback'))),
    ('callback-false', exb(sess(temporary=True), [{'ok': False}]), lambda r: (need(r['value'] is True, 'excluded'), need(r['trace'] == [], 'no callback'))),
    ('callback-throws', exb(sess(temporary=True), [{'throw': err('tx')}]), lambda r: (term(r, 'ok'), need(r['value'] is True, 'excluded without calling'), need(r['trace'] == [], 'no callback')))])
add('X02', 'temporary-missing-false-callback', 'excluded_from_bound', **{k: v for k, v in exb(sess(), [{'ok': False}]).items() if k != 'op'},
    check=lambda r: (need(r['value'] is False, 'retained'), need(calls_of(r) == [['isTemporary', 's']], 'one callback')))
add('X03', 'temporary-missing-true-callback', 'excluded_from_bound', **{k: v for k, v in exb(sess(), [{'ok': True}]).items() if k != 'op'},
    check=lambda r: (need(r['value'] is True, 'excluded'), need(calls_of(r) == [['isTemporary', 's']], 'one callback')))
add('X04', 'temporary-explicit-false', 'excluded_from_bound', subs=[
    ('callback-true', exb(sess(temporary=False), [{'ok': True}]), lambda r: (need(r['value'] is True, 'callback decides'), need(len(calls_of(r)) == 1, 'invoked'))),
    ('callback-false', exb(sess(temporary=False), [{'ok': False}]), lambda r: (need(r['value'] is False, 'retained'), need(len(calls_of(r)) == 1, 'invoked')))])
add('X05', 'temporary-null', 'excluded_from_bound', subs=[
    ('callback-true', exb(sess(temporary=None), [{'ok': True}]), lambda r: (need(r['value'] is True, 'callback decides'), need(len(calls_of(r)) == 1, 'invoked'))),
    ('undefined', exb(sess(temporary={'undef': 1}), [{'ok': False}]), lambda r: need(len(calls_of(r)) == 1, 'invoked'))])
add('X06', 'temporary-number-one', 'excluded_from_bound', subs=[
    ('callback-false', exb(sess(temporary=1), [{'ok': False}]), lambda r: (need(r['value'] is False, 'not literal true'), need(len(calls_of(r)) == 1, 'callback invoked'))),
    ('callback-true', exb(sess(temporary=1), [{'ok': True}]), lambda r: need(r['value'] is True, 'callback decides'))])
add('X07', 'temporary-string-true', 'excluded_from_bound', subs=[
    ('callback-false', exb(sess(temporary='true'), [{'ok': False}]), lambda r: (need(r['value'] is False, 'not literal true'), need(len(calls_of(r)) == 1, 'callback invoked'))),
    ('other-truthy', exb(sess(temporary={'ref': 'obj'}), [{'ok': False}]), lambda r: need(len(calls_of(r)) == 1, 'callback invoked'))])
add('X08', 'temporary-callback-throws', 'excluded_from_bound', subs=[
    ('error', exb(sess(), [{'throw': err('tx', message='no')}]), lambda r: (term(r, 'throw_existing'), need(r['value'] == {'ref': 'tx'}, 'original failure'))),
    ('primitive', exb(sess(), [{'throw': {'class': 'primitive', 'ref': 'tp', 'value': 'str'}}]), lambda r: (term(r, 'throw_existing'), need(r['value'] == {'ref': 'tp'}, 'original failure'))),
    ('bound', {'op': 'bound_session_index', 'args': {'sessions': [sess('s1', externalThread={'engine': 'claude', 'thread': 't'})]},
               'script': {'isTemporary': [{'throw': err('tx', message='no')}]}},
     lambda r: (term(r, 'throw_existing'), need(r['value'] == {'ref': 'tx'}, 'escapes the bound index')))])
add('X09', 'temporary-duplicate-id-calls', 'bound_session_index', **{
    'args': {'sessions': [sess('s1', temporary=True, externalThread={'engine': 'claude', 'thread': 'a'}),
                          sess('s1', externalThread={'engine': 'claude', 'thread': 'b'}),
                          sess('s1', temporary=False, externalThread={'engine': 'claude', 'thread': 'c'}),
                          sess('s1', temporary=1, externalThread={'engine': 'claude', 'thread': 'd'})]},
    'script': {'isTemporary': [{'ok': False}]}},
    check=lambda r: (need(calls_of(r) == [['isTemporary', 's1']] * 3, 'one call per non-literal-true occurrence'),
                     need(r['value'] == [['claude:b', 's1'], ['claude:c', 's1'], ['claude:d', 's1']], 'entries')))
add('X10', 'temporary-excludes-before-threads', 'bound_session_index', subs=[
    ('literal', {'args': {'sessions': [sess('s1', temporary=True, externalThread={'engine': 'claude', 'thread': 't', 'forks': {'arr': ['f']}}, engineThreads={'obj': [['codex:x', tid('z')]]}, terminal={'cliSession': 'c'}),
                                       sess('s2', externalThread={'engine': 'claude', 'thread': 't'})]}},
     lambda r: (need(r['value'] == [['claude:t', 's2']], 'excluded session contributes nothing'), need(calls_of(r) == [['isTemporary', 's2']], 'only s2 evaluated'))),
    ('callback', {'args': {'sessions': [sess('s1', externalThread={'engine': 'claude', 'thread': 't', 'forks': {'arr': ['f']}}, terminal={'cliSession': 'c'}),
                                        sess('s2', externalThread={'engine': 'claude', 'thread': 't'})]}, 'script': {'isTemporary': [{'ok': True}, {'ok': False}]}},
     lambda r: need(r['value'] == [['claude:t', 's2']], 'callback-excluded session contributes nothing'))])

# =========================================================================================================== B
def bnd(*sessions, **script):
    out = {'op': 'bound_session_index', 'args': {'sessions': list(sessions)}}
    if script:
        out['script'] = script
    return out


def ses(i, eng_, th, **kw):
    return sess(i, externalThread={'engine': eng_, 'thread': th}, **kw)


def bv(want):
    return lambda r: (term(r, 'ok'), need(r['value'] == want, f"bound {json.dumps(r['value'])} != {json.dumps(want)}"))


add('B01', 'bound-empty', 'bound_session_index', subs=[('none', bnd(), bv([])), ('no-threads', bnd(sess('s1'), sess('s2')), bv([]))])
add('B02', 'bound-one', 'bound_session_index', **{k: v for k, v in bnd(ses('s1', 'claude', 't1')).items() if k != 'op'}, check=bv([['claude:t1', 's1']]))
add('B03', 'bound-duplicate-first-wins', 'bound_session_index',
    **{k: v for k, v in bnd(ses('s1', 'claude', 't'), ses('s2', 'claude', 't'), ses('s3', 'claude', 't')).items() if k != 'op'}, check=bv([['claude:t', 's1']]))
add('B04', 'bound-temporary-first-skipped', 'bound_session_index', subs=[
    ('literal', bnd(ses('s1', 'claude', 't', temporary=True), ses('s2', 'claude', 't')), bv([['claude:t', 's2']])),
    ('callback', bnd(ses('s1', 'claude', 't'), ses('s2', 'claude', 't'), isTemporary=[{'ok': True}, {'ok': False}]), bv([['claude:t', 's2']]))])
add('B05', 'bound-within-session-duplicates', 'bound_session_index', subs=[
    ('forks', bnd(sess('s1', externalThread={'engine': 'claude', 'thread': 't', 'forks': {'arr': ['t', 'u', 't']}})), bv([['claude:t', 's1'], ['claude:u', 's1']])),
    ('slots', bnd(sess('s1', externalThread={'engine': 'claude', 'thread': 't'}, engineThreads={'obj': [['claude:x', tid('t')]]})), bv([['claude:t', 's1']]))])
add('B06', 'bound-colon-collision', 'bound_session_index', subs=[
    ('engine-thread', bnd(ses('s1', 'a', 'b:c'), ses('s2', 'a:b', 'c')), bv([['a:b:c', 's1']])),
    ('reverse', bnd(ses('s1', 'a:b', 'c'), ses('s2', 'a', 'b:c')), bv([['a:b:c', 's1']])),
    ('empty-parts', bnd(ses('s1', '', ':x'), ses('s2', ':', 'x')), lambda r: term(r, 'ok'))])
add('B07', 'bound-account-source-ignored', 'bound_session_index',
    **{k: v for k, v in bnd(ses('s1', 'claude', 't', source='claude:acc1', account=A1), ses('s2', 'claude', 't', source='claude:acc2', account=A2),
                            ses('s3', 'claude', 't', source='codex:home')).items() if k != 'op'},
    check=bv([['claude:t', 's1']]))
add('B08', 'bound-empty-id-still-present', 'bound_session_index', subs=[
    ('empty-first', bnd(ses('', 'claude', 't'), ses('s2', 'claude', 't')), bv([['claude:t', '']])),
    ('undefined-first', bnd(ses({'undef': 1}, 'claude', 't'), ses('s2', 'claude', 't')), bv([['claude:t', {'undef': 1}]]))])
add('B09', 'bound-insertion-order', 'bound_session_index',
    **{k: v for k, v in bnd(ses('s1', 'qwen', 'z'), ses('s2', 'claude', 'a'), ses('s3', 'codex', 'm'), ses('s4', 'claude', '3'), ses('s5', 'claude', '1')).items() if k != 'op'},
    check=bv([['qwen:z', 's1'], ['claude:a', 's2'], ['codex:m', 's3'], ['claude:3', 's4'], ['claude:1', 's5']]))
add('B10', 'bound-vs-title-map-winner', 'bound_session_index',
    **{k: v for k, v in bnd(ses('s1', 'claude', 't', title='first'), ses('s2', 'claude', 't', title='second'), ses('s1', 'claude', 'u', title='third')).items() if k != 'op'},
    check=lambda r: (need(r['value'] == [['claude:t', 's1'], ['claude:u', 's1']], 'bound first wins'),
                     need(r['callerTitles'] == [['s1', 'third'], ['s2', 'second']], 'caller title map last wins')))

# =========================================================================================================== P
def imp3():
    """A multi-request base case: Claude and Codex accounts give several observations."""
    return {'op': 'import_sources', 'args': {'env': {}}, 'script': acc_script({'ok': [acc(A1, 'one')]}, {'ok': [acc(A2, 'two')]})}


def same(want_check):
    return want_check


add('P01', 'packet-empty-vs-undefined', 'session_threads', subs=[
    ('empty', thr(sess(externalThread={'engine': 'claude', 'thread': ''})), tv([['claude', '']])),
    ('undefined', thr(sess(externalThread={'engine': 'claude', 'thread': {'undef': 1}})), tv([['claude', {'undef': 1}]])),
    ('empty-engine-undefined-thread', thr(sess(externalThread={'engine': '', 'thread': {'undef': 1}})), tv([['', {'undef': 1}]])),
    ('find', {'op': 'find_source', 'args': {'sources': [{'source': {'undef': 1}}, {'source': ''}], 'query': ''}}, lambda r: need(r['value'] == 1, 'empty differs from undefined'))])
add('P02', 'packet-null-vs-undefined', 'session_threads', subs=[
    ('null', thr(sess(externalThread={'engine': 'claude', 'thread': None})), tv([['claude', None]])),
    ('undefined', thr(sess(externalThread={'engine': 'claude', 'thread': {'undef': 1}})), tv([['claude', {'undef': 1}]])),
    ('cause-null', rl({'class': 'primitive', 'ref': 'p_null', 'value': None}), wraps('p_null')),
    ('cause-undefined', rl({'class': 'primitive', 'ref': 'p_undef', 'value': {'undef': 1}}), wraps('p_undef'))])
UNITS_ALL = [0x0000, 0x0041, 0xD800, 0xDBFF, 0xDC00, 0xDFFF, 0xFFFF, 0xFEFF, 0x00A0, 0xD83D, 0xDE00, 0xDE00, 0xD83D]
add('P03', 'packet-surrogate-roundtrip', 'session_threads', subs=[
    ('units', thr(sess(externalThread={'engine': 'claude', 'thread': jv_u16(UNITS_ALL)})), lambda r: need(r['value'] == [['claude', jv_u16(UNITS_ALL)]], 'units preserved')),
    ('pairs-and-lone', thr(sess(externalThread={'engine': 'claude', 'thread': 'a\U0001F600b', 'forks': {'arr': [jv_u16([0xDE00, 0xD83D])]}})),
     lambda r: need(r['value'][1] == ['claude', jv_u16([0xDE00, 0xD83D])], 'reversed pair stays two lone surrogates')),
    ('slot-thread-id', thr(sess(engineThreads={'obj': [['a:b', tid(jv_u16([0xDC00]))]]})),
     lambda r: need(r['value'] == [['a', jv_u16([0xDC00])]], 'lone surrogate in the slot thread id'))])
add('P04', 'packet-object-extra-fields', 'list_external', subs=[
    ('copy', {'op': 'list_external', 'args': {'source': dsrc('claude', extra_src=7), 'folder': '/f', 'options': {'ref': 'o1'}},
              'script': {'engine': eng(rows=[{'ref': 'r1', 'title': 'x' * 260, 'id': 'i1', 'n': {'num': '3ff8000000000000'}, 'o': {'ref': 'objx'},
                                              'a': {'arr': ['p', None, {'undef': 1}]}, 'm': {'obj': [['k', 1], ['j', 2]]}, 'z': {'undef': 1}}])}},
     lambda r: (need(r['value'][0][0] == 'copy_title', 'copied'), term(r, 'ok'))),
    ('keep', {'op': 'list_external', 'args': {'source': dsrc('claude'), 'folder': '/f', 'options': {'ref': 'o1'}},
              'script': {'engine': eng(rows=[{'ref': 'r1', 'title': 'x', 'obj': {'ref': 'objy'}}])}}, lambda r: need(r['value'] == [keep(1)], 'kept'))])
NEG0 = {'num': '8000000000000000'}
add('P05', 'packet-number-bits', 'session_threads', subs=[
    ('negative-zero', thr(sess(externalThread={'engine': 'claude', 'thread': NEG0})), lambda r: need(r['value'] == [['claude', NEG0]], '-0 preserved')),
    ('zero', thr(sess(externalThread={'engine': 'claude', 'thread': 0})), lambda r: need(r['value'] == [['claude', 0]], '+0')),
    ('finite', thr(sess(externalThread={'engine': 'claude', 'thread': {'num': '3ff8000000000000'}, 'forks': {'arr': [{'num': '7fefffffffffffff'}, {'num': '0000000000000001'}, 123456789012]}})),
     lambda r: need(len(r['value']) == 4, 'four')),
    ('forks', thr(sess(externalThread={'engine': 'claude', 'thread': 't', 'forks': {'arr': [NEG0, 0, -1, 9007199254740991, -9007199254740991]}})),
     lambda r: need(r['value'][1] == ['claude', NEG0] and r['value'][2] == ['claude', 0], 'negative zero vs zero'))])


def probe_entries():
    """P06-P10 probe cases: (id, suffix, base case dict, probe dict). Expect derived by probe_expect()."""
    base = imp3()
    one = thr(sess(externalThread={'engine': 'claude', 'thread': 't'}))
    return {
        'P06': [('version-begin', one, {'at': 0, 'corrupt': 'version'}), ('length-begin', one, {'at': 0, 'corrupt': 'length'}),
                ('version-step', base, {'at': 1, 'corrupt': 'version'}), ('length-step', base, {'at': 2, 'corrupt': 'length'}),
                ('tag-begin', one, {'at': 0, 'corrupt': 'tag'}), ('tag-step', base, {'at': 1, 'corrupt': 'tag'}),
                ('unit-begin', one, {'at': 0, 'corrupt': 'unit'}), ('unit-step', base, {'at': 3, 'corrupt': 'unit'})],
        'P07': [('op-id-begin', one, {'at': 0, 'corrupt': 'op_id'}), ('op-id-step', base, {'at': 1, 'corrupt': 'op_id'}),
                ('seq-begin', one, {'at': 0, 'corrupt': 'seq'}), ('seq-step', base, {'at': 2, 'corrupt': 'seq'})],
        'P08': [('duplicate-1', base, {'at': 1, 'corrupt': 'duplicate'}), ('duplicate-3', base, {'at': 3, 'corrupt': 'duplicate'})],
        'P09': [('reentry-threads', one, {'at': 0, 'corrupt': 'reentry'}), ('reentry-import', base, {'at': 0, 'corrupt': 'reentry'})],
        'P10': [('oversize-begin', one, {'at': 0, 'corrupt': 'oversize'}), ('oversize-step', base, {'at': 1, 'corrupt': 'oversize'}),
                ('exact-limit-begin', one, {'at': 0, 'corrupt': 'exact_limit'}), ('exact-limit-step', base, {'at': 1, 'corrupt': 'exact_limit'}),
                ('exact-limit-phys-begin', one, {'at': 0, 'corrupt': 'exact_limit_phys'}),
                ('exact-limit-phys-step', base, {'at': 1, 'corrupt': 'exact_limit_phys'}),
                ('oversize-phys-begin', one, {'at': 0, 'corrupt': 'oversize_phys'}),
                ('oversize-phys-step', base, {'at': 1, 'corrupt': 'oversize_phys'})],
    }


PROBE_NAMES = {'P06': 'packet-wrong-version-length', 'P07': 'packet-wrong-operation-sequence', 'P08': 'packet-duplicate-completion',
               'P09': 'packet-terminal-reentry', 'P10': 'packet-size-boundary'}
for pid, items in probe_entries().items():
    subs = []
    for suffix, base_case, probe in items:
        over = {'op': base_case['op'], 'args': base_case['args'], 'probe': probe}
        if 'script' in base_case:
            over['script'] = base_case['script']
        subs.append((suffix, over, None))
        PROBES[f'{pid}/{suffix}'] = probe
    add(pid, PROBE_NAMES[pid], items[0][1]['op'], subs=subs)

extra_word_case = thr(sess(externalThread={'engine': 'claude', 'thread': 't'}))
PROBES['P11'] = {'at': 0, 'corrupt': 'extra_word'}
add('P11', 'packet-extra-pool-word', extra_word_case['op'], args=extra_word_case['args'], probe=PROBES['P11'])


# =========================================================================================================== I
# Integration cases: integrate.py reads integration (route/op/piece/fixture) and runs the piece's own driver. expect describes
# the assertions; no oracle comparison (the oracle uses scripted fake engines, the integration uses the real piece driver).
INTEG = [
    ('I01', 'integration-claude-list', 'claude', 'list'), ('I02', 'integration-claude-read', 'claude', 'read'),
    ('I03', 'integration-qwen-list', 'qwen', 'list'), ('I04', 'integration-qwen-read', 'qwen', 'read'),
    ('I05', 'integration-codex-list', 'codex', 'list'), ('I06', 'integration-codex-read', 'codex', 'read'),
    ('I07', 'integration-grok-list', 'grok', 'list'), ('I08', 'integration-grok-read', 'grok', 'read'),
    ('I09', 'integration-opencode-list', 'opencode', 'list'), ('I10', 'integration-opencode-read', 'opencode', 'read'),
]


# =========================================================================================================== derive + write
def build_sub_cases(defaults):
    """Every runnable case (subs expanded, merged over defaults) with its group and checks, in fixture order."""
    out = []
    for g in GROUPS:
        for entry in ENTRIES[g]:
            for case in expand_case(entry):
                merged = merge_case(defaults, case)
                merged['_group'] = g
                out.append((case, merged))
    return out


def derive(entries_by_group):
    defaults = load_defaults()
    runnable = []
    for g in GROUPS:
        if g == 'I':
            continue
        for entry in entries_by_group[g]:
            for case in expand_case(entry):
                runnable.append((g, case, merge_case(defaults, case)))
    results = run_oracle([m for _, _, m in runnable])
    by_id = {}
    for (g, case, merged), res in zip(runnable, results):
        cid = case['id']
        need(res.get('terminal') != 'native_throw', f'{cid}: oracle failure {res}')
        need(res['id'] == cid, f'{cid}: id mismatch')
        by_id[cid] = (case, res)
    return by_id


def expect_of(cid, case, res):
    probe = PROBES.get(cid)
    if probe is not None:
        return probe_expect(probe, res, case.get('script'))
    check = CHECKS.get(cid)
    need(check is not None, f'{cid}: no check function')
    try:
        check(res)
    except AssertionError as e:
        FAILS.append(f'{cid}: {e}')
    exp = {'terminal': res['terminal'], 'value': res['value'], 'trace': res['trace']}
    if 'callerTitles' in res:
        exp['callerTitles'] = res['callerTitles']
    return exp


def finish(entries_by_group):
    by_id = derive(entries_by_group)
    out = {}
    for g in GROUPS:
        entries = []
        for entry in entries_by_group[g]:
            entry = json.loads(json.dumps(entry))
            if g == 'I':
                entries.append(entry)
                continue
            if 'subs' in entry:
                for sub in entry['subs']:
                    case, res = by_id[sub['id']]
                    sub['expect'] = expect_of(sub['id'], case, res)
            else:
                case, res = by_id[entry['id']]
                entry['expect'] = expect_of(entry['id'], case, res)
            if entry['id'] in MUTANT_KILLERS:
                entry['mutant'] = MUTANT_KILLERS[entry['id']]
            entries.append(entry)
        out[g] = entries
    return out


def add_integration(plan):
    for cid, name, route, op in INTEG:
        plan['I'].append({'id': cid, 'name': name, 'integration': plan_integration(route, op)})


def plan_integration(route, op):
    import integrate
    return integrate.fixture_for(route, op)


def main(argv):
    check_only = '--check' in argv
    add_integration(ENTRIES)
    out = finish(ENTRIES)
    count = 0
    for g, entries in out.items():
        for e in entries:
            count += 1
    if FAILS:
        print('\n'.join(FAILS))
        return 1
    need(count == 156, f"{count} entries, expected 156")
    names = [e['id'] for g in GROUPS for e in out[g]]
    need(len(set(names)) == 156, 'ids unique')
    status = 0
    for g in GROUPS:
        text = json.dumps(out[g], ensure_ascii=False, indent=1) + '\n'
        path = FIXTURES / f'{g}.json'
        if check_only:
            if not path.exists() or path.read_text(encoding='utf-8') != text:
                print(f'differs: {path.name}')
                status = 1
        else:
            path.write_text(text, encoding='utf-8')
    n_sub = sum(len(e.get('subs', [None])) for g in GROUPS for e in out[g])
    print(f'{count} fixtures, {n_sub} runnable variants, {len(MUTANT_KILLERS)} mutant-killers' + ('' if not check_only else (' (check ok)' if status == 0 else ' (check FAILED)')))
    return status


if __name__ == '__main__':
    sys.exit(main(sys.argv))
