#!/usr/bin/env python3
"""L3-9 forged-replay suite (ABI.md 11): 32 forgeries F01-F32 and 8 honest controls.

Mechanism: capture the real packets of an honest run with forge_harness (FORGE_DUMP), mutate one designated region of the
N-th step call (FORGE_TAMPER), and run again through the production sol::req_step path.  A forgery passes only if
  1. the forged step is refused: either the core itself returns a done packet with record core InvalidPacket and a world
     that is not one generation past the input world (nothing to adopt), or, for a complete replayed triple whose old world
     is self-consistent (F30, F32), the result generation is not the driver's latest honest generation + 1 (the driver's
     freshness rule), and
  2. the whole observation equals the honest observation plus exactly one request.reject trace line: the trusted driver
     kept its last honest world and continuation, performed no host effect for the forged step (no cleanup either), and
     its honest resubmission finished the scenario unchanged (ABI.md 10).
An honest control passes if re-submitting the captured packets unchanged yields exactly the unforged observation.
Usage: sh dev_build.sh && sh forge_build.sh && python3 forge_test.py [ID ...]
"""
import json, os, subprocess, sys, tempfile
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import common

BIN = os.path.join(HERE, 'work', 'dev', 'forge_harness')
HDR = 32
_cache = {}


def case(id):
    for l in common.load_group(id[0]):
        if l['id'] == id:
            return common.strip_case(l)
    raise KeyError(id)


def run(c, tamper=None, dump=None, *, harness=BIN):
    env = dict(os.environ)
    env.pop('FORGE_TAMPER', None); env.pop('FORGE_DUMP', None)
    if tamper is not None: env['FORGE_TAMPER'] = json.dumps(tamper)
    if dump: env['FORGE_DUMP'] = dump
    p = subprocess.run([harness], input=json.dumps(c), capture_output=True, text=True, env=env)
    lines = p.stdout.strip().splitlines()
    return json.loads(lines[-1]) if lines else {'crash': p.stderr[-300:]}


def capture(id, *, harness=BIN):
    key = (harness, id)
    if key not in _cache:
        f = tempfile.mktemp(suffix='.jsonl')
        obs = run(case(id), dump=f, harness=harness)
        recs = [json.loads(l) for l in open(f)]
        os.remove(f)
        packets = {d['n']: d for d in recs if d['kind'] == 'step'}
        packets[0] = next(d for d in recs if d['kind'] == 'begin')
        _cache[key] = (obs, packets)
    return _cache[key]


def well_formed(pk):
    """verdict mode: a core packet with a header, a non-zero kind, at least one section and a section table that fits."""
    if len(pk) < HDR or pk[2] == 0: return False
    n = pk[24]
    if n < 1 or HDR + n > len(pk): return False
    lens = pk[HDR:HDR + n]
    return all(l >= 0 for l in lens) and HDR + n + sum(lens) <= len(pk)


def secs(pk):
    n = pk[24]; at = HDR + n; r = []
    for l in pk[HDR:HDR + n]:
        r.append(pk[at:at + l]); at += l
    return r


def build(pk, sections):
    n = pk[24]
    out = list(pk[:HDR]) + [len(s) for s in sections]
    for s in sections: out += s
    out[25] = len(out)
    return out


def sec_edit(pk, i, fn):
    s = secs(pk); s = [list(x) for x in s]; s[i] = fn(s[i]); return build(pk, s)


def extra_pool_word(pool):
    return pool + [0]


def missing_pool_word(pool):
    assert len(pool) > 2, 'expected a packed data word'
    return pool[:-1]


def comp_pool(fn):
    def m(w, p, c, steps, n):
        return w, p, sec_edit(c, 0, fn)
    return m


