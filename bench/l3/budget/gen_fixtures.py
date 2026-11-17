"""Writes the 140 fixed L3-10 fixtures (fixtures/<G>.json), ten per group A..N, per docs/REQUEST_BUDGET_SPEC.md section 10.
Each fixture: {id, name, rule, expect, kd?, world, instances?, script}. Format and expectation DSL: SCENARIO.md.
Run: python3 gen_fixtures.py   (deterministic; then check_oracle.py in a later milestone).
Domain rules: no dependency throws (spec 0.5), no per-call `now` overrides, and none of the excluded 15.1 rows 10, 16, 26, 27."""
import sys
sys.dont_write_bytecode = True
import datetime
import json
import struct

from common import FIXTURES, GROUPS

T0 = 1700000000000
MIN = 60000


def iso(ms):
    d = datetime.datetime(1970, 1, 1) + datetime.timedelta(milliseconds=ms)
    return d.strftime('%Y-%m-%dT%H:%M:%S') + f'.{ms % 1000:03d}Z'


ISO0 = iso(T0)
U = {'undef': 1}
NEG0 = {'num': '8000000000000000'}


def num(x): return {'num': struct.pack('>d', float(x)).hex()}
def u16(*units): return {'u16': list(units)}
def O(**kw): return {'obj': [[k, v] for k, v in kw.items()]}
def A(*items): return {'arr': list(items)}
def SESS(id): return {'sess': id}
def UU(n): return f'00000000-0000-4000-8000-{n:012x}'
def BID(n): return f'11111111-1111-4111-8111-{n:012x}'


UNIT = {'timeMinutes': 120, 'delegations': 24, 'commands': 3, 'reviews': 2, 'checks': 1}
def unit(**kw): return {**UNIT, **kw}
def lim(mult=1, u=None, **kw): return {**{k: v * mult for k, v in (u or UNIT).items()}, **kw}
def used(**kw): return {**{'activeMs': 0, 'delegations': 0, 'commands': 0, 'reviews': 0, 'checks': 0, 'tools': 0, 'failures': 0, 'retries': 0}, **kw}


def budget(n=1, state='open', origin='user', limits=None, use=None, unlimited=False, reason='commands', how='quiet', ended=True,
           activeSince=None, card=None, autoContinue=None, follows=None, delegations=()):
    """A persisted budget in source property order: id, openedAt, origin, state, limits, [unlimited], used, [activeSince], [halt], [ended], ..."""
    b = {'id': BID(n), 'openedAt': ISO0, 'origin': origin, 'state': state, 'limits': {**UNIT, **(limits or {})}}
    if unlimited: b['unlimited'] = True
    b['used'] = used(**(use or {}))
    if activeSince is not None: b['activeSince'] = activeSince
    if state == 'halted': b['halt'] = {'reason': reason, 'at': ISO0, 'stoppedDelegationIds': list(delegations)}
    if state == 'closed' and ended: b['ended'] = {'how': how, 'at': ISO0}
    if card is not None: b['card'] = card
    if autoContinue is not None: b['autoContinue'] = autoContinue
    if follows is not None: b['follows'] = follows
    return b


def sess(id='s1', b=None):
    d = {'id': id}
    if b is not None: d['budget'] = b
    return d


def ref(sid='s1', n=1): return {'sessionId': sid, 'budgetId': BID(n)}
def uref(sid='s1', n=1): return {'sessionId': sid, 'budgetId': UU(n)}
def W(sessions, u=None, **kw): return {'sessions': sessions, 'unit': u or UNIT, **kw}
def deps(run=None, quiet=None, release=None):
    d = {}
    if run is not None: d['runningDelegationIds'] = run
    if quiet is not None: d['isQuiet'] = quiet
    if release is not None: d['releaseHeld'] = release
    return {'deps': d}


class Script:
    def __init__(self): self.steps, self.n = [], 0
    def call(self, op, *args, i=None):
        self.n += 1
        step = {'ev': 'call', 'req': f'r{self.n}', 'op': op, 'args': list(args)}
        if i: step['i'] = i
        self.steps.append(step)
        return step['req']
    def adv(self, ms): self.steps.append({'ev': 'advance', 'ms': ms})
    def settime(self, now): self.steps.append({'ev': 'setTime', 'now': now})
    def timer(self, token, captured=False):
        step = {'ev': 'deliverTimer', 'token': token}
        if captured: step['captured'] = True
        self.steps.append(step)
    def setunit(self, u): self.steps.append({'ev': 'setUnit', 'unit': u})
    def put(self, session): self.steps.append({'ev': 'putSession', 'session': session})
    def remove(self, id): self.steps.append({'ev': 'removeSession', 'id': id})
    def setdep(self, dep, replies): self.steps.append({'ev': 'setDepReply', 'dep': dep, 'replies': replies})
    def snap(self, label): self.steps.append({'ev': 'snapshot', 'label': label})


# ---- expectation builders (DSL: SCENARIO.md) ---------------------------------------------------------------------------------
NOV = object()
def R(req, value=NOV, terminal='ok', keys=None):
    d = {'req': req, 'terminal': terminal}
    if value is not NOV: d['value'] = value
    if keys is not None: d['valueKeys'] = keys
    return d
def SV(id, path, value): return {'id': id, 'path': path, 'value': value}
def SA(id, path): return {'id': id, 'path': path, 'absent': True}
def SK(id, path, keys): return {'id': id, 'path': path, 'keys': keys}
def E(reqs=(), counts=None, has=None, absent=None, order=None, session=None, clocks=None, timers=None):
    e = {}
    if reqs: e['requests'] = list(reqs)
    for k, v in (('counts', counts), ('has', has), ('absent', absent), ('order', order), ('session', session), ('clocks', clocks), ('timers', timers)):
        if v is not None: e[k] = v
    assert e, 'a fixture needs at least one expectation'
    return e


OKT = {'ok': True}
def NO(reason): return {'ok': False, 'reason': reason}
HALT_ORDER = [['runningDelegationIds', 'cancelRecovery'], ['cancelRecovery', 'stopMain'], ['stopMain', 'stopAttachedTurns'], ['stopAttachedTurns', 'holdQueued'],
              ['holdQueued', 'save'], ['save', 'changed'], ['changed', 'announce']]
KEYS_BASE = ['id', 'openedAt', 'origin', 'state', 'limits', 'used']

FIX = {g: [] for g in GROUPS}


def fx(fid, name, rule, kd=None):
    """Registers the fixture built by the decorated function, which returns (world, script, expect[, instances])."""
    def deco(fn):
        out = fn()
        f = {'id': fid, 'name': name, 'rule': rule, 'expect': out[2]}
        if kd: f['kd'] = kd
        f['world'] = out[0]
        if len(out) > 3 and out[3]: f['instances'] = out[3]
        f['script'] = out[1].steps
        FIX[fid[0]].append(f)
        return fn
    return deco


# ---- texts (copied from B:68-143) ---------------------------------------------------------------------------------------------
COUNTER = {'ko': {'time': '시간', 'delegations': '위임', 'commands': '빌드·명령', 'reviews': '감사', 'checks': '실제 확인'},
           'en': {'time': 'time', 'delegations': 'delegations', 'commands': 'commands', 'reviews': 'reviews', 'checks': 'checks'}}
NOLIMIT = {'ko': '제한 없음', 'en': 'no limit'}
REFUSAL = {'ko': lambda c, a: f'예산 한도({c} {a})에 닿아 사용자가 늘리거나 끝내기를 고를 때까지 멈춘다. 새 위임·명령을 시도하지 않고 지금까지 결과와 확인하지 못한 것만 짧게 보고한다.',
           'en': lambda c, a: f'The request budget limit ({c} {a}) was reached, so work is paused until the user extends it or ends it. Do not try new delegations or commands; briefly report the results so far and what you could not verify.'}
CONTINUE = {'ko': lambda c, a: f'사용자가 예산 한도({c} {a})를 늘렸다. 앞서 끊긴 도구 호출은 사용자 중단이 아니라 이 예산 한도에 닿아 앱이 멈춘 것이다. 멈춘 지점에서 이어 간다.',
            'en': lambda c, a: f'The user raised the request budget limit ({c} {a}). The tool call that was cut off earlier was stopped by the app at this budget limit, not by the user. Continue from where you stopped.'}
CARD_MISSING = {'ko': '위임 전에 work_card 도구로 산출물과 완료 조건을 먼저 적는다.', 'en': 'Before delegating, write the deliverable and the done conditions with the work_card tool.'}
ERR = {'ko': {'missing': '지금 예산을 찾을 수 없어 작업 카드를 저장하지 못했다.', 'deliverable': 'deliverable 은 1~300자여야 한다.', 'done': 'done 은 각 300자 이하 1~5줄이어야 한다.',
              'targets': 'targets 의 수는 0 이상 정수여야 하고 note 는 300자 이하여야 한다.', 'verify': 'verify 는 label 1~300자와 status(planned·done·skipped)를 가진 0~5개 항목이어야 한다.',
              'conflicts': 'conflicts 는 각 300자 이하 0~5줄이어야 한다.', 'state': 'state 는 working·done·waiting-user·blocked 중 하나여야 한다.'},
       'en': {'missing': 'The current budget was not found, so the work card was not saved.', 'deliverable': 'deliverable must be 1–300 characters.',
              'done': 'done must be 1–5 lines of at most 300 characters each.', 'targets': 'targets counts must be non-negative integers and note at most 300 characters.',
              'verify': 'verify must be 0–5 items with a label of 1–300 characters and a status (planned, done or skipped).',
              'conflicts': 'conflicts must be 0–5 lines of at most 300 characters each.', 'state': 'state must be one of working, done, waiting-user or blocked.'}}
REASONS = ['time', 'delegations', 'commands', 'reviews', 'checks']


def limit_text(b, limit, lang='ko'): return NOLIMIT[lang] if b.get('unlimited') else str(limit)
def amount_text(reason, b, lang='ko'):
    if reason == 'time': return f"{b['used']['activeMs'] // MIN}/{limit_text(b, b['limits']['timeMinutes'], lang)}"
    return f"{b['used'][reason]}/{limit_text(b, b['limits'][reason], lang)}"
def refusal_text(reason, b, lang='ko'): return REFUSAL[lang](COUNTER[lang][reason], amount_text(reason, b, lang))
def continue_text(reason, b, lang='ko'): return CONTINUE[lang](COUNTER[lang][reason], amount_text(reason, b, lang))
def wire(reason, b):
    used_v = b['used']['activeMs'] // MIN if reason == 'time' else b['used'][reason]
    return {'key': 'budget.refusal.' + reason, 'args': {'used': used_v, 'limit': b['limits']['timeMinutes'] if reason == 'time' else b['limits'][reason]}}


# ============================== A: attach/open ===============================================================================
@fx('A01', 'missing-budget-opens-default', 'B:180-185,205-225; spec 4.2 OPEN')
def _():
    s = Script(); r = s.call('attach', SESS('s1'), 'user')
    return (W([sess('s1')]), s, E([R(r, {'sessionId': 's1', 'budgetId': UU(1)})], counts={'unit': 1, 'uuid': 1, 'now': 1, 'changed': 1, 'save': 0},
            order=[['unit', 'uuid'], ['uuid', 'now'], ['now', 'changed']],
            session=[SV('s1', 'budget.id', UU(1)), SV('s1', 'budget.state', 'open'), SV('s1', 'budget.origin', 'user'), SV('s1', 'budget.openedAt', ISO0),
                     SV('s1', 'budget.limits', UNIT), SK('s1', 'budget', KEYS_BASE)]))


@fx('A02', 'existing-open-returned-unchanged', 'B:182-183; spec 4.2 attach')
def _():
    s = Script(); r = s.call('attach', SESS('s1'), 'direct', 8)
    return (W([sess('s1', budget(1))]), s, E([R(r, ref('s1', 1))], counts={'unit': 0, 'uuid': 0, 'now': 0, 'changed': 0, 'findSession': 0},
            session=[SV('s1', 'budget.origin', 'user'), SV('s1', 'budget.limits', UNIT)]))


@fx('A03', 'existing-halted-returned-unchanged', 'B:182-183; spec 4.2 attach (halted is not closed)')
def _():
    s = Script(); r = s.call('attach', SESS('s1'), 'user')
    return (W([sess('s1', budget(1, 'halted'))]), s, E([R(r, ref('s1', 1))], counts={'uuid': 0, 'changed': 0, 'save': 0},
            session=[SV('s1', 'budget.state', 'halted'), SV('s1', 'budget.halt.reason', 'commands')]))


@fx('A04', 'closed-with-live-clock-drops-it', 'B:207-208,374-378; spec 4.2 OPEN step 1, 5.6 DROP')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1')
    s.put(sess('s1', budget(1, 'closed')))
    r = s.call('attach', SESS('s1'), 'user')
    return (W([sess('s1', budget(1))]), s, E([R(r, uref('s1', 1))], counts={'timer.set': 1, 'timer.clear': 1, 'uuid': 1},
            order=[['timer.set', 'timer.clear'], ['timer.clear', 'unit']], clocks={'count': 0}, timers=0,
            session=[SV('s1', 'budget.id', UU(1)), SV('s1', 'budget.state', 'open')]))


@fx('A05', 'configured-default-size-one', 'B:209-217; spec 4.2 OPEN steps 2-4')
def _():
    u = {'timeMinutes': 30, 'delegations': 5, 'commands': 2, 'reviews': 3, 'checks': 4}
    s = Script(); r = s.call('attach', SESS('s1'), 'user')
    return (W([sess('s1')], u), s, E([R(r, uref('s1', 1))], session=[SV('s1', 'budget.limits', u), SA('s1', 'budget.unlimited')]))


@fx('A06', 'default-unlimited-opens-unlimited', 'B:210,218; spec 4.2 OPEN, K4/#12')
def _():
    u = {**unit(timeMinutes=30, delegations=5), 'defaultUnlimited': True}
    s = Script(); r = s.call('attach', SESS('s1'), 'user')
    return (W([sess('s1')], u), s, E([R(r, uref('s1', 1))], session=[SV('s1', 'budget.unlimited', True), SV('s1', 'budget.limits.timeMinutes', 30),
            SV('s1', 'budget.limits.delegations', 5), SK('s1', 'budget', ['id', 'openedAt', 'origin', 'state', 'limits', 'unlimited', 'used'])]))


@fx('A07', 'explicit-size-one-overrides-unlimited-default', 'B:210; spec 4.2 OPEN step 3')
def _():
    u = {**UNIT, 'defaultUnlimited': True}
    s = Script(); r = s.call('attach', SESS('s1'), 'user', 1)
    return (W([sess('s1')], u), s, E([R(r, uref('s1', 1))], session=[SA('s1', 'budget.unlimited'), SV('s1', 'budget.limits', UNIT)]))


@fx('A08', 'sizes-2-4-8-scale-the-unit', 'B:211,217; spec 4.2 OPEN step 4')
def _():
    s = Script(); rs = [s.call('attach', SESS(f's{i + 1}'), 'user', n) for i, n in enumerate((2, 4, 8))]
    return (W([sess('s1'), sess('s2'), sess('s3')]), s, E([R(r, uref(f's{i + 1}', i + 1)) for i, r in enumerate(rs)],
            session=[SV('s1', 'budget.limits', lim(2)), SV('s2', 'budget.limits', lim(4)), SV('s3', 'budget.limits', lim(8))]))


@fx('A09', 'explicit-unlimited-keeps-unit-limits', 'B:210-218; spec 4.2 OPEN, #12')
def _():
    s = Script(); r = s.call('attach', SESS('s1'), 'schedule', 'unlimited')
    return (W([sess('s1')]), s, E([R(r, uref('s1', 1))], session=[SV('s1', 'budget.unlimited', True), SV('s1', 'budget.limits', UNIT), SV('s1', 'budget.origin', 'schedule')]))


@fx('A10', 'all-origins-and-call-order', 'B:205-225; spec 4.2 OPEN order unit, uuid, now, assign, changed')
def _():
    s = Script(); rs = [s.call('attach', SESS(f's{i + 1}'), o) for i, o in enumerate(('user', 'schedule', 'direct', 'release'))]
    return (W([sess(f's{i}') for i in range(1, 5)]), s, E([R(r, uref(f's{i + 1}', i + 1)) for i, r in enumerate(rs)],
            counts={'unit': 4, 'uuid': 4, 'now': 4, 'changed': 4, 'save': 0}, order=[['unit', 'uuid'], ['uuid', 'now'], ['now', 'changed']],
            session=[SV(f's{i + 1}', 'budget.origin', o) for i, o in enumerate(('user', 'schedule', 'direct', 'release'))] + [SV('s4', 'budget.id', UU(4))]))


# ============================== B: replacement / reopen ======================================================================
@fx('B01', 'halted-user-replacement-follows-and-carries', 'B:191-203; spec 4.2 replaceForUserMessage; 15.1 #2/#3', kd=['KD-2'])
def _():
    s = Script(); r = s.call('replaceForUserMessage', SESS('s1'))
    return (W([sess('s1', budget(1, 'halted'))]), s, E([R(r, uref('s1', 1))], counts={'uuid': 1, 'carryHeld': 1, 'unit': 1, 'save': 0}, order=[['uuid', 'carryHeld']],
            session=[SV('s1', 'budget.follows', BID(1)), SV('s1', 'budget.state', 'open'), SV('s1', 'budget.origin', 'user'), SK('s1', 'budget', KEYS_BASE + ['follows'])]))


@fx('B02', 'halted-schedule-replacement-skips-follows-and-carry', 'B:188-196; spec 4.2; 15.1 #3 (kept)')
def _():
    s = Script(); r = s.call('replaceForUserMessage', SESS('s1'), U, 'schedule')
    return (W([sess('s1', budget(1, 'halted'))]), s, E([R(r, uref('s1', 1))], counts={'uuid': 1, 'carryHeld': 0, 'changed': 1},
            session=[SA('s1', 'budget.follows'), SV('s1', 'budget.origin', 'schedule'), SV('s1', 'budget.state', 'open')]))


@fx('B03', 'direct-and-release-take-the-follows-path', 'B:196; spec 4.2 (only origin===schedule skips)', kd=['KD-2'])
def _():
    s = Script(); r1 = s.call('replaceForUserMessage', SESS('s1'), U, 'direct'); r2 = s.call('replaceForUserMessage', SESS('s2'), U, 'release')
    return (W([sess('s1', budget(1, 'halted')), sess('s2', budget(2, 'halted'))]), s, E([R(r1, uref('s1', 1)), R(r2, uref('s2', 2))], counts={'carryHeld': 2, 'uuid': 2},
            session=[SV('s1', 'budget.follows', BID(1)), SV('s2', 'budget.follows', BID(2)), SV('s1', 'budget.origin', 'direct'), SV('s2', 'budget.origin', 'release')]))


@fx('B04', 'open-budget-is-reused', 'B:202; spec 4.2 (non-halted delegates to attach)')
def _():
    s = Script(); r1 = s.call('replaceForUserMessage', SESS('s1')); r2 = s.call('replaceForUserMessage', SESS('s1'), 8, 'schedule')
    return (W([sess('s1', budget(1))]), s, E([R(r1, ref('s1', 1)), R(r2, ref('s1', 1))], counts={'unit': 0, 'uuid': 0, 'changed': 0, 'carryHeld': 0},
            session=[SV('s1', 'budget.limits', UNIT)]))


@fx('B05', 'absent-and-closed-fall-back-to-attach', 'B:202,181-184; spec 4.2')
def _():
    s = Script(); r1 = s.call('replaceForUserMessage', SESS('s1')); r2 = s.call('replaceForUserMessage', SESS('s2'), 2, 'direct')
    return (W([sess('s1'), sess('s2', budget(2, 'closed'))]), s, E([R(r1, uref('s1', 1)), R(r2, uref('s2', 2))], counts={'carryHeld': 0, 'uuid': 2},
            session=[SA('s1', 'budget.follows'), SA('s2', 'budget.follows'), SV('s2', 'budget.origin', 'direct'), SV('s2', 'budget.limits', lim(2))]))


@fx('B06', 'quiet-closed-budget-reopens-once', 'B:228-235; spec 4.2 reopen')
def _():
    s = Script(); r1 = s.call('reopen', ref('s1', 1)); r2 = s.call('reopen', ref('s1', 1))
    return (W([sess('s1', budget(1, 'closed', how='quiet'))]), s, E([R(r1, True), R(r2, False)], counts={'changed': 1, 'save': 0, 'now': 0, 'findSession': 2},
            session=[SV('s1', 'budget.state', 'open'), SA('s1', 'budget.ended')]))


@fx('B07', 'user-end-is-not-reopened', 'B:230; spec 4.2 reopen')
def _():
    s = Script(); r = s.call('reopen', ref('s1', 1))
    return (W([sess('s1', budget(1, 'closed', how='user-end'))]), s, E([R(r, False)], counts={'changed': 0},
            session=[SV('s1', 'budget.state', 'closed'), SV('s1', 'budget.ended.how', 'user-end')]))


@fx('B08', 'user-message-open-halted-and-ended-less-are-not-reopened', 'B:230; spec 4.2 reopen')
def _():
    s = Script(); rs = [s.call('reopen', ref(f's{i}', i)) for i in range(1, 5)]
    return (W([sess('s1', budget(1, 'closed', how='user-message')), sess('s2', budget(2)), sess('s3', budget(3, 'halted')), sess('s4', budget(4, 'closed', ended=False))]), s,
            E([R(r, False) for r in rs], counts={'changed': 0}, session=[SV('s1', 'budget.state', 'closed'), SV('s4', 'budget.state', 'closed'), SV('s3', 'budget.state', 'halted')]))


@fx('B09', 'stale-and-missing-references', 'B:169-173,229; spec 4.2 current/reopen')
def _():
    s = Script(); r1 = s.call('reopen', ref('s1', 99)); r2 = s.call('reopen', ref('nope', 1))
    return (W([sess('s1', budget(1, 'closed', how='quiet'))]), s, E([R(r1, False), R(r2, False)], counts={'findSession': 2, 'changed': 0},
            session=[SV('s1', 'budget.state', 'closed')]))


@fx('B10', 'replacement-invalidates-the-old-reference', 'B:187-203,169-178; spec 4.2')
def _():
    s = Script(); s.call('replaceForUserMessage', SESS('s1'))
    r2 = s.call('current', ref('s1', 1)); r3 = s.call('isHalted', ref('s1', 1)); r4 = s.call('isHalted', uref('s1', 1)); r5 = s.call('current', uref('s1', 1))
    return (W([sess('s1', budget(1, 'halted'))]), s, E([R(r2, U), R(r3, False), R(r4, False), R(r5, {'id': UU(1), 'state': 'open', 'follows': BID(1)})],
            session=[SV('s1', 'budget.id', UU(1))]), None)


# ============================== C: charging ==================================================================================
@fx('C01', 'missing-and-stale-ref-do-not-block', 'B:249-250; spec 4.3 charge steps 1-2')
def _():
    s = Script(); r1 = s.call('charge', ref('nope', 1), {'commands': 5}); r2 = s.call('charge', ref('s1', 99), {'commands': 5})
    return (W([sess('s1', budget(1))]), s, E([R(r1, OKT), R(r2, OKT)], counts={'findSession': 2, 'save': 0}, session=[SV('s1', 'budget.used.commands', 0)]))


@fx('C02', 'closed-budget-does-not-block-or-charge', 'B:250; spec 4.3')
def _():
    s = Script(); r = s.call('charge', ref('s1', 1), {'commands': 5, 'retries': 2})
    return (W([sess('s1', budget(1, 'closed'))]), s, E([R(r, OKT)], session=[SV('s1', 'budget.used.commands', 0), SV('s1', 'budget.used.retries', 0)]))


@fx('C03', 'halted-returns-the-halt-reason', 'B:251; spec 4.3 step 3')
def _():
    s = Script(); rs = [s.call('charge', ref(f's{i}', i), {'delegations': 1}) for i in (1, 2, 3)]
    return (W([sess('s1', budget(1, 'halted', reason='commands')), sess('s2', budget(2, 'halted', reason='reviews')), sess('s3', budget(3, 'halted', reason='checks'))]), s,
            E([R(rs[0], NO('commands')), R(rs[1], NO('reviews')), R(rs[2], NO('checks'))], counts={'findSession': 3, 'save': 0, 'changed': 0},
              session=[SV('s1', 'budget.used.delegations', 0)]))


@fx('C04', 'halted-time-reason-is-returned-verbatim', 'B:251; spec 4.3 (time is a valid halt reason)')
def _():
    s = Script(); r1 = s.call('charge', ref('s1', 1), {'commands': 1}); r2 = s.call('charge', ref('s1', 1), {})
    return (W([sess('s1', budget(1, 'halted', reason='time'))]), s, E([R(r1, NO('time')), R(r2, NO('time'))], counts={'runningDelegationIds': 0, 'save': 0},
            session=[SV('s1', 'budget.halt.reason', 'time')]))