def begin_pool(id, fn, *, harness=BIN, verdict=False):
    """verdict: return 'refused', 'accepted' (a well-formed non-reject core packet) or 'other' instead of a bool."""
    _, packets = capture(id, harness=harness)
    begin = packets[0]
    forged = sec_edit(begin['comp'], 0, fn)
    with tempfile.TemporaryDirectory() as tmp:
        dump = os.path.join(tmp, 'begin.jsonl')
        obs = run(case(id), {'at': 0, 'input': forged}, dump=dump, harness=harness)
        with open(dump) as f:
            out = json.loads(next(f))['out']
    if len(out) < 4 or out[0] != 1381057073:
        return 'other' if verdict else False
    world_end = 4 + out[2]
    pk = out[world_end:]
    if verdict:
        if not well_formed(pk): return 'other'
        if pk[2] == 3 and secs(pk)[1] == [5, 0]: return 'refused'
        return 'accepted'
    return (out[4:world_end] == begin['world'] and len(pk) >= HDR and pk[2] == 3
            and secs(pk)[1] == [5, 0] and rejected(obs) and n_rejects(obs) == 1)


def req_rows(obs, rid=None):
    return obs.get('requests', [])


def rejected(obs):
    rs = obs.get('requests', [])
    return any(r.get('terminal') == 'core' and json.dumps(r.get('value')).find('InvalidPacket') >= 0 for r in rs)


def classify(obs, base):
    rs = obs.get('requests')
    if rs is None:
        return 'DRIVER-FAILURE(%s)' % json.dumps(obs)[:80]
    if rejected(obs):
        return 'InvalidPacket'
    codes = [r['value'].get('code') for r in rs if r.get('terminal') == 'core' and isinstance(r.get('value'), dict)]
    if codes:
        return 'ACCEPTED-as-%s' % ','.join(str(c) for c in codes)
    return 'ACCEPTED(%s)' % ('result-unchanged' if obs == base else 'result-altered')


def tamper_run(id, n, mut):
    """mut(world, prev, comp) -> (world, prev, comp) on copies of the honest arguments of step n."""
    obs, steps = capture(id)
    d = steps[n]
    w, p, c = mut(list(d['world']), list(d['prev']), list(d['comp']), steps, n)
    return run(case(id), {'at': n, 'world': w, 'prev': p, 'comp': c})


def core_refused(id, n, mut, strict=False, *, harness=BIN, verdict=False):
    """re-run the forgery with a dump and check what sol::req_step itself returned for the forged step n.
    strict: only an explicit reject record (5, 0) counts, not a malformed core output.
    verdict: return 'refused', 'accepted' (a well-formed non-reject core packet) or 'other' instead."""
    obs, steps = capture(id, harness=harness)
    d = steps[n]
    w, p, c = mut(list(d['world']), list(d['prev']), list(d['comp']), steps, n)
    f = tempfile.mktemp(suffix='.jsonl')
    run(case(id), {'at': n, 'world': w, 'prev': p, 'comp': c}, dump=f, harness=harness)
    out = [json.loads(l) for l in open(f) if json.loads(l)['n'] == n and json.loads(l)['kind'] == 'step'][0]['out']
    os.remove(f)
    if len(out) < 4 or out[0] != 1381057073: return 'other' if verdict else (False if strict else 'core')
    ww = out[2]; world, pk = out[4:4 + ww], out[4 + ww:]
    if verdict:
        if not well_formed(pk): return 'other'
        return 'refused' if pk[2] == 3 and secs(pk)[1] == [5, 0] else 'accepted'
    if ww >= 3 and world[2] == w[2] + 1:
        # a complete, self-consistent old triple (world included) is fresh to the core; only the driver's freshness
        # rule (result generation must be its latest honest generation + 1) can refuse it
        return 'stale' if world[2] != d['world'][2] + 1 else False
    if len(pk) < HDR or pk[2] != 3: return False
    rec = secs(pk)[1]
    if strict:
        return 'core' if rec == [5, 0] else False
    return 'core' if len(rec) >= 2 and rec[0] == 5 and rec[1] == 0 else False


def without_rejects(obs):
    o = dict(obs); o['trace'] = [t for t in obs.get('trace', []) if t.get('c') != 'request.reject']; return o


def n_rejects(obs):
    return sum(1 for t in obs.get('trace', []) if t.get('c') == 'request.reject')


def hdr(i, f):
    """edit header word i in both the previous await and the echoing completion; f(old)->new."""
    def m(w, p, c, steps, n):
        p[i] = f(p[i]); c[i] = f(c[i]); return w, p, c
    return m


def prev_only(i, f):
    def m(w, p, c, steps, n):
        p[i] = f(p[i]); return w, p, c
    return m