@fx('C05', 'equality-to-the-limit-succeeds-then-one-more-halts', 'B:254; spec 4.3 (killer M01)')
def _():
    s = Script(); r1 = s.call('charge', ref('s1', 1), {'commands': 3}); r2 = s.call('charge', ref('s1', 1), {'commands': 1})
    return (W([sess('s1', budget(1))]), s, E([R(r1, OKT), R(r2, NO('commands'))], counts={'findSession': 4, 'runningDelegationIds': 1, 'stopMain': 1, 'save': 1, 'announce': 1},
            session=[SV('s1', 'budget.used.commands', 3), SV('s1', 'budget.state', 'halted'), SV('s1', 'budget.halt.reason', 'commands')]))


@fx('C06', 'first-over-limit-key-in-fixed-order', 'B:252-258; spec 4.3 step 4 (delegations, commands, reviews, checks)')
def _():
    s = Script(); r = s.call('charge', ref('s1', 1), {'checks': 2, 'reviews': 3})
    return (W([sess('s1', budget(1))]), s, E([R(r, NO('reviews'))], session=[SV('s1', 'budget.halt.reason', 'reviews'), SV('s1', 'budget.used.checks', 0), SV('s1', 'budget.used.reviews', 0)]))


@fx('C07', 'rejection-is-atomic-including-retries', 'B:252-260; spec 4.3 (killer M02)')
def _():
    s = Script(); r = s.call('charge', ref('s1', 1), {'delegations': 2, 'commands': 4, 'retries': 3})
    return (W([sess('s1', budget(1))]), s, E([R(r, NO('commands'))], session=[SV('s1', 'budget.used.delegations', 0), SV('s1', 'budget.used.commands', 0),
            SV('s1', 'budget.used.retries', 0), SV('s1', 'budget.state', 'halted')]))


@fx('C08', 'unlimited-charges-without-halting', 'B:254; spec 4.3 (killer M03)')
def _():
    s = Script(); r = s.call('charge', ref('s1', 1), {'commands': 100, 'delegations': 1000, 'retries': 7})
    return (W([sess('s1', budget(1, unlimited=True))]), s, E([R(r, OKT)], counts={'runningDelegationIds': 0, 'save': 0},
            session=[SV('s1', 'budget.used.commands', 100), SV('s1', 'budget.used.delegations', 1000), SV('s1', 'budget.used.retries', 7), SV('s1', 'budget.state', 'open')]))


@fx('C09', 'negative-nullish-and-zero-amounts', 'B:253,259-260; spec 4.3 (?? 0, no lower clamp)')
def _():
    s = Script(); r = s.call('charge', ref('s1', 1), O(commands=-2, reviews=None, checks=0, delegations=U, retries=-1))
    return (W([sess('s1', budget(1, use={'commands': 1}))]), s, E([R(r, OKT)], session=[SV('s1', 'budget.used.commands', -1), SV('s1', 'budget.used.reviews', 0),
            SV('s1', 'budget.used.checks', 0), SV('s1', 'budget.used.delegations', 0), SV('s1', 'budget.used.retries', -1), SV('s1', 'budget.state', 'open')]))


@fx('C10', 'failures-and-tools-count-in-every-state', 'B:264-277; spec 4.3; 15.1 #7')
def _():
    s = Script()
    fs = [s.call('noteFailure', ref(f's{i}', i)) for i in (1, 2, 3)]
    ts = [s.call('noteTool', ref(f's{i}', i), 't1', f'a{i}') for i in (1, 2, 3)]
    return (W([sess('s1', budget(1)), sess('s2', budget(2, 'halted')), sess('s3', budget(3, 'closed'))]), s,
            E([R(r, U) for r in fs + ts], session=[SV(f's{i}', 'budget.used.failures', 1) for i in (1, 2, 3)] + [SV(f's{i}', 'budget.used.tools', 1) for i in (1, 2, 3)]))


# ============================== D: tools =====================================================================================
@fx('D01', 'first-activity-counts', 'B:270-276; spec 4.3 noteTool')
def _():
    s = Script(); r = s.call('noteTool', ref('s1', 1), 't1', 'a1')
    return (W([sess('s1', budget(1))]), s, E([R(r, U)], counts={'findSession': 1}, session=[SV('s1', 'budget.used.tools', 1)]))


@fx('D02', 'duplicate-activity-counts-once', 'B:273; spec 4.3')
def _():
    s = Script(); [s.call('noteTool', ref('s1', 1), 't1', 'a1') for _i in range(2)]
    return (W([sess('s1', budget(1))]), s, E([R('r2', U)], session=[SV('s1', 'budget.used.tools', 1)]))


@fx('D03', 'different-activities-each-count', 'B:273-276; spec 4.3')
def _():
    s = Script(); [s.call('noteTool', ref('s1', 1), 't1', f'a{i}') for i in range(3)]
    return (W([sess('s1', budget(1))]), s, E([R('r3', U)], counts={'findSession': 3}, session=[SV('s1', 'budget.used.tools', 3)]))


@fx('D04', 'same-activity-in-different-turns-counts-twice', 'B:271-273; spec 2.2 (dedup key is the turn)')
def _():
    s = Script(); s.call('noteTool', ref('s1', 1), 't1', 'a1'); s.call('noteTool', ref('s1', 1), 't2', 'a1')
    return (W([sess('s1', budget(1))]), s, E([R('r2', U)], counts={'findSession': 2}, session=[SV('s1', 'budget.used.tools', 2)]))


@fx('D05', 'same-turn-across-budgets-dedups-by-turn', 'B:271-273; spec 2.2 (killer M04)')
def _():
    s = Script(); s.call('noteTool', ref('s1', 1), 't1', 'a1'); s.call('noteTool', ref('s2', 2), 't1', 'a1')
    return (W([sess('s1', budget(1)), sess('s2', budget(2))]), s, E([R('r2', U)], counts={'findSession': 1}, session=[SV('s1', 'budget.used.tools', 1), SV('s2', 'budget.used.tools', 0)]))


@fx('D06', 'missing-session-ref-consumes-the-key', 'B:270-276; spec 4.3; 15.1 #6')
def _():
    s = Script(); s.call('noteTool', ref('nope', 1), 't1', 'a1'); s.call('noteTool', ref('s1', 1), 't1', 'a1')
    return (W([sess('s1', budget(1))]), s, E([R('r2', U)], counts={'findSession': 1}, session=[SV('s1', 'budget.used.tools', 0)]))


@fx('D07', 'stale-budget-ref-consumes-the-key', 'B:270-276; spec 4.3; 15.1 #6')
def _():
    s = Script(); s.call('noteTool', ref('s1', 99), 't1', 'a1'); s.call('noteTool', ref('s1', 1), 't1', 'a1')
    return (W([sess('s1', budget(1))]), s, E([R('r2', U)], counts={'findSession': 1}, session=[SV('s1', 'budget.used.tools', 0)]))


@fx('D08', 'forgetTurn-lets-the-activity-count-again', 'B:280-282; spec 4.3 forgetTurn')
def _():
    s = Script(); s.call('noteTool', ref('s1', 1), 't1', 'a1'); r2 = s.call('forgetTurn', 't1'); s.call('noteTool', ref('s1', 1), 't1', 'a1')
    return (W([sess('s1', budget(1))]), s, E([R(r2, U)], counts={'findSession': 2}, session=[SV('s1', 'budget.used.tools', 2)]))


@fx('D09', 'duplicates-make-no-session-lookup', 'B:273; spec 4.3 (return before CURRENT)')
def _():
    s = Script(); [s.call('noteTool', ref('s1', 1), 't1', 'a1') for _i in range(4)]
    return (W([sess('s1', budget(1))]), s, E([R('r4', U)], counts={'findSession': 1}, session=[SV('s1', 'budget.used.tools', 1)]))


@fx('D10', 'large-turn-set-without-scanning-sessions', 'B:270-277; spec 9 (no N*A copying)')
def _():
    s = Script(); [s.call('noteTool', ref('s1', 1), 'big', f'act{i}') for i in range(200)]; [s.call('noteTool', ref('s1', 1), 'big', f'act{i}') for i in range(200)]
    return (W([sess('s1', budget(1))]), s, E([R('r400', U)], counts={'findSession': 200}, session=[SV('s1', 'budget.used.tools', 200)]))


# ============================== E: runs ======================================================================================
@fx('E01', 'first-start-begins-and-arms', 'B:286-290,312-342; spec 4.4, 5.1-5.3')
def _():
    s = Script(); r = s.call('runStarted', ref('s1', 1), 'k1')
    return (W([sess('s1', budget(1))]), s, E([R(r, U)], counts={'findSession': 2, 'now': 1, 'timer.set': 1, 'timer.unref': 1}, clocks={'count': 1}, timers=1,
            session=[SV('s1', 'budget.activeSince', ISO0)]))


@fx('E02', 'second-run-does-not-rearm', 'B:317; spec 5.1 step 6 (killer M06)')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); r = s.call('runStarted', ref('s1', 1), 'k2')
    return (W([sess('s1', budget(1))]), s, E([R(r, U)], counts={'findSession': 4, 'now': 1, 'timer.set': 1, 'timer.clear': 0}, clocks={'count': 1}, timers=1))


@fx('E03', 'duplicate-start-resets-waiting-and-rearms', 'B:286-289; spec 4.4 runStarted')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); s.adv(1000); s.call('runWaiting', ref('s1', 1), 'k1', True); s.call('runStarted', ref('s1', 1), 'k1')
    return (W([sess('s1', budget(1))]), s, E([R('r3', U)], counts={'timer.set': 2, 'timer.clear': 1, 'now': 3}, timers=1,
            session=[SV('s1', 'budget.used.activeMs', 1000), SV('s1', 'budget.activeSince', iso(T0 + 1000))]))


@fx('E04', 'missing-runs-and-clocks-return-without-lookup', 'B:292-303; spec 4.4')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); a = s.call('runWaiting', ref('s1', 1), 'nope', True); b = s.call('runEnded', ref('s1', 1), 'nope')
    c = s.call('runWaiting', ref('s9', 1), 'k', True); d = s.call('runEnded', ref('s9', 1), 'k')
    return (W([sess('s1', budget(1))]), s, E([R(a, U), R(b, U), R(c, U), R(d, U)], counts={'findSession': 2, 'now': 1, 'timer.set': 1, 'timer.clear': 0}, clocks={'count': 1}))


@fx('E05', 'unchanged-waiting-is-a-no-op', 'B:294-296; spec 4.4 runWaiting')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); s.call('runWaiting', ref('s1', 1), 'k1', False); s.call('runWaiting', ref('s1', 1), 'k1', True); s.call('runWaiting', ref('s1', 1), 'k1', True)
    return (W([sess('s1', budget(1))]), s, E([R('r2', U), R('r4', U)], counts={'findSession': 3, 'now': 2, 'timer.set': 1, 'timer.clear': 1}, timers=0,
            session=[SA('s1', 'budget.activeSince')]))


@fx('E06', 'one-active-run-among-waiting-keeps-consuming', 'B:316; spec 5.1 (killer M05)')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); s.call('runStarted', ref('s1', 1), 'k2'); s.adv(500); s.call('runWaiting', ref('s1', 1), 'k1', True)
    return (W([sess('s1', budget(1))]), s, E([R('r3', U)], counts={'timer.clear': 0, 'now': 1, 'timer.set': 1}, timers=1,
            session=[SV('s1', 'budget.used.activeMs', 0), SV('s1', 'budget.activeSince', ISO0)]))


@fx('E07', 'all-waiting-settles-union-time', 'B:316-321,360-372; spec 5.4 (wall-clock union, not per-run)')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); s.call('runStarted', ref('s1', 1), 'k2'); s.adv(500); s.call('runWaiting', ref('s1', 1), 'k1', True)
    s.adv(500); s.call('runWaiting', ref('s1', 1), 'k2', True)
    return (W([sess('s1', budget(1))]), s, E([R('r4', U)], counts={'timer.clear': 1, 'timer.set': 1, 'now': 2}, timers=0,
            session=[SV('s1', 'budget.used.activeMs', 1000), SA('s1', 'budget.activeSince')]))


@fx('E08', 'resume-after-waiting-begins-a-new-span', 'B:316-324; spec 5.1')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); s.adv(1000); s.call('runWaiting', ref('s1', 1), 'k1', True); s.adv(5000); s.call('runWaiting', ref('s1', 1), 'k1', False)
    return (W([sess('s1', budget(1))]), s, E([R('r3', U)], counts={'timer.set': 2, 'timer.clear': 1}, timers=1,
            session=[SV('s1', 'budget.used.activeMs', 1000), SV('s1', 'budget.activeSince', iso(T0 + 6000))]))


@fx('E09', 'ending-the-last-run-settles-and-keeps-the-clock', 'B:299-303; spec 4.4 runEnded')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); s.adv(2000); r2 = s.call('runEnded', ref('s1', 1), 'k1'); r3 = s.call('runEnded', ref('s1', 1), 'k1')
    return (W([sess('s1', budget(1))]), s, E([R(r2, U), R(r3, U)], counts={'findSession': 3, 'timer.clear': 1}, clocks={'count': 1}, timers=0,
            session=[SV('s1', 'budget.used.activeMs', 2000), SA('s1', 'budget.activeSince')]))


@fx('E10', 'halted-closed-and-stale-refs', 'B:286-290; spec 4.4; 15.1 #8')
def _():
    s = Script(); rs = [s.call('runStarted', ref('s1', 1), 'k'), s.call('runStarted', ref('s2', 2), 'k'), s.call('runStarted', ref('s3', 99), 'k')]
    return (W([sess('s1', budget(1, 'halted')), sess('s2', budget(2, 'closed')), sess('s3', budget(3))]), s, E([R(r, U) for r in rs],
            counts={'findSession': 5, 'now': 0, 'timer.set': 0}, clocks={'count': 2, 'has': [BID(1), BID(2)], 'absent': [BID(3)]}, timers=0, session=[SA('s1', 'budget.activeSince')]))


# ============================== F: timers ====================================================================================
ONE_MIN = {'timeMinutes': 1}


@fx('F01', 'exact-deadline-halts-with-time', 'B:344-358; spec 5.5 FIRE')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); s.adv(MIN)
    return (W([sess('s1', budget(1, limits=ONE_MIN))]), s, E([R('r1', U)], counts={'timer.fire': 1, 'timer.clear': 0, 'save': 1, 'stopMain': 1, 'findSession': 5}, timers=0,
            session=[SV('s1', 'budget.state', 'halted'), SV('s1', 'budget.halt.reason', 'time'), SV('s1', 'budget.halt.at', iso(T0 + MIN)), SV('s1', 'budget.used.activeMs', MIN),
                     SA('s1', 'budget.activeSince')]))


@fx('F02', 'early-delivery-rearms-the-remainder', 'B:352-356; spec 5.5 step 6 (killer M08)')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); s.settime(T0 + 30000); s.timer('t1')
    return (W([sess('s1', budget(1, limits=ONE_MIN))]), s, E([R('r1', U)], counts={'timer.set': 2, 'timer.fire': 1, 'stopMain': 0}, timers=1,
            session=[SV('s1', 'budget.state', 'open'), SV('s1', 'budget.used.activeMs', 30000), SV('s1', 'budget.activeSince', iso(T0 + 30000))]))


@fx('F03', 'late-overshoot-counts-the-real-elapsed', 'B:350,367-370; spec 5.4, 5.5')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); s.settime(T0 + 90000); s.timer('t1')
    return (W([sess('s1', budget(1, limits=ONE_MIN))]), s, E([R('r1', U)], counts={'timer.fire': 1}, timers=0,
            session=[SV('s1', 'budget.state', 'halted'), SV('s1', 'budget.used.activeMs', 90000), SV('s1', 'budget.halt.at', iso(T0 + 90000))]))


@fx('F04', 'backward-clock-contributes-zero', 'B:367; spec 5.4 (killer M07)')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); s.settime(T0 - 5000); r2 = s.call('runEnded', ref('s1', 1), 'k1')
    return (W([sess('s1', budget(1))]), s, E([R(r2, U)], counts={'timer.clear': 1}, session=[SV('s1', 'budget.used.activeMs', 0), SA('s1', 'budget.activeSince')]))


@fx('F05', 'zero-remaining-fires-synchronously', 'B:336-339; spec 5.3 step 3')
def _():
    s = Script(); r = s.call('runStarted', ref('s1', 1), 'k1')
    return (W([sess('s1', budget(1, limits=ONE_MIN, use={'activeMs': MIN}))]), s, E([R(r, U)], counts={'timer.set': 0, 'timer.fire': 0, 'save': 1, 'stopMain': 1}, timers=0,
            session=[SV('s1', 'budget.state', 'halted'), SV('s1', 'budget.halt.reason', 'time'), SV('s1', 'budget.halt.at', ISO0), SA('s1', 'budget.activeSince')]))


@fx('F06', 'unlimited-starts-a-span-without-a-timer', 'B:334; spec 5.3 step 1')
def _():
    s = Script(); r = s.call('runStarted', ref('s1', 1), 'k1')
    return (W([sess('s1', budget(1, unlimited=True))]), s, E([R(r, U)], counts={'timer.set': 0}, clocks={'count': 1}, timers=0, session=[SV('s1', 'budget.activeSince', ISO0)]))


@fx('F07', 'raising-to-unlimited-cancels-the-timer-without-settling', 'B:418-425; spec 4.6 raise; K4 (killer M09)')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); s.adv(1000); r = s.call('raise', ref('s1', 1), 'unlimited')
    return (W([sess('s1', budget(1))]), s, E([R(r, True)], counts={'timer.clear': 1, 'changed': 1, 'unit': 0, 'now': 1}, timers=0,
            session=[SV('s1', 'budget.unlimited', True), SV('s1', 'budget.used.activeMs', 0), SV('s1', 'budget.activeSince', ISO0)]))


@fx('F08', 'stale-and-unlimited-late-callbacks-do-nothing', 'B:344-349; spec 5.5 (late callbacks)')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); s.call('runStarted', ref('s2', 2), 'k1')
    s.put(sess('s1', budget(3, limits=ONE_MIN))); s.call('raise', ref('s2', 2), 'unlimited'); s.timer('t1'); s.timer('t2', captured=True)
    return (W([sess('s1', budget(1, limits=ONE_MIN)), sess('s2', budget(2, limits=ONE_MIN))]), s, E([R('r3', True)], counts={'timer.fire': 2, 'stopMain': 0, 'save': 0}, timers=0,
            clocks={'count': 2}, session=[SV('s1', 'budget.id', BID(3)), SV('s1', 'budget.state', 'open'), SV('s2', 'budget.state', 'open'), SV('s2', 'budget.unlimited', True)]))


@fx('F09', 'oversized-delay-fires-after-one-ms-and-unref-is-optional', 'B:340-341; spec 5.3 steps 4-6')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); s.adv(1)
    return (W([sess('s1', budget(1, limits={'timeMinutes': 40000}))], timerHasUnref=False), s, E([R('r1', U)], counts={'timer.set': 2, 'timer.fire': 1, 'timer.unref': 0}, timers=1,
            session=[SV('s1', 'budget.used.activeMs', 1), SV('s1', 'budget.state', 'open')]))


@fx('F10', 'since-zero-counts-and-the-run-count-quirk', 'B:317,352; spec 5.1, 5.5 step 6; 15.1 #9/#11')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); s.call('runWaiting', ref('s1', 1), 'k1', True); s.timer('t1', captured=True)
    return (W([sess('s1', budget(1, limits=ONE_MIN))], clockStart=0), s, E([R('r2', U)], counts={'timer.set': 2, 'timer.fire': 1, 'now': 3}, timers=1,
            session=[SV('s1', 'budget.activeSince', iso(0)), SV('s1', 'budget.state', 'open'), SV('s1', 'budget.used.activeMs', 0)]))


# ============================== G: halt / close ==============================================================================
@fx('G01', 'missing-session-halt-is-void', 'B:381-384; spec 4.5 halt steps 1-3')
def _():
    s = Script(); r = s.call('halt', ref('nope', 1), 'commands')
    return (W([sess('s1', budget(1))]), s, E([R(r, U)], counts={'findSession': 2, 'runningDelegationIds': 0, 'save': 0, 'stopMain': 0}))


@fx('G02', 'stale-budget-halt-is-void', 'B:381-384; spec 4.5')
def _():
    s = Script(); r = s.call('halt', ref('s1', 99), 'commands')
    return (W([sess('s1', budget(1))]), s, E([R(r, U)], counts={'findSession': 2, 'save': 0}, session=[SV('s1', 'budget.state', 'open')]))


@fx('G03', 'repeated-and-closed-halt-do-nothing-more', 'B:384; spec 4.5 (two lookups, no further effects)')
def _():
    s = Script(); s.call('halt', ref('s1', 1), 'commands'); s.call('halt', ref('s1', 1), 'reviews'); s.call('halt', ref('s2', 2), 'checks')
    return (W([sess('s1', budget(1)), sess('s2', budget(2, 'closed'))]), s, E([R('r2', U)], counts={'findSession': 6, 'save': 1, 'stopMain': 1, 'announce': 1},
            session=[SV('s1', 'budget.halt.reason', 'commands'), SV('s2', 'budget.state', 'closed')]))


@fx('G04', 'delegation-ids-in-order-with-duplicates', 'B:386-396; spec 4.5 steps 5-16 (killer M10)')
def _():
    s = Script(); r = s.call('halt', ref('s1', 1), 'commands')
    return (W([sess('s1', budget(1))], **deps(run=[['d1', 'd2', 'd1']])), s, E([R(r, U)], counts={'stopDelegation': 3, 'runningDelegationIds': 1, 'cancelRecovery': 1, 'stopMain': 1, 'stopAttachedTurns': 1,
            'holdQueued': 1, 'save': 1, 'changed': 1, 'announce': 1}, order=HALT_ORDER + [['stopMain', 'stopDelegation'], ['stopDelegation', 'stopAttachedTurns']],
            session=[SV('s1', 'budget.halt.stoppedDelegationIds', ['d1', 'd2', 'd1']), SV('s1', 'budget.halt.at', ISO0)]))


@fx('G05', 'active-span-settles-before-the-halt-effects', 'B:385,360-372; spec 4.5 step 4 (killer M10)')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); s.adv(5000); r = s.call('halt', ref('s1', 1), 'commands'); s.snap('after-halt')
    return (W([sess('s1', budget(1))]), s, E([R(r, U)], counts={'timer.clear': 1, 'save': 1}, order=[['timer.clear', 'runningDelegationIds']] + HALT_ORDER, timers=0, clocks={'count': 1},
            session=[SV('s1', 'budget.used.activeMs', 5000), SV('s1', 'budget.halt.at', iso(T0 + 5000)), SA('s1', 'budget.activeSince'), SV('s1', 'budget.state', 'halted')]))


@fx('G06', 'empty-delegation-list', 'B:386-391; spec 4.5')
def _():
    s = Script(); r = s.call('halt', ref('s1', 1), 'checks')
    return (W([sess('s1', budget(1))], **deps(run=[[]])), s, E([R(r, U)], counts={'stopDelegation': 0, 'stopMain': 1, 'save': 1},
            session=[SV('s1', 'budget.halt.stoppedDelegationIds', []), SV('s1', 'budget.halt.reason', 'checks')]))


@fx('G07', 'not-quiet-leaves-the-budget-open', 'B:400-403; spec 4.5 maybeClose')
def _():
    s = Script(); r = s.call('maybeClose', SESS('s1'))
    return (W([sess('s1', budget(1))], **deps(quiet=[False])), s, E([R(r, False)], counts={'isQuiet': 1, 'changed': 0, 'findSession': 0}, session=[SV('s1', 'budget.state', 'open')]))


@fx('G08', 'any-run-suppresses-the-quiet-query-even-if-waiting', 'B:403; spec 4.5 maybeClose step 2')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); r1 = s.call('maybeClose', SESS('s1')); s.call('runWaiting', ref('s1', 1), 'k1', True); r3 = s.call('maybeClose', SESS('s1'))
    return (W([sess('s1', budget(1))], **deps(quiet=[True, True])), s, E([R(r1, False), R(r3, False)], counts={'isQuiet': 0, 'changed': 0}, session=[SV('s1', 'budget.state', 'open')]))


@fx('G09', 'quiet-close-does-not-save', 'B:404-407,237-242; spec 4.5 maybeClose, 5.6 CLOSE')
def _():
    s = Script(); r1 = s.call('maybeClose', SESS('s1'))
    s.call('runStarted', ref('s2', 2), 'k1'); s.adv(1000); s.call('runEnded', ref('s2', 2), 'k1'); r4 = s.call('maybeClose', SESS('s2'))
    return (W([sess('s1', budget(1)), sess('s2', budget(2))], **deps(quiet=[True, True])), s, E([R(r1, True), R(r4, True)], counts={'isQuiet': 2, 'changed': 2, 'save': 0}, clocks={'count': 0}, timers=0,
            session=[SV('s1', 'budget.state', 'closed'), SV('s1', 'budget.ended.how', 'quiet'), SV('s1', 'budget.ended.at', ISO0), SV('s2', 'budget.ended.at', iso(T0 + 1000)),
                     SV('s2', 'budget.used.activeMs', 1000)]))