def prev_sec(si, fn):
    def m(w, p, c, steps, n): return w, sec_edit(p, si, fn), c
    return m


def state_word(k, v):
    def m(w, p, c, steps, n): return w, sec_edit(p, 2, lambda s: [v if j == 16 + k else x for j, x in enumerate(s)]), c
    return m


def comp_split_string(rec, pool=None):
    def m(w, p, c, steps, n):
        sections = secs(c)
        # default U+00E9, followed by packed-word padding
        sections[0] = list(pool) if pool is not None else [1, 2, 0xC3A90000000000]
        sections[1] = rec
        return w, p, build(c, sections)
    return m


def world_open(i, v):
    def m(w, p, c, steps, n):
        ni, na, no, np_ = w[5], w[6], w[7], w[8]
        o = 16 + np_ + 2 * ni + 10 * na
        assert no > 0, 'no opening row at this step'
        w[o + i] = v; return w, p, c
    return m


def replay(src):
    """feed the complete captured (world, prev, completion) triple of step src at the target step."""
    def m(w, p, c, steps, n):
        d = steps[src]; return list(d['world']), list(d['prev']), list(d['comp'])
    return m


def stale_world(src):
    def m(w, p, c, steps, n): return list(steps[src]['world']), p, c
    return m


def other_comp(src):
    def m(w, p, c, steps, n): return w, p, list(steps[src]['comp'])
    return m


def comp_sec_from(si, src):
    def m(w, p, c, steps, n):
        o = secs(steps[src]['comp'])[si]
        return w, p, sec_edit(c, si, lambda s: list(o))
    return m


def flag(bit):
    def m(w, p, c, steps, n):
        p[22] |= bit; c[22] |= bit; return w, p, c
    return m


def comp_only(i, f):
    def m(w, p, c, steps, n):
        c[i] = f(c[i]); return w, p, c
    return m


def comp_rec(k, v):
    def m(w, p, c, steps, n): return w, p, sec_edit(c, 1, lambda s: [v if j == k else x for j, x in enumerate(s)])
    return m


def comp_rec_cut(k):
    def m(w, p, c, steps, n): return w, p, sec_edit(c, 1, lambda s: s[:-k])
    return m


def comp_set_rec(v):
    def m(w, p, c, steps, n): return w, p, sec_edit(c, 1, lambda s: list(v))
    return m


def comp_sec_set(i, v):
    def m(w, p, c, steps, n): return w, p, sec_edit(c, i, lambda s: list(v))
    return m


def comp_sec7():
    def m(w, p, c, steps, n):
        s = [list(x) for x in secs(c)] + [[]]
        out = list(c[:HDR]) + [len(x) for x in s]
        for x in s: out += x
        out[24] = 7; out[25] = len(out)
        return w, p, out
    return m


def wdig(a, skip):
    """req_world::dig: the two-modulus polynomial hash the world seal (word 11) uses."""
    h1, h2 = 1, 7
    for i, x in enumerate(a):
        if i == skip: continue
        u, sg = (x, 0) if x >= 0 else (-(x + 1), 1)
        h1 = (h1 * 1000003 + 2 * (u % 2147483629) + sg) % 2147483629
        h2 = (h2 * 999983 + 2 * (u % 2147483587) + sg) % 2147483587
    return ((h1 * 1000003 + len(a)) % 2147483629) * 2147483648 + h2 + 1


def world_budget(k, v):
    """set world word k (9 external calls used, 10 cleanup credits) to v and reseal, as a host that ran long would."""
    def m(w, p, c, steps, n):
        w[k] = v; w[11] = wdig(w, 11); return w, p, c
    return m


def child_cut():
    def m(w, p, c, steps, n): return w, p, sec_edit(c, 5, lambda s: s[:-1])
    return m