@fx('G10', 'halt-from-charge-fire-and-decide-re-halt', 'B:255,357,466; spec 4.3, 5.5, 4.6 decide')
def _():
    u = unit(timeMinutes=1)
    s = Script(); r1 = s.call('charge', ref('s1', 1), {'commands': 4})
    s.call('runStarted', ref('s2', 2), 'k1'); s.adv(MIN)
    s.call('runStarted', ref('s3', 3), 'k1'); r4 = s.call('decide', 's3', BID(3), 'extend')
    return (W([sess('s1', budget(1)), sess('s2', budget(2, limits=ONE_MIN)), sess('s3', budget(3, 'halted', limits=ONE_MIN, use={'activeMs': 2 * MIN}))], u), s,
            E([R(r1, NO('commands')), R(r4, OKT)], counts={'save': 4, 'announce': 3, 'stopMain': 3, 'queueContinue': 2, 'runningDelegationIds': 3}, timers=0,
              session=[SV('s1', 'budget.halt.reason', 'commands'), SV('s2', 'budget.halt.reason', 'time'), SV('s3', 'budget.halt.reason', 'time'), SV('s3', 'budget.state', 'halted')]))


# ============================== H: raise =====================================================================================
@fx('H01', 'missing-and-closed-return-false', 'B:415-416; spec 4.6 raise step 1')
def _():
    s = Script(); r1 = s.call('raise', ref('nope', 1), 2); r2 = s.call('raise', ref('s1', 1), 4)
    return (W([sess('s1', budget(1, 'closed'))]), s, E([R(r1, False), R(r2, False)], counts={'unit': 0, 'changed': 0}, session=[SV('s1', 'budget.limits', UNIT)]))


@fx('H02', 'size-one-returns-false-without-unit', 'B:427; spec 4.6 step 4')
def _():
    s = Script(); r = s.call('raise', ref('s1', 1), 1)
    return (W([sess('s1', budget(1))]), s, E([R(r, False)], counts={'unit': 0, 'changed': 0}, session=[SV('s1', 'budget.limits', UNIT)]))


@fx('H03', 'numeric-raise-takes-the-maximum-not-the-sum', 'B:431-437; spec 4.6 step 6 (killer M12)')
def _():
    s = Script(); r1 = s.call('raise', ref('s1', 1), 2); r2 = s.call('raise', ref('s2', 2), 2)
    return (W([sess('s1', budget(1)), sess('s2', budget(2, limits={'timeMinutes': 500, 'delegations': 48}))]), s, E([R(r1, True), R(r2, True)], counts={'unit': 2, 'changed': 2},
            session=[SV('s1', 'budget.limits', lim(2)), SV('s2', 'budget.limits', {'timeMinutes': 500, 'delegations': 48, 'commands': 6, 'reviews': 4, 'checks': 2})]))


@fx('H04', 'no-numeric-change-returns-false', 'B:438; spec 4.6 step 8 (killer M12)')
def _():
    s = Script(); r1 = s.call('raise', ref('s1', 1), 2); r2 = s.call('raise', ref('s2', 2), 2)
    return (W([sess('s1', budget(1, limits=lim(2))), sess('s2', budget(2, limits=lim(5)))]), s, E([R(r1, False), R(r2, False)], counts={'unit': 2, 'changed': 0},
            session=[SV('s1', 'budget.limits', lim(2)), SV('s2', 'budget.limits', lim(5))]))


@fx('H05', 'time-only-increase-while-consuming-settles-and-rearms', 'B:440-443; spec 4.6 step 9')
def _():
    u = unit(timeMinutes=1)
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); s.adv(30000); r = s.call('raise', ref('s1', 1), 2)
    return (W([sess('s1', budget(1, limits={'timeMinutes': 1, 'delegations': 48, 'commands': 6, 'reviews': 4, 'checks': 2}))], u), s,
            E([R(r, True)], counts={'timer.clear': 1, 'timer.set': 2, 'changed': 1, 'unit': 1}, timers=1,
              session=[SV('s1', 'budget.limits.timeMinutes', 2), SV('s1', 'budget.used.activeMs', 30000), SV('s1', 'budget.activeSince', iso(T0 + 30000))]))


@fx('H06', 'non-time-increase-does-not-settle', 'B:440; spec 4.6 step 9')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); s.adv(10000); r = s.call('raise', ref('s1', 1), 2)
    return (W([sess('s1', budget(1, limits={'timeMinutes': 500}))]), s, E([R(r, True)], counts={'timer.clear': 0, 'timer.set': 1, 'changed': 1}, timers=1,
            session=[SV('s1', 'budget.limits.timeMinutes', 500), SV('s1', 'budget.limits.commands', 6), SV('s1', 'budget.used.activeMs', 0), SV('s1', 'budget.activeSince', ISO0)]))


@fx('H07', 'unlimited-transition-without-a-clock', 'B:418-425; spec 4.6 step 3')
def _():
    s = Script(); r = s.call('raise', ref('s1', 1), 'unlimited')
    return (W([sess('s1', budget(1, use={'commands': 2}))]), s, E([R(r, True)], counts={'unit': 0, 'changed': 1, 'timer.clear': 0},
            session=[SV('s1', 'budget.unlimited', True), SV('s1', 'budget.limits', UNIT), SV('s1', 'budget.used.commands', 2), SK('s1', 'budget', KEYS_BASE + ['unlimited'])]))


@fx('H08', 'already-unlimited-returns-false', 'B:419; spec 4.6 step 3')
def _():
    s = Script(); r = s.call('raise', ref('s1', 1), 'unlimited')
    return (W([sess('s1', budget(1, unlimited=True))]), s, E([R(r, False)], counts={'changed': 0}, session=[SV('s1', 'budget.unlimited', True)]))


@fx('H09', 'numeric-increase-while-unlimited-keeps-unlimited', 'B:427-445; spec 4.6; 15.1 #13')
def _():
    s = Script(); r = s.call('raise', ref('s1', 1), 2)
    return (W([sess('s1', budget(1, unlimited=True))]), s, E([R(r, True)], counts={'unit': 1, 'changed': 1}, session=[SV('s1', 'budget.limits', lim(2)), SV('s1', 'budget.unlimited', True)]))


@fx('H10', 'unlimited-keeps-active-since-and-halted-stays-halted', 'B:418-445; spec 4.6; 15.1 #14 (killer M09)')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); s.adv(4000); r2 = s.call('raise', ref('s1', 1), 'unlimited'); r3 = s.call('raise', ref('s2', 2), 4)
    return (W([sess('s1', budget(1)), sess('s2', budget(2, 'halted'))]), s, E([R(r2, True), R(r3, True)], counts={'timer.clear': 1, 'changed': 2}, timers=0,
            session=[SV('s1', 'budget.activeSince', ISO0), SV('s1', 'budget.used.activeMs', 0), SV('s2', 'budget.state', 'halted'), SV('s2', 'budget.limits', lim(4)), SV('s2', 'budget.halt.reason', 'commands')]))


# ============================== I: decide ====================================================================================
@fx('I01', 'not-found', 'B:450-452; spec 4.6 decide steps 1-2')
def _():
    s = Script(); r1 = s.call('decide', 'nope', BID(1), 'extend'); r2 = s.call('decide', 's1', BID(99), 'end'); r3 = s.call('decide', 's2', BID(2), 'extend')
    nf = {'ok': False, 'code': 'not-found'}
    return (W([sess('s1', budget(1, 'halted')), sess('s2')]), s, E([R(r1, nf), R(r2, nf), R(r3, nf)], counts={'findSession': 3, 'save': 0, 'changed': 0}, session=[SV('s1', 'budget.state', 'halted')]))


@fx('I02', 'not-halted', 'B:453; spec 4.6 decide step 3')
def _():
    s = Script(); r1 = s.call('decide', 's1', BID(1), 'extend'); r2 = s.call('decide', 's2', BID(2), 'end')
    nh = {'ok': False, 'code': 'not-halted'}
    return (W([sess('s1', budget(1)), sess('s2', budget(2, 'closed'))]), s, E([R(r1, nh), R(r2, nh)], counts={'save': 0, 'unit': 0, 'dropHeld': 0}, session=[SV('s1', 'budget.limits', UNIT)]))


@fx('I03', 'default-extend-is-additive', 'B:459,461-469; spec 4.6 extend (killer M13)')
def _():
    s = Script(); r = s.call('decide', 's1', BID(1), 'extend')
    return (W([sess('s1', budget(1, 'halted', use={'commands': 3}))]), s, E([R(r, OKT)], counts={'unit': 1, 'save': 1, 'changed': 1, 'findSession': 2, 'queueContinue': 2, 'releaseHeld': 1},
            order=[['save', 'changed'], ['changed', 'queueContinue'], ['queueContinue', 'releaseHeld']],
            session=[SV('s1', 'budget.limits', lim(2)), SV('s1', 'budget.state', 'open'), SA('s1', 'budget.halt'), SV('s1', 'budget.used.commands', 3)]))


@fx('I04', 'sizes-2-4-8-add-multiples-of-the-unit', 'B:459; spec 4.6 extend (killer M13)')
def _():
    u = {'timeMinutes': 10, 'delegations': 5, 'commands': 3, 'reviews': 2, 'checks': 1}
    s = Script(); rs = [s.call('decide', f's{i + 1}', BID(i + 1), 'extend', n) for i, n in enumerate((2, 4, 8))]
    return (W([sess(f's{i}', budget(i, 'halted', limits=u)) for i in (1, 2, 3)], u), s, E([R(r, OKT) for r in rs],
            session=[SV('s1', 'budget.limits', lim(3, u)), SV('s2', 'budget.limits', lim(5, u)), SV('s3', 'budget.limits', lim(9, u))]))


@fx('I05', 'unlimited-extend-needs-no-unit', 'B:456; spec 4.6 extend step 2')
def _():
    s = Script(); r = s.call('decide', 's1', BID(1), 'extend', 'unlimited')
    return (W([sess('s1', budget(1, 'halted'))]), s, E([R(r, OKT)], counts={'unit': 0, 'save': 1, 'changed': 1},
            session=[SV('s1', 'budget.unlimited', True), SV('s1', 'budget.limits', UNIT), SV('s1', 'budget.state', 'open'), SA('s1', 'budget.halt')]))


@fx('I06', 'reason-and-release-true-does-not-wake', 'B:468-469; spec 4.6 extend steps 9-11 (killer M14)')
def _():
    s = Script(); r = s.call('decide', 's1', BID(1), 'extend')
    return (W([sess('s1', budget(1, 'halted', reason='reviews'))], **deps(release=[True])), s, E([R(r, OKT)], counts={'queueContinue': 1, 'releaseHeld': 1}, order=[['queueContinue', 'releaseHeld']]))


@fx('I07', 'reason-and-release-false-wakes-once', 'B:469; spec 4.6 extend step 11 (killer M14)')
def _():
    s = Script(); r = s.call('decide', 's1', BID(1), 'extend')
    return (W([sess('s1', budget(1, 'halted', reason='reviews'))], **deps(release=[False])), s, E([R(r, OKT)], counts={'queueContinue': 2, 'releaseHeld': 1}, order=[['queueContinue', 'releaseHeld']]))


@fx('I08', 'time-reason-with-both-release-results', 'B:455,468-469; spec 4.6 (replaces the excluded halt-without-reason case, 15.1 #16)')
def _():
    s = Script(); r1 = s.call('decide', 's1', BID(1), 'extend'); r2 = s.call('decide', 's2', BID(2), 'extend')
    return (W([sess('s1', budget(1, 'halted', reason='time')), sess('s2', budget(2, 'halted', reason='time'))], **deps(release=[True, False])), s,
            E([R(r1, OKT), R(r2, OKT)], counts={'queueContinue': 3, 'releaseHeld': 2, 'save': 2}, session=[SV('s1', 'budget.state', 'open'), SV('s2', 'budget.state', 'open')]))


@fx('I09', 'user-end-closes-drops-and-saves', 'B:472-476; spec 4.6 end')
def _():
    s = Script(); r = s.call('decide', 's1', BID(1), 'end')
    return (W([sess('s1', budget(1, 'halted'))]), s, E([R(r, OKT)], counts={'dropHeld': 1, 'save': 1, 'changed': 1, 'releaseHeld': 0, 'queueContinue': 0},
            order=[['dropHeld', 'save'], ['save', 'changed']],
            session=[SV('s1', 'budget.state', 'closed'), SV('s1', 'budget.ended.how', 'user-end'), SV('s1', 'budget.ended.at', ISO0), SV('s1', 'budget.halt.reason', 'commands')]))


@fx('I10', 'retained-run-and-immediate-re-halt-after-refresh', 'B:466-469; spec 4.6 decide (no early return)')
def _():
    u = unit(timeMinutes=1)
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); r2 = s.call('decide', 's1', BID(1), 'extend')
    return (W([sess('s1', budget(1, 'halted', limits=ONE_MIN, use={'activeMs': 2 * MIN}))], u), s, E([R(r2, OKT)], counts={'save': 2, 'announce': 1, 'queueContinue': 2, 'releaseHeld': 1},
            order=[['announce', 'queueContinue'], ['queueContinue', 'releaseHeld']],
            session=[SV('s1', 'budget.state', 'halted'), SV('s1', 'budget.halt.reason', 'time'), SV('s1', 'budget.limits.timeMinutes', 2)]))


# ============================== J: card validation ===========================================================================
def card(**kw):
    base = {'deliverable': 'Ship it', 'done': A('one')}
    base.update(kw)
    return O(**base)


def one_session(): return W([sess('s1', budget(1))])
A300, A301 = 'a' * 300, 'a' * 301
def bad(key): return {'ok': False, 'error': ERR['ko'][key]}
BAD_TARGETS = O(confirmed=-1, candidate=0, unknown=0)
GOOD_TARGETS = O(confirmed=1, candidate=0, unknown=2)


@fx('J01', 'missing-and-closed-use-the-missing-error', 'B:481-484,88-107; spec 4.7 steps 1-4')
def _():
    s = Script(); r1 = s.call('setCard', ref('nope', 1), card()); r2 = s.call('setCard', ref('s1', 1), card()); r3 = s.call('setCard', ref('s1', 1), card(), 'en')
    return (W([sess('s1', budget(1, 'closed'))]), s, E([R(r1, bad('missing')), R(r2, bad('missing')), R(r3, {'ok': False, 'error': ERR['en']['missing']})],
            counts={'findSession': 6, 'now': 0, 'save': 0}))


@fx('J02', 'deliverable-type-empty-and-300-301-units', 'B:145-147,486; spec 4.7 step 5')
def _():
    s = Script(); cs = [5, '', '   ', A300, A301, ' ' + A300 + ' ', U, None]
    rs = [s.call('setCard', ref('s1', 1), card(deliverable=c)) for c in cs]
    ok = [3, 5]
    return (one_session(), s, E([R(r, OKT if i in ok else bad('deliverable')) for i, r in enumerate(rs)], counts={'now': 2, 'save': 2, 'changed': 2}, session=[SV('s1', 'budget.card.deliverable', A300)]))


@fx('J03', 'done-counts-and-invalid-lines', 'B:149-150,487; spec 4.7 step 6')
def _():
    s = Script()
    cs = [O(deliverable='d'), card(done=A()), card(done=A(*['x'] * 6)), card(done=A(A301)), card(done=A('ok', '')), card(done=A(*['x'] * 5)), card(done='text'), card(done=A(5)),
          card(done=A('  ')), card(done=A(A300))]
    rs = [s.call('setCard', ref('s1', 1), c) for c in cs]
    ok = [5, 9]
    return (one_session(), s, E([R(r, OKT if i in ok else bad('done')) for i, r in enumerate(rs)], counts={'now': 2, 'save': 2}, session=[SV('s1', 'budget.card.done', [A300])]))


@fx('J04', 'targets-missing-negative-fractional-and-absent', 'B:489-495; spec 4.7 step 8')
def _():
    s = Script()
    cs = [card(targets=None), card(targets=BAD_TARGETS), card(targets=O(confirmed=num(1.5), candidate=0, unknown=0)), card(targets=O(confirmed=1)), card(targets=GOOD_TARGETS), card(targets=U)]
    rs = [s.call('setCard', ref('s1', 1), c) for c in cs]
    ok = [4, 5]
    return (one_session(), s, E([R(r, OKT if i in ok else bad('targets')) for i, r in enumerate(rs)], counts={'now': 6, 'save': 2}, session=[SA('s1', 'budget.card.targets')]))


@fx('J05', 'raw-note-types-and-300-301-units', 'B:492; spec 4.7 step 8 (note is measured raw); 15.1 #19')
def _():
    s = Script()
    def t(note): return card(targets=O(confirmed=0, candidate=0, unknown=0, note=note))
    cs = [t(0), t(A300), t(A301), t(None), t(''), t(U)]
    rs = [s.call('setCard', ref('s1', 1), c) for c in cs]
    ok = [1, 4, 5]
    return (one_session(), s, E([R(r, OKT if i in ok else bad('targets')) for i, r in enumerate(rs)], counts={'now': 6, 'save': 3},
            session=[SK('s1', 'budget.card.targets', ['confirmed', 'candidate', 'unknown'])]))


@fx('J06', 'verify-count-label-and-status', 'B:496-500; spec 4.7 step 9')
def _():
    s = Script()
    def it(label='x', status='done'): return O(label=label, status=status)
    cs = [card(verify=A()), card(verify=A(*[it()] * 6)), card(verify=A(it(status='bad'))), card(verify=A(None)), card(verify=A(it(label=''))), card(verify=A(*[it(status=st) for st in ('planned', 'done', 'skipped', 'done', 'planned')])),
          card(verify='x'), card(verify=A(it(label=A301))), card(verify=A(it(label='  t  ')))]
    rs = [s.call('setCard', ref('s1', 1), c) for c in cs]
    ok = [0, 5, 8]
    return (one_session(), s, E([R(r, OKT if i in ok else bad('verify')) for i, r in enumerate(rs)], counts={'save': 3},
            session=[SV('s1', 'budget.card.verify', [{'label': 't', 'status': 'done'}])]))


@fx('J07', 'conflicts-count-empty-and-invalid', 'B:501-504; spec 4.7 step 10')
def _():
    s = Script()
    cs = [card(conflicts=A()), card(conflicts=A(*['c'] * 6)), card(conflicts=A('')), card(conflicts=A(A301)), card(conflicts=A(*['c'] * 5)), card(conflicts=A(' c '))]
    rs = [s.call('setCard', ref('s1', 1), c) for c in cs]
    ok = [0, 4, 5]
    return (one_session(), s, E([R(r, OKT if i in ok else bad('conflicts')) for i, r in enumerate(rs)], counts={'save': 3}, session=[SV('s1', 'budget.card.conflicts', ['c'])]))


@fx('J08', 'all-state-values-and-invalid-state', 'B:505-508; spec 4.7 step 11')
def _():
    s = Script()
    cs = [card(state='working'), card(state='done'), card(state='waiting-user'), card(state='blocked'), card(state='idle'), card(state=5), card(state=''), card(state=U)]
    rs = [s.call('setCard', ref('s1', 1), c) for c in cs]
    ok = [0, 1, 2, 3, 7]
    return (one_session(), s, E([R(r, OKT if i in ok else bad('state')) for i, r in enumerate(rs)], counts={'save': 5}, session=[SA('s1', 'budget.card.state')]))


@fx('J09', 'multi-error-precedence', 'B:486-508; spec 4.7 (missing, deliverable, done, targets, verify, conflicts, state)')
def _():
    bv, bc, bs = A(O(label='', status='x')), A(''), 'idle'
    cases = [(O(deliverable=5, done=A(), targets=BAD_TARGETS, verify=bv, conflicts=bc, state=bs), 'deliverable'),
             (card(done=A(), targets=BAD_TARGETS, verify=bv, conflicts=bc, state=bs), 'done'),
             (card(targets=BAD_TARGETS, verify=bv, conflicts=bc, state=bs), 'targets'),
             (card(verify=bv, conflicts=bc, state=bs), 'verify'),
             (card(conflicts=bc, state=bs), 'conflicts'),
             (card(state=bs), 'state')]
    s = Script(); rs = [(s.call('setCard', ref('s1', 1), c), k) for c, k in cases]
    return (one_session(), s, E([R(r, bad(k)) for r, k in rs], counts={'save': 0, 'changed': 0, 'now': 4}, session=[SA('s1', 'budget.card')]))


@fx('J10', 'old-card-preserved-and-optional-error-consumes-now', 'B:488-510; spec 4.7 step 7; 15.1 #18, #21', kd=['KD-1'])
def _():
    s = Script(); r1 = s.call('setCard', ref('s1', 1), card(deliverable='first', verify=A(O(label='v', status='done'))))
    s.settime(T0 + 5000); r2 = s.call('setCard', ref('s1', 1), card(deliverable='second', targets=BAD_TARGETS))
    mid = len(s.steps)
    return (one_session(), s, E([R(r1, OKT), R(r2, bad('targets'))], counts={'now': 2, 'save': 1, 'changed': 1},
            session=[SV('s1', 'budget.card.deliverable', 'first'), SV('s1', 'budget.card.updatedAt', ISO0), SV('s1', 'budget.card.verify', [{'label': 'v', 'status': 'done'}])]))


# ============================== K: card representation =======================================================================
@fx('K01', 'trimmed-success-with-every-optional-field', 'B:488-509; spec 4.7 steps 7-13')
def _():
    c = O(deliverable='  hello  ', done=A('  a ', 'b\t'), targets=O(confirmed=1, candidate=2, unknown=3, note='n'), verify=A(O(label=' v ', status='planned')), conflicts=A(' c '), state='working')
    s = Script(); r = s.call('setCard', ref('s1', 1), c, 'ko', O(sessionId='s9', turnId='t9'))
    return (one_session(), s, E([R(r, {'ok': True})], counts={'now': 1, 'save': 1, 'changed': 1}, order=[['now', 'save'], ['save', 'changed']],
            session=[SV('s1', 'budget.card.deliverable', 'hello'), SV('s1', 'budget.card.done', ['a', 'b']), SV('s1', 'budget.card.targets', {'confirmed': 1, 'candidate': 2, 'unknown': 3, 'note': 'n'}),
                     SV('s1', 'budget.card.verify', [{'label': 'v', 'status': 'planned'}]), SV('s1', 'budget.card.conflicts', ['c']), SV('s1', 'budget.card.state', 'working'),
                     SV('s1', 'budget.card.updatedAt', ISO0), SK('s1', 'budget.card', ['deliverable', 'done', 'updatedAt', 'targets', 'verify', 'conflicts', 'state', 'by'])]))


@fx('K02', 'astral-300-unit-boundary-counts-utf16-units', 'B:146; spec 4.7 step 5; 15.1 #19 (killer M15)')
def _():
    em = '\U0001F600'
    cs = [('a' * 298 + em, True), ('a' * 299 + em, False), (em * 150, True), (em * 151, False)]
    s = Script(); rs = [(s.call('setCard', ref('s1', 1), card(deliverable=c)), ok) for c, ok in cs]
    return (one_session(), s, E([R(r, OKT if ok else bad('deliverable')) for r, ok in rs], counts={'save': 2}, session=[SV('s1', 'budget.card.deliverable', em * 150)]))


@fx('K03', 'lone-surrogates-count-and-survive', 'B:146,488; spec 4.7, 2.3 (arbitrary UTF-16)')
def _():
    s = Script()
    r1 = s.call('setCard', ref('s1', 1), card(deliverable=u16(*[0xD800] * 301))); r2 = s.call('setCard', ref('s1', 1), card(deliverable=u16(*[0xD800] * 300)))
    r3 = s.call('setCard', ref('s1', 1), card(deliverable=u16(0xD800, 0x61), done=A('a', u16(0xDC00))))
    return (one_session(), s, E([R(r1, bad('deliverable')), R(r2, OKT), R(r3, OKT)], counts={'save': 2},
            session=[SV('s1', 'budget.card.deliverable', u16(0xD800, 0x61)), SV('s1', 'budget.card.done', ['a', u16(0xDC00)])]))


@fx('K04', 'ecmascript-trim-versus-u0085-and-u200b', 'B:146,488; spec 4.7 (String.prototype.trim set)')
def _():
    s = Script()
    r1 = s.call('setCard', ref('s1', 1), card(deliverable='﻿')); r2 = s.call('setCard', ref('s1', 1), card(deliverable=' 　x '))
    r3 = s.call('setCard', ref('s1', 1), card(deliverable='​')); r4 = s.call('setCard', ref('s1', 1), card(deliverable='\u0085', done=A('\u0085x​')))
    return (one_session(), s, E([R(r1, bad('deliverable')), R(r2, OKT), R(r3, OKT), R(r4, OKT)], counts={'save': 3},
            session=[SV('s1', 'budget.card.deliverable', '\u0085'), SV('s1', 'budget.card.done', ['\u0085x​'])]))