# (id, case, step, mutation, region)
F = [
    ('F01', 'S01', 3, hdr(1, lambda v: v + 1), 'header 1 version'),
    ('F02', 'S01', 3, prev_only(2, lambda v: 0), 'prev header 2 kind await->input'),
    ('F03', 'S01', 3, hdr(5, lambda v: 9), 'header 5 request id'),
    ('F04', 'S01', 3, hdr(4, lambda v: 2), 'header 4 instance id'),
    ('F05', 'S01', 3, hdr(6, lambda v: v + 1), 'header 6 sequence'),
    ('F06', 'S01', 3, hdr(3, lambda v: 2), 'header 3 operation'),
    ('F07', 'S01', 3, hdr(10, lambda v: 3), 'header 10 call kind 1->3'),
    ('F08', 'S01', 3, prev_only(2, lambda v: 3), 'prev header 2 kind await->done'),
    ('F09', 'G06', 10, prev_sec(4, lambda s: [2 if j == 1 else x for j, x in enumerate(s)]), 'section 4 word 1 (log op)'),
    ('F10', 'G06', 10, prev_sec(4, lambda s: [x + 5 if j == 2 else x for j, x in enumerate(s)]), 'section 4 word 2 (log seq)'),
    ('F11', 'G06', 10, prev_sec(4, lambda s: s[3:]), 'section 4 first 3 words removed'),
    ('F12', 'G06', 10, prev_sec(4, lambda s: s + s[:3]), 'section 4 3 extra words appended'),
    ('F13', 'G06', 10, prev_sec(4, lambda s: s + [7]), 'section 4 one trailing word'),
    ('F14', 'G06', 10, hdr(16, lambda v: 5), 'header 16 item start seq'),
    ('F15', 'G06', 10, hdr(17, lambda v: 1), 'header 17 dropped'),
    ('F16', 'G06', 10, hdr(9, lambda v: v + 1), 'header 9 phase'),
    ('F17', 'S01', 3, hdr(13, lambda v: 2), 'header 13 child op'),
    ('F18', 'S01', 3, hdr(14, lambda v: v + 1), 'header 14 child seq'),
    ('F19', 'S01', 5, hdr(21, lambda v: 3), 'header 21 source rows accepted'),
    ('F20', 'S01', 5, comp_sec_from(5, 3), 'child completion of another child call (step 3)'),
    ('F21', 'O01', 13, world_open(1, 7), 'world opening row word 1 (key)'),
    ('F22', 'O01', 13, world_open(2, 8), 'world opening row word 2 (owner)'),
    ('F23', 'G06', 12, state_word(12, 5), 'state frame word 12 copy token'),
    ('F24', 'G06', 12, flag(16), 'header 22 flag 16 create returned'),
    ('F25', 'G06', 12, flag(32), 'header 22 flag 32 rm completed'),
    ('F26', 'G06', 12, state_word(9, 77), 'state frame word 9 timer'),
    ('F27', 'C02', 12, state_word(13, 3), 'state frame word 13 callback token'),
    ('F28', 'C02', 12, state_word(14, 9), 'state frame word 14 cancel reason ref'),
    ('F29', 'O01', 13, stale_world(10), 'world of step 10 with current packets'),
    ('F30', 'O01', 13, replay(12), 'full replay of step 12 triple (ticket consumed)'),
    ('F31', 'I08', 29, other_comp(20), "other instance's completion (step 20)"),
    ('F32', 'I08', 30, replay(27), 'replay of finished request 2 (owner released)'),
    ('F33', 'L02', 12, comp_only(18, lambda v: v + 1), 'completion header 18 differs from the await'),
    ('F34', 'L02', 12, comp_sec7(), 'completion with a seventh section'),
    ('F35', 'L02', 12, comp_rec(2, 12), 'list item slot with an unknown tag'),
    ('F36', 'L02', 12, comp_rec(4, 99), 'list item string slot past the reply pool'),
    ('F37', 'L02', 10, comp_rec(3, 1), 'ref slot with a non-zero second word'),
    ('F38', 'L02', 18, comp_rec(1, 7), 'bound session id slot tagged array'),
    ('F39', 'L02', 15, comp_rec(1, 2), 'excluded reply word outside 0/1'),
    ('F40', 'S01', 3, child_cut(), 'child completion one word short'),
    ('F41', 'L02', 14, comp_rec(5, 1), 'stored item record word changed (record digest)'),
    ('F42', 'L02', 14, comp_rec_cut(1), 'stored item record one word short'),
    ('F43', 'L02', 20, comp_rec(6, 3), 'result chain element word changed (chain digest)'),
    ('F44', 'L02', 20, comp_rec(1, 2), 'result chain count differs from the item count'),
    ('F45', 'L02', 21, comp_rec(3, 1), 'stored titles word changed (T digest)'),
    ('F46', 'L02', 13, comp_set_rec([1]), 'storage op refused'),
    ('F47', 'L02', 19, comp_set_rec([0, 0]), 'chain append handle 0'),
    ('F48', 'L02', 14, comp_sec_set(0, [65]), 'storage reply with a pool section'),
    ('F49', 'L02', 5, world_budget(9, 1000001), 'world external calls past the 1,000,000 budget (resealed)'),
    ('F50', 'O01', 12, world_budget(10, 999999), 'world cleanup credits plus calls past the budget (resealed)'),
    ('F51', 'L02', 17, comp_set_rec([99]), 'session re-read (call 17) record changed, section 5 honest'),
    ('F52', 'L02', 9, comp_sec_set(0, []), 'folder success slot outside the empty completion pool'),
    ('F53', 'L02', 9, comp_split_string([1] + [0] * 6 + [3] + [0] * 5 + [6, 1, 1] + [0] * 3),
     'C4 folder error message starts inside a WTF-8 form'),
    ('F54', 'O01', 15, comp_split_string([0, 6, 1, 1] + [0] * 7),
     'C4 call 33 title starts inside a WTF-8 form'),
    ('F55', 'O01', 15, comp_split_string([0] + [0] * 9 + [1] + [0] * 3 + [6, 1, 1] + [0] * 12),
     'C4 call 33 turn.reply starts inside a WTF-8 form'),
    ('F56', 'O01', 15, comp_split_string([0] + [0] * 6 + [6, 1, 1] + [0]),
     'C4 call 33 lastPlan starts inside a WTF-8 form'),
    ('F57', 'O01', 16, comp_split_string([0, 6, 1, 1]),
     'C4 call 32 sessionId starts inside a WTF-8 form'),
    ('F58', 'O01', 15, comp_split_string([0] + [0] * 6 + [8, 0, 0] + [0]),
     'C4 call 33 lastPlan with a tag the host does not read'),
    ('F59', 'O01', 16, comp_split_string([0, 8, 0, 0]),
     'C4 call 32 sessionId with a tag the host does not read'),
    # lone high, 0xFF separator, lone low: a valid pool, but a slot that covers the separator is refused (C3)
    ('F60', 'O01', 15, comp_split_string([0] + [0] * 6 + [6, 0, 7] + [0], [2, 7, 0xEDA080FFEDB080]),
     'C3 call 33 lastPlan covers the 0xFF separator'),
    ('F61', 'O01', 16, comp_split_string([0, 6, 3, 1], [2, 7, 0xEDA080FFEDB080]),
     'C3 call 32 sessionId is the 0xFF separator alone'),
    ('F62', 'O01', 16, comp_pool(extra_pool_word), 'completion pool has an extra zero word'),
    ('F63', 'O01', 16, comp_pool(missing_pool_word), 'completion pool is missing a word'),
]
# the call budget (ABI.md 10): (id, case, step, world word 9 set to, effects the refused step must end with)
BUDGET = [
    ('B01', 'L02', 5, 1000000, ['request.done']),
    ('B02', 'L02', 10, 999999, ['timer.clear', 'request.done']),
    ('B03', 'O01', 12, 999999, ['timer.clear', 'opening.delete', 'request.done']),
    ('B04', 'O01', 12, 999998, ['timer.clear', 'opening.delete', 'request.done']),
    ('B05', 'G06', 15, 999999, ['timer.clear', 'rm', 'opening.delete', 'request.done']),
    ('B06', 'R03', 15, 999999, ['listener.remove', 'rm', 'opening.delete', 'request.done']),
]
CONTROLS = [('H01', 'S01', 'sources', 3), ('H02', 'L02', 'list', 8), ('H03', 'O01', 'new open', 13),
            ('H04', 'O02', 'existing open', 8), ('H05', 'G06', 'Grok verify', 12), ('H06', 'G07', 'rollback', 12),
            ('H07', 'C02', 'cancellation', 10), ('H08', 'I05', 'two interleaved', 14)]