@fx('K05', 'empty-optional-arrays-are-stored', 'B:496-504; spec 4.7 steps 9-10')
def _():
    s = Script(); r = s.call('setCard', ref('s1', 1), card(verify=A(), conflicts=A()))
    return (one_session(), s, E([R(r, OKT)], session=[SV('s1', 'budget.card.verify', []), SV('s1', 'budget.card.conflicts', []),
            SK('s1', 'budget.card', ['deliverable', 'done', 'updatedAt', 'verify', 'conflicts'])]))


@fx('K06', 'empty-note-is-omitted', 'B:494; spec 4.7 step 8; 15.1 #20')
def _():
    s = Script(); r = s.call('setCard', ref('s1', 1), card(targets=O(confirmed=1, candidate=2, unknown=3, note='')))
    return (one_session(), s, E([R(r, OKT)], session=[SK('s1', 'budget.card.targets', ['confirmed', 'candidate', 'unknown']), SV('s1', 'budget.card.targets.unknown', 3)]))


@fx('K07', 'whitespace-note-is-retained-untrimmed', 'B:492-494; spec 4.7 step 8; 15.1 #20')
def _():
    s = Script(); r1 = s.call('setCard', ref('s1', 1), card(targets=O(confirmed=0, candidate=0, unknown=0, note='   '))); r2 = s.call('setCard', ref('s2', 2), card(targets=O(confirmed=0, candidate=0, unknown=0, note=' x ')))
    return (W([sess('s1', budget(1)), sess('s2', budget(2))]), s, E([R(r1, OKT), R(r2, OKT)], session=[SV('s1', 'budget.card.targets.note', '   '), SV('s2', 'budget.card.targets.note', ' x ')]))


@fx('K08', 'by-projection-and-ignored-supplied-metadata', 'B:509,488; spec 4.7 steps 7, 12')
def _():
    c = card(updatedAt='bogus', by=O(sessionId='zz', turnId='zz'), extra='x')
    s = Script(); r1 = s.call('setCard', ref('s1', 1), c, 'ko', O(sessionId='s9', turnId='t9', extra=1)); r2 = s.call('setCard', ref('s2', 2), c, 'ko', None)
    return (W([sess('s1', budget(1)), sess('s2', budget(2))]), s, E([R(r1, OKT), R(r2, OKT)], session=[SK('s1', 'budget.card.by', ['sessionId', 'turnId']), SV('s1', 'budget.card.by.turnId', 't9'),
            SV('s1', 'budget.card.updatedAt', ISO0), SK('s1', 'budget.card', ['deliverable', 'done', 'updatedAt', 'by']), SK('s2', 'budget.card', ['deliverable', 'done', 'updatedAt'])]))


@fx('K09', 'halted-success-and-hasCard-without-state-checks', 'B:484,516-518; spec 4.7 hasCard')
def _():
    stored = {'deliverable': 'd', 'done': ['x'], 'updatedAt': ISO0}
    s = Script(); r1 = s.call('setCard', ref('s1', 1), card()); r2 = s.call('hasCard', ref('s1', 1)); r3 = s.call('hasCard', ref('s2', 2)); r4 = s.call('hasCard', ref('s3', 3)); r5 = s.call('hasCard', ref('nope', 1))
    return (W([sess('s1', budget(1, 'halted')), sess('s2', budget(2, 'closed', card=stored)), sess('s3', budget(3))]), s,
            E([R(r1, OKT), R(r2, True), R(r3, True), R(r4, False), R(r5, False)], counts={'save': 1, 'changed': 1}, session=[SV('s1', 'budget.state', 'halted')]))


@fx('K10', 'number-isinteger-edge-values', 'B:493; spec 2.3 (Number.isInteger, not isSafeInteger)')
def _():
    def t(v): return card(targets=O(confirmed=v, candidate=0, unknown=0))
    vals = [(NEG0, True), (num(2 ** 53), True), (num(1e300), True), (num(float('nan')), False), (num(float('inf')), False), (num(float('-inf')), False), (num(0.5), False), (num(-1.0), False)]
    s = Script(); rs = [(s.call('setCard', ref('s1', 1), t(v)), ok) for v, ok in vals]
    return (one_session(), s, E([R(r, OKT if ok else bad('targets')) for r, ok in rs], counts={'save': 3}, session=[SV('s1', 'budget.card.targets.confirmed', num(1e300))]))


# ============================== L: auto-continuation =========================================================================
def ac(sid='s1', count=1, **kw): return {'sessionId': sid, 'userTurnId': 'u1', 'count': count, **kw}


@fx('L01', 'clear-present-and-absent', 'B:521-524; spec 4.8 clearAutoContinue')
def _():
    s = Script(); r1 = s.call('clearAutoContinue', ref('s1', 1)); r2 = s.call('clearAutoContinue', ref('s1', 1)); r3 = s.call('clearAutoContinue', ref('nope', 1))
    return (W([sess('s1', budget(1, autoContinue=ac()))]), s, E([R(r1, U), R(r2, U), R(r3, U)], counts={'save': 0, 'changed': 0, 'findSession': 3}, session=[SA('s1', 'budget.autoContinue')]))


@fx('L02', 'record-with-missing-and-stale-refs', 'B:527-531; spec 4.8 recordAutoContinue')
def _():
    s = Script(); r1 = s.call('recordAutoContinue', ref('nope', 1), ac()); r2 = s.call('recordAutoContinue', ref('s1', 99), ac())
    return (W([sess('s1', budget(1))]), s, E([R(r1, False), R(r2, False)], counts={'findSession': 2, 'save': 0}, session=[SA('s1', 'budget.autoContinue')]))


@fx('L03', 'record-and-delete-on-closed-without-limits', 'B:530; spec 4.8; 15.1 #22 (no count maximum)')
def _():
    s = Script(); r1 = s.call('recordAutoContinue', ref('s1', 1), ac(count=9)); r2 = s.call('recordAutoContinue', ref('s2', 2), None); r3 = s.call('recordAutoContinue', ref('s3', 3), U)
    return (W([sess('s1', budget(1, 'closed')), sess('s2', budget(2, 'closed', autoContinue=ac())), sess('s3', budget(3, 'halted', autoContinue=ac()))]), s,
            E([R(r1, True), R(r2, True), R(r3, True)], counts={'save': 0, 'changed': 0}, session=[SV('s1', 'budget.autoContinue.count', 9), SA('s2', 'budget.autoContinue'), SA('s3', 'budget.autoContinue')]))


@fx('L04', 'pointed-budget-owner-is-saved', 'B:540-553; spec 4.8 stopAutoContinue')
def _():
    s = Script(); r = s.call('stopAutoContinue', 's1', ref('s2', 2))
    return (W([sess('s1', budget(1)), sess('s2', budget(2, autoContinue=ac('s1')))]), s, E([R(r, U)], counts={'findSession': 2, 'save': 1, 'changed': 1}, order=[['save', 'changed']],
            session=[SV('s2', 'budget.autoContinue.stopped', True), SA('s1', 'budget.autoContinue')]))


@fx('L05', 'stale-pointer-falls-back-to-the-callers-budget', 'B:540-542; spec 4.8 (stale pointed allows fallback)')
def _():
    s = Script(); r = s.call('stopAutoContinue', 's1', ref('s2', 99))
    return (W([sess('s1', budget(1, autoContinue=ac('s1'))), sess('s2', budget(2, autoContinue=ac('s1')))]), s, E([R(r, U)], counts={'findSession': 2, 'save': 1, 'changed': 1},
            session=[SV('s1', 'budget.autoContinue.stopped', True), SA('s2', 'budget.autoContinue.stopped')]))


@fx('L06', 'found-non-open-pointer-blocks-the-fallback', 'B:541-543; spec 4.8; 15.1 #23')
def _():
    s = Script(); r = s.call('stopAutoContinue', 's1', ref('s2', 2))
    return (W([sess('s1', budget(1, autoContinue=ac('s1'))), sess('s2', budget(2, 'closed'))]), s, E([R(r, U)], counts={'save': 0, 'changed': 0}, session=[SA('s1', 'budget.autoContinue.stopped')]))


@fx('L07', 'matching-record-is-stopped', 'B:545-547; spec 4.8 step 5')
def _():
    s = Script(); r = s.call('stopAutoContinue', 's1')
    return (W([sess('s1', budget(1, autoContinue=ac('s1', 1, lastFrom='t0')))]), s, E([R(r, U)], counts={'findSession': 1, 'save': 1, 'changed': 1},
            session=[SV('s1', 'budget.autoContinue.stopped', True), SK('s1', 'budget.autoContinue', ['sessionId', 'userTurnId', 'count', 'lastFrom', 'stopped'])]))


@fx('L08', 'already-stopped-returns-without-saving', 'B:546; spec 4.8 step 5')
def _():
    s = Script(); r = s.call('stopAutoContinue', 's1')
    return (W([sess('s1', budget(1, autoContinue=ac('s1', 1, stopped=True)))]), s, E([R(r, U)], counts={'save': 0, 'changed': 0}))


@fx('L09', 'working-own-card-creates-a-stopped-record', 'B:548-549; spec 4.8 step 6')
def _():
    c = {'deliverable': 'd', 'done': ['x'], 'state': 'working', 'by': {'sessionId': 's1', 'turnId': 't1'}, 'updatedAt': ISO0}
    s = Script(); r = s.call('stopAutoContinue', 's1')
    return (W([sess('s1', budget(1, card=c))]), s, E([R(r, U)], counts={'save': 1, 'changed': 1},
            session=[SV('s1', 'budget.autoContinue', {'sessionId': 's1', 'userTurnId': '', 'count': 0, 'stopped': True}), SK('s1', 'budget.autoContinue', ['sessionId', 'userTurnId', 'count', 'stopped'])]))


@fx('L10', 'other-owner-nonworking-and-no-card-are-no-ops', 'B:545-550; spec 4.8 (killer M16)')
def _():
    base = {'deliverable': 'd', 'done': ['x'], 'updatedAt': ISO0}
    cards = [None, {**base, 'state': 'done', 'by': {'sessionId': 's1', 'turnId': 't'}}, {**base, 'state': 'working', 'by': {'sessionId': 's9', 'turnId': 't'}}]
    sessions = [sess('s1', budget(1, autoContinue=ac('s2'))), sess('s2', budget(2)), sess('s3', budget(3, card=cards[1])), sess('s4', budget(4, card=cards[2]))]
    s = Script(); rs = [s.call('stopAutoContinue', sid) for sid in ('s1', 's2', 's3', 's4')]
    return (W(sessions), s, E([R(r, U) for r in rs], counts={'save': 0, 'changed': 0}, session=[SV('s1', 'budget.autoContinue.sessionId', 's2'), SA('s1', 'budget.autoContinue.stopped'),
            SA('s2', 'budget.autoContinue'), SA('s3', 'budget.autoContinue')]))


# ============================== M: restore / cleanup =========================================================================
@fx('M01', 'restore-without-a-budget-does-nothing', 'B:556-557; spec 4.9 bootRestore')
def _():
    s = Script(); r = s.call('bootRestore', SESS('s1'))
    return (W([sess('s1')]), s, E([R(r, U)], counts={'findSession': 0, 'now': 0, 'timer.clear': 0, 'changed': 0, 'save': 0}))


@fx('M02', 'restore-drops-the-running-clock-without-charging', 'B:558-559; spec 4.9; 15.1 #24')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); s.adv(3000); r = s.call('bootRestore', SESS('s1'))
    return (W([sess('s1', budget(1))]), s, E([R(r, U)], counts={'timer.clear': 1, 'now': 1}, clocks={'count': 0}, timers=0,
            session=[SV('s1', 'budget.used.activeMs', 0), SA('s1', 'budget.activeSince')]))


@fx('M03', 'restore-keeps-a-halted-budget', 'B:556-560; spec 4.9')
def _():
    b = budget(1, 'halted', reason='reviews', activeSince=ISO0, use={'activeMs': 5000, 'reviews': 2})
    s = Script(); r = s.call('bootRestore', SESS('s1'))
    return (W([sess('s1', b)]), s, E([R(r, U)], counts={'save': 0, 'changed': 0}, session=[SA('s1', 'budget.activeSince'), SV('s1', 'budget.state', 'halted'),
            SV('s1', 'budget.halt.reason', 'reviews'), SV('s1', 'budget.used.activeMs', 5000), SV('s1', 'budget.used.reviews', 2)]))