def main():
    want = set(sys.argv[1:])
    rows = []
    for id, cs, n, mut, region in F:
        if want and id not in want: continue
        try:
            obs = tamper_run(cs, n, mut)
            base = capture(cs)[0]
            core = core_refused(cs, n, mut, strict=id in ('F62', 'F63'))
            same = without_rejects(obs) == base and n_rejects(obs) == 1
            ok = bool(core) and same
            if ok: got = 'InvalidPacket' if core == 'core' else 'InvalidPacket(driver-stale)'
            else: got = 'core-accepted' if not core else 'InvalidPacket+different-effects(%d rejects)' % n_rejects(obs)
        except Exception as e:
            got, ok = 'ERROR %s' % e, False
        rows.append((id, 'InvalidPacket', got, ok, '%s step %d: %s' % (cs, n, region)))
    for id, fn, region in [('F64', extra_pool_word, 'extra zero word'), ('F65', missing_pool_word, 'missing word')]:
        if want and id not in want: continue
        try:
            ok = begin_pool('O01', fn)
            got = 'InvalidPacket' if ok else 'core did not return InvalidPacket with unchanged world'
        except Exception as e:
            got, ok = 'ERROR %s' % e, False
        rows.append((id, 'InvalidPacket', got, ok, 'O01 begin: pool has a %s' % region))
    for id, cs, n, v, tail in BUDGET:
        if want and id not in want: continue
        try:
            obs = tamper_run(cs, n, world_budget(9, v))
            rs = obs.get('requests', [])
            codes = [r['value'].get('code') for r in rs if r.get('terminal') == 'core' and isinstance(r.get('value'), dict)]
            tr = [t.get('c') for t in obs.get('trace', []) if t.get('c') != 'prim']
            ok = codes == ['InputTooLarge'] and n_rejects(obs) == 0 and tr[-len(tail):] == tail
            got = 'InputTooLarge+cleanup' if ok else 'codes=%s tail=%s' % (codes, tr[-len(tail):])
        except Exception as e:
            got, ok = 'ERROR %s' % e, False
        rows.append((id, 'InputTooLarge+cleanup', got, ok, '%s step %d: calls used %d' % (cs, n, v)))
    # C2 recheck: the child completion in section 5 padded with unreferenced pool bytes. P1: the child is exactly at the
    # 16,777,216-word logical limit, so the parent is over it and must be refused (one request.reject, then the honest
    # resubmission). P2: the parent is exactly at the limit and must be accepted with the honest result.
    for id, mode in [('P1', 0), ('P2', 1)]:
        if want and id not in want: continue
        try:
            base = capture('S01')[0]
            obs = run(case('S01'), {'at': 3, 'comp5_fill': mode})
            if mode == 0:
                ok = without_rejects(obs) == base and n_rejects(obs) == 1
                exp = 'InvalidPacket'
            else:
                ok = obs == base and n_rejects(obs) == 0
                exp = 'identical'
            got = exp if ok else 'rejects=%d same=%s' % (n_rejects(obs), without_rejects(obs) == base)
        except Exception as e:
            exp, got, ok = 'pass', 'ERROR %s' % e, False
        rows.append((id, exp, got, ok, 'S01 step 3: child completion padded, %s at the logical limit' % ('child' if mode == 0 else 'parent')))
    for id, cs, name, n in CONTROLS:
        if want and id not in want: continue
        try:
            base, steps = capture(cs)
            plain = run(case(cs))
            obs = tamper_run(cs, n, lambda w, p, c, s, n: (w, p, c))
            ok = obs == plain and base == plain and not rejected(plain) and n_rejects(plain) == 0
            got = 'identical' if ok else 'DIFFERENT'
        except Exception as e:
            got, ok = 'ERROR %s' % e, False
        rows.append((id, 'identical', got, ok, '%s step %d: honest %s' % (cs, n, name)))
    for id, exp, got, ok, note in rows:
        print('%s expected=%s got=%s %s  [%s]' % (id, exp, got, 'pass' if ok else 'FAIL', note))
    npass = sum(1 for r in rows if r[3])
    print('summary: %d/%d behave as required (%d forgeries accepted or mishandled)' % (
        npass, len(rows), sum(1 for r in rows if not r[3] and r[0].startswith('F'))))
    sys.exit(0 if npass == len(rows) else 1)


if __name__ == '__main__':
    main()