@fx('M04', 'restore-preserves-usage-card-and-tool-dedup', 'B:555-560; spec 4.9 (toolSeen kept)')
def _():
    c = {'deliverable': 'd', 'done': ['x'], 'updatedAt': ISO0}
    s = Script(); s.call('noteTool', ref('s1', 1), 't1', 'a1'); s.call('bootRestore', SESS('s1')); s.call('noteTool', ref('s1', 1), 't1', 'a1')
    return (W([sess('s1', budget(1, use={'commands': 2, 'activeMs': 7000}, card=c, autoContinue=ac('s1'), activeSince=ISO0))]), s, E([R('r2', U)], counts={'findSession': 1},
            session=[SV('s1', 'budget.used.tools', 1), SV('s1', 'budget.used.commands', 2), SV('s1', 'budget.used.activeMs', 7000), SV('s1', 'budget.card.deliverable', 'd'),
                     SV('s1', 'budget.autoContinue.count', 1), SA('s1', 'budget.activeSince')]))


@fx('M05', 'forgetSession-drops-only-matching-clocks', 'B:563-564; spec 4.9; 15.1 #25')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); s.call('runStarted', ref('s2', 2), 'k1'); r = s.call('forgetSession', 's1')
    return (W([sess('s1', budget(1)), sess('s2', budget(2))]), s, E([R(r, U)], counts={'timer.clear': 1}, clocks={'count': 1, 'has': [BID(2)], 'absent': [BID(1)]}, timers=1,
            session=[SV('s1', 'budget.activeSince', ISO0), SV('s1', 'budget.state', 'open')]))


@fx('M06', 'forgetSession-with-no-match-changes-nothing', 'B:563-564; spec 4.9')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); s.call('runStarted', ref('s2', 2), 'k1'); r = s.call('forgetSession', 'nope')
    return (W([sess('s1', budget(1)), sess('s2', budget(2))]), s, E([R(r, U)], counts={'timer.clear': 0}, clocks={'count': 2}, timers=2))


@fx('M07', 'dispose-clears-every-timer-and-all-clocks', 'B:568-572; spec 4.9 dispose')
def _():
    s = Script()
    for i in (1, 2, 3): s.call('runStarted', ref(f's{i}', i), 'k1')
    s.adv(1000); r = s.call('dispose')
    return (W([sess(f's{i}', budget(i)) for i in (1, 2, 3)]), s, E([R(r, U)], counts={'timer.set': 3, 'timer.clear': 3, 'now': 3}, clocks={'count': 0}, timers=0,
            session=[SV('s1', 'budget.used.activeMs', 0), SV('s3', 'budget.activeSince', ISO0)]))


@fx('M08', 'dispose-clears-the-tool-dedup', 'B:571; spec 4.9')
def _():
    s = Script(); s.call('noteTool', ref('s1', 1), 't1', 'a1'); s.call('dispose'); s.call('noteTool', ref('s1', 1), 't1', 'a1')
    return (W([sess('s1', budget(1))]), s, E([R('r3', U)], counts={'findSession': 2}, session=[SV('s1', 'budget.used.tools', 2)]))


@fx('M09', 'cleanup-neither-settles-nor-edits-persisted-markers', 'B:562-572; spec 4.9; 15.1 #25')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); s.adv(4000); s.call('forgetSession', 's1'); r3 = s.call('dispose')
    return (W([sess('s1', budget(1))]), s, E([R(r3, U)], counts={'now': 1, 'timer.clear': 1}, clocks={'count': 0}, timers=0,
            session=[SV('s1', 'budget.used.activeMs', 0), SV('s1', 'budget.activeSince', ISO0), SV('s1', 'budget.state', 'open')]))


@fx('M10', 'two-instances-and-repeated-cleanup', 'B:162-165,562-572; spec 2.2 (clocks are per instance)')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1', i='A'); s.call('runStarted', ref('s1', 1), 'k1', i='B')
    s.call('dispose', i='A'); s.call('dispose', i='A'); s.call('forgetSession', 's1', i='B'); r6 = s.call('dispose', i='B')
    return (W([sess('s1', budget(1))]), s, E([R(r6, U)], counts={'timer.set': 2, 'timer.clear': 2}, timers=0, session=[SV('s1', 'budget.state', 'open')]), ['A', 'B'])


# ============================== N: text / helpers ============================================================================
HB = {'id': BID(1), 'openedAt': ISO0, 'origin': 'user', 'state': 'open', 'limits': dict(UNIT), 'used': used(activeMs=125000, delegations=1, commands=2, reviews=1, checks=0)}
HBU = {**HB, 'limits': dict(UNIT), 'used': dict(HB['used']), 'unlimited': True}
HW = W([])


@fx('N01', 'all-counter-names-in-korean', 'B:68-74,131-133; spec 6.2 refusal')
def _():
    s = Script(); rs = [(s.call('refusal', r, HB, 'ko'), r) for r in REASONS]
    return (HW, s, E([R(r, refusal_text(k, HB, 'ko')) for r, k in rs], counts={'now': 0, 'findSession': 0}))


@fx('N02', 'all-counter-names-in-english', 'B:68-74,131-133; spec 6.2 refusal')
def _():
    s = Script(); rs = [(s.call('refusal', r, HB, 'en'), r) for r in REASONS]
    return (HW, s, E([R(r, refusal_text(k, HB, 'en')) for r, k in rs], counts={'now': 0}))


@fx('N03', 'omitted-and-undefined-language-default-to-korean', 'B:113,131,136,141; spec 6.1')
def _():
    s = Script()
    r1 = s.call('refusal', 'commands', HB); r2 = s.call('refusal', 'commands', HB, U); r3 = s.call('continueText', 'checks', HB); r4 = s.call('continueText', 'checks', HB, U)
    r5 = s.call('cardMissing'); r6 = s.call('cardMissing', U); r7 = s.call('limitText', HBU, 5); r8 = s.call('limitText', HB, 5, U)
    return (HW, s, E([R(r1, refusal_text('commands', HB)), R(r2, refusal_text('commands', HB)), R(r3, continue_text('checks', HB)), R(r4, continue_text('checks', HB)),
                      R(r5, CARD_MISSING['ko']), R(r6, CARD_MISSING['ko']), R(r7, NOLIMIT['ko']), R(r8, '5')]))


@fx('N04', 'minute-floor-boundaries', 'B:118-125; spec 6.2 amountText/refusalWire')
def _():
    s = Script(); rs = []
    for ms in (0, 59999, 60000, 119999, 120000):
        b = {**HB, 'used': used(activeMs=ms)}
        rs.append((s.call('refusalWire', 'time', b), s.call('refusal', 'time', b, 'en'), b))
    return (HW, s, E([x for w, t, b in rs for x in (R(w, wire('time', b)), R(t, refusal_text('time', b, 'en')))]))


@fx('N05', 'unlimited-text-versus-numeric-wire-limit', 'B:113-128; spec 6.2')
def _():
    s = Script()
    r1 = s.call('refusal', 'commands', HBU, 'ko'); r2 = s.call('refusal', 'commands', HBU, 'en'); r3 = s.call('refusal', 'time', HBU, 'ko'); r4 = s.call('refusalWire', 'commands', HBU)
    r5 = s.call('limitText', HBU, 3); r6 = s.call('limitText', HBU, 3, 'en'); r7 = s.call('limitText', HB, 3); r8 = s.call('limitText', HB, 0)
    return (HW, s, E([R(r1, refusal_text('commands', HBU, 'ko')), R(r2, refusal_text('commands', HBU, 'en')), R(r3, refusal_text('time', HBU, 'ko')), R(r4, wire('commands', HBU)),
                      R(r5, '제한 없음'), R(r6, 'no limit'), R(r7, '3'), R(r8, '0')]))


@fx('N06', 'refusal-exact-bytes-and-wire-keys', 'B:77-79,123-133; spec 6.2')
def _():
    s = Script(); a = s.call('refusal', 'time', HB, 'ko'); b = s.call('refusal', 'time', HB, 'en'); ws = [(s.call('refusalWire', r, HB), r) for r in REASONS]
    return (HW, s, E([R(a, refusal_text('time', HB, 'ko')), R(b, refusal_text('time', HB, 'en'))] + [R(w, wire(r, HB)) for w, r in ws]))


@fx('N07', 'continuation-exact-bytes', 'B:81-83,136-138; spec 6.2 continueText')
def _():
    s = Script(); rs = [(s.call('continueText', r, HB, lang), r, lang) for lang in ('ko', 'en') for r in REASONS]
    return (HW, s, E([R(x, continue_text(r, HB, lang)) for x, r, lang in rs]))


@fx('N08', 'card-missing-and-card-error-exact-bytes', 'B:84-107,141-143; spec 6.2')
def _():
    s = Script(); a = s.call('cardMissing', 'ko'); b = s.call('cardMissing', 'en')
    bads = {'deliverable': card(deliverable=''), 'done': card(done=A()), 'targets': card(targets=BAD_TARGETS), 'verify': card(verify=A(None)), 'conflicts': card(conflicts=A('')), 'state': card(state='idle')}
    exp = [R(a, CARD_MISSING['ko']), R(b, CARD_MISSING['en'])]
    for lang in ('ko', 'en'):
        exp.append(R(s.call('setCard', ref('nope', 1), card(), lang), {'ok': False, 'error': ERR[lang]['missing']}))
        for k, c in bads.items(): exp.append(R(s.call('setCard', ref('s1', 1), c, lang), {'ok': False, 'error': ERR[lang][k]}))
    return (W([sess('s1', budget(1))]), s, E(exp, session=[SA('s1', 'budget.card')]))


@fx('N09', 'budgetUnitOf-absent-and-configured', 'B:156-160; spec 6.2 budgetUnitOf')
def _():
    s = Script()
    r1 = s.call('budgetUnitOf'); r2 = s.call('budgetUnitOf', U); r3 = s.call('budgetUnitOf', None)
    r4 = s.call('budgetUnitOf', O(timeMinutes=30, delegations=5, commands=2, reviews=3, checks=4, extra='x')); r5 = s.call('budgetUnitOf', O(timeMinutes=30, delegations=5, commands=2, reviews=3, checks=4, defaultUnlimited=True))
    dflt = {**UNIT, 'defaultUnlimited': True}; conf = {'timeMinutes': 30, 'delegations': 5, 'commands': 2, 'reviews': 3, 'checks': 4}
    dk, ck = list(dflt), list(conf)
    return (HW, s, E([R(r1, dflt, keys=dk), R(r2, dflt, keys=dk), R(r3, dflt, keys=dk), R(r4, conf, keys=ck), R(r5, conf, keys=ck)], counts={'unit': 0}))


@fx('N10', 'helpers-do-not-consume-live-elapsed-time', 'B:110-138; spec 6.2 (no dependency call, no settle)')
def _():
    s = Script(); s.call('runStarted', ref('s1', 1), 'k1'); s.adv(90000)
    a = s.call('refusal', 'time', HB, 'ko'); b = s.call('refusalWire', 'time', HB); c = s.call('continueText', 'time', HB, 'en'); d = s.call('limitText', HB, 120)
    return (W([sess('s1', budget(1))]), s, E([R(a, refusal_text('time', HB, 'ko')), R(b, wire('time', HB)), R(c, continue_text('time', HB, 'en')), R(d, '120')],
            counts={'now': 1, 'timer.clear': 0, 'findSession': 2}, session=[SV('s1', 'budget.used.activeMs', 0), SV('s1', 'budget.activeSince', ISO0)]))


# ---- output -------------------------------------------------------------------------------------------------------------------
def check():
    seen = set()
    for g in GROUPS:
        ids = [f['id'] for f in FIX[g]]
        assert ids == [f'{g}{i:02d}' for i in range(1, 11)], (g, ids)
        for f in FIX[g]:
            assert f['id'] not in seen and f['rule'] and f['expect'] and f['script'], f['id']
            seen.add(f['id'])
    assert len(seen) == 140


def main():
    check()
    FIXTURES.mkdir(parents=True, exist_ok=True)
    for g in GROUPS:
        with open(FIXTURES / f'{g}.json', 'w', encoding='utf-8') as fh:
            json.dump(FIX[g], fh, ensure_ascii=True, indent=1)
            fh.write('\n')
    print(f'wrote {sum(len(v) for v in FIX.values())} fixtures to {FIXTURES}')


if __name__ == '__main__':
    main()
