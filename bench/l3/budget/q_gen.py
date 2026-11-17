"""L3-10 Q generator (spec section 11): random request-budget scenarios, deterministic from a seed.
  python3 q_gen.py [--out work/q/cases.jsonl] [--seed 20261117] [--per-family 250] [--families Q1,Q3] [--count N]
Seed 20261117, xorshift32 with rejection-sampled choices. The family list is Fisher-Yates shuffled with the seed; every case then gets its own
stream derived from (seed, family, ordinal), so a case does not depend on which other families are generated.
Default: 8 families x 250 cases. `--count N` yields N cases total, round-robin over the shuffled families.
Case: {id:"Q<f>-<seed>-<n>", family, seed, world, instances?, script}; format in SCENARIO.md. Scripts have 4-20 steps, keep valid and
retained stale references, and bias toward boundaries (limits, zero time, 300/301 units, list lengths 0/1/5/6, missing vs {undef:1}).
Domain rules: no dependency throws, no per-call `now` overrides, no halted budget without a `halt` field.
Importable: gen(seed, families, per_family, count=None) -> [case]; perf_case(axis, n) -> scenario (spec section 9 axes)."""
import sys
sys.dont_write_bytecode = True
import json
from pathlib import Path

from common import HERE
from gen_fixtures import A, ISO0, MIN, NEG0, O, T0, U, UNIT, num, u16

SEED = 20261117
FAMILIES = ['Q1', 'Q2', 'Q3', 'Q4', 'Q5', 'Q6', 'Q7', 'Q8']
KEYS = ('delegations', 'commands', 'reviews', 'checks')
LKEYS = ('timeMinutes',) + KEYS
REASONS = ['time', 'delegations', 'commands', 'reviews', 'checks']
ORIGINS = ['user', 'schedule', 'direct', 'release']
NOARG = object()          # marks an omitted trailing argument (the arity of a call is its args length)


# ---- random stream ------------------------------------------------------------------------------------------------------
class Rng:
    """xorshift32 (state never 0); uniform below(n) by rejection sampling (always consumes at least one word)."""
    def __init__(self, seed):
        self.s = (seed & 0xFFFFFFFF) or 1

    def next32(self):
        x = self.s
        x ^= (x << 13) & 0xFFFFFFFF
        x ^= x >> 17
        x ^= (x << 5) & 0xFFFFFFFF
        self.s = x
        return x

    def below(self, n):
        limit = (1 << 32) // n * n
        while True:
            x = self.next32()
            if x < limit:
                return x % n

    def pick(self, seq): return seq[self.below(len(seq))]
    def chance(self, num_, den=100): return self.below(den) < num_
    def between(self, lo, hi): return lo + self.below(hi - lo + 1)

    def shuffle(self, items):
        """Fisher-Yates."""
        for i in range(len(items) - 1, 0, -1):
            j = self.below(i + 1)
            items[i], items[j] = items[j], items[i]
        return items

    def wpick(self, table):
        x = self.below(sum(w for w, _ in table))
        for w, v in table:
            if x < w: return v
            x -= w


def case_rng(seed, fam_no, n):
    r = Rng((seed * 2654435761 + fam_no * 40503 + n * 2246822519) & 0xFFFFFFFF)
    for _ in range(4): r.next32()
    return r


# ---- strings ------------------------------------------------------------------------------------------------------------------
def units_of(text_):
    raw = text_.encode('utf-16-le')
    return [raw[i] | (raw[i + 1] << 8) for i in range(0, len(raw), 2)]


def skey_of(sid): return tuple(units_of(sid)) if isinstance(sid, str) else tuple(sid['u16'])


EM = '\U0001F600'
TEXT_OK = ['Ship it', 'x', '  padded  ', 'a' * 300, ' ' + 'a' * 300 + ' ', EM * 150, 'a' * 298 + EM, '\u0085', '​', 'Q', 'tab\there', '한글 산출물', u16(0xD800, 0x61), u16(0xDC00)]
TEXT_BAD = ['', '   ', 'a' * 301, EM * 151, 'a' * 299 + EM, '﻿', ' 　']
NONSTR = [5, None, U, True, 0, A(), O(a=1)]
ID_POOL = ['d1', 'd2', 'd3', 'dup', 'd-é', 'd' + EM, '', 'x' * 40]
UNICODE_IDS = ['s-é', 's-中文', 's-' + EM, 's-é', 's \u0085', 's-한글']


def text(r, bad_pct=25):
    if r.below(100) < bad_pct: return r.pick(TEXT_BAD) if r.chance(75) else r.pick(NONSTR)
    return r.pick(TEXT_OK)


def lines(r, lo, bad_pct=15):
    """A list of texts of length 0/1/5/6 (boundaries) or 2-4, at least `lo` long unless the draw is already a boundary above it."""
    n = max(r.pick([0, 1, 1, 5, 6, r.between(2, 4)]), lo)
    return A(*[text(r, bad_pct) for _ in range(n)])


def delegation_ids(r):
    n = r.pick([0, 1, 5, 6, r.between(2, 4), 1, 0, 3])
    return [r.pick(ID_POOL) if r.chance(80) else u16(0xD800, r.between(0x41, 0x5A)) for _ in range(n)]


# ---- budget model (tracks state, ids and timers well enough to keep references valid and timer tokens exact in Q3) -----------
class Bud:
    def __init__(self, bid, state='open', limits=None, used=None, unlimited=False, origin='user'):
        self.id, self.state, self.origin = bid, state, origin
        self.limits = dict(limits or UNIT)
        self.used = {'activeMs': 0, 'delegations': 0, 'commands': 0, 'reviews': 0, 'checks': 0, 'tools': 0, 'failures': 0, 'retries': 0}
        self.used.update(used or {})
        self.unlimited, self.unl_undef, self.ended_how, self.halt_reason = unlimited, False, None, None
        self.extra = []                         # [(name, JV)]: optional persisted fields after `used`

    def jv(self):
        pairs = [('id', self.id), ('openedAt', ISO0), ('origin', self.origin), ('state', self.state), ('limits', O(**self.limits))]
        if self.unlimited: pairs.append(('unlimited', True))
        elif self.unl_undef: pairs.append(('unlimited', U))
        pairs.append(('used', O(**self.used)))
        pairs += self.extra
        return {'obj': [[k, v] for k, v in pairs]}


class Clock:
    def __init__(self, skey): self.skey, self.runs, self.since, self.timer = skey, {}, None, None


class Sess:
    def __init__(self, skey, sidjv, bud): self.skey, self.sidjv, self.bud, self.plain = skey, sidjv, bud, isinstance(sidjv, str)

    def jv(self):
        pairs = [('id', self.sidjv)]
        if self.bud is not None: pairs.append(('budget', self.bud.jv()))
        return {'obj': [[k, v] for k, v in pairs]}


class Sim:
    """A compact mirror of the budget class decisions (open/halted/closed, clocks, timers). It only chooses arguments, so a mismatch with the
    oracle never breaks a case; it is exact for the Q3 operations, whose advances are restricted so timer token numbering is unambiguous."""
    def __init__(self, r, deps, unit, uuids=None):
        self.r, self.deps, self.unit, self.uuids = r, deps, dict(unit), uuids
        self.now, self.sess, self.clocks, self.timers, self.tokinfo = T0, {}, {}, {}, {}
        self.ntok, self.uuid_n, self.stale, self.next_ids = 0, 0, [], None

    def next_uuid(self):
        self.uuid_n += 1
        if self.uuids is not None: return self.uuids[self.uuid_n - 1]
        return f'00000000-0000-4000-8000-{self.uuid_n:012x}'

    def cur(self, skey, bid):
        s = self.sess.get(skey)
        b = s.bud if s is not None else None
        return s, (b if b is not None and b.id == bid else None)

    def cancel(self, tok): self.timers.pop(tok, None)

    def register(self, delay, skey, bid):
        eff = delay if 1 <= delay <= 2147483647 else 1
        self.ntok += 1
        self.timers[self.ntok] = self.now + eff
        self.tokinfo[self.ntok] = (skey, bid)
        return self.ntok

    def settle(self, b, skey):
        c = self.clocks.get(b.id)
        if c is None or c.skey != skey: return 0
        if c.timer is not None: self.cancel(c.timer)
        c.timer = None
        if c.since is None: return 0
        added = max(0, self.now - c.since)
        c.since = None
        b.used['activeMs'] += added
        return added

    def drop(self, bid):
        c = self.clocks.pop(bid, None)
        if c is not None and c.timer is not None: self.cancel(c.timer)

    def arm(self, skey, b, c):
        if b.unlimited: return
        left = b.limits['timeMinutes'] * MIN - b.used['activeMs']
        if left <= 0:
            self.fire(skey, b.id)
            return
        c.timer = self.register(left, skey, b.id)

    def refresh(self, skey, bid):
        s, b = self.cur(skey, bid)
        c = self.clocks.get(bid)
        if b is None or c is None: return
        consuming = b.state == 'open' and len(c.runs) > 0 and any(not w for w in c.runs.values())
        if consuming == (c.since is not None): return
        if not consuming:
            self.settle(b, skey)
            return
        c.since = self.now
        self.arm(skey, b, c)

    def fire(self, skey, bid):
        s, b = self.cur(skey, bid)
        c = self.clocks.get(bid)
        if c is not None: c.timer = None
        if b is None or b.state != 'open' or b.unlimited: return
        self.settle(b, skey)
        if c is not None and b.used['activeMs'] < b.limits['timeMinutes'] * MIN and len(c.runs) > 0:
            c.since = self.now
            self.arm(skey, b, c)
            return
        self.halt(skey, bid, 'time')

    def halt(self, skey, bid, reason):
        s, b = self.cur(skey, bid)
        if s is None or b is None or b.state != 'open': return
        self.settle(b, skey)
        ids = self.next_ids if self.next_ids is not None else delegation_ids(self.r)
        self.next_ids = None
        self.deps['runningDelegationIds'].append(ids)
        b.state, b.halt_reason = 'halted', reason
        b.extra = [(k, v) for k, v in b.extra if k != 'halt']

    def close(self, b, skey, how):
        self.settle(b, skey)
        b.state, b.ended_how = 'closed', how
        self.drop(b.id)

    def open(self, s, size):
        if s.bud is not None:
            self.stale.append((s.skey, s.sidjv, s.bud.id))
            self.drop(s.bud.id)
        chosen = size if size is not None else ('unlimited' if self.unit.get('defaultUnlimited') else 1)
        times = 1 if chosen == 'unlimited' else chosen
        s.bud = Bud(self.next_uuid(), 'open', {k: self.unit[k] * times for k in LKEYS}, unlimited=(chosen == 'unlimited'))

    def attach(self, s, size):
        if s.bud is not None and s.bud.state != 'closed': return
        self.open(s, size)

    def replace(self, s, size):
        b = s.bud
        if b is not None and b.state == 'halted':
            self.close(b, s.skey, 'user-message')
            self.open(s, size)
            return
        self.attach(s, size)

    def reopen(self, skey, bid):
        s, b = self.cur(skey, bid)
        if b is None or b.state != 'closed' or b.ended_how != 'quiet': return
        b.state, b.ended_how = 'open', None
        b.extra = [(k, v) for k, v in b.extra if k != 'ended']

    def charge(self, skey, bid, what):
        s, b = self.cur(skey, bid)
        if b is None or b.state != 'open': return
        for k in KEYS:
            amt = what.get(k) or 0
            if not b.unlimited and amt > 0 and b.used[k] + amt > b.limits[k]:
                self.halt(skey, bid, k)
                return
        for k in KEYS: b.used[k] += what.get(k) or 0
        b.used['retries'] += what.get('retries') or 0

    def run_started(self, skey, bid, key):
        s, b = self.cur(skey, bid)
        if b is None: return
        c = self.clocks.get(bid)
        if c is None: c = self.clocks[bid] = Clock(skey)
        c.runs[key] = False
        self.refresh(skey, bid)

    def run_waiting(self, skey, bid, key, waiting):
        c = self.clocks.get(bid)
        if c is None or key not in c.runs or c.runs[key] == waiting: return
        c.runs[key] = waiting
        self.refresh(skey, bid)

    def run_ended(self, skey, bid, key):
        c = self.clocks.get(bid)
        if c is None or key not in c.runs: return
        del c.runs[key]
        self.refresh(skey, bid)

    def raise_(self, skey, bid, size):
        s, b = self.cur(skey, bid)
        if b is None or b.state == 'closed': return
        c = self.clocks.get(bid)
        if size == 'unlimited':
            if b.unlimited: return
            b.unlimited = True
            if c is not None and c.timer is not None: self.cancel(c.timer)
            if c is not None: c.timer = None
            return
        if size == 1: return
        changed = time_raised = False
        for k in LKEYS:
            nxt = max(b.limits[k], size * self.unit[k])
            if nxt == b.limits[k]: continue
            b.limits[k] = nxt
            changed = True
            time_raised = time_raised or k == 'timeMinutes'
        if not changed: return
        if time_raised and c is not None and c.since is not None and not b.unlimited:
            self.settle(b, skey)
            self.refresh(skey, bid)

    def decide(self, skey, bid, choice, size):
        s = self.sess.get(skey)
        b = s.bud if s is not None else None
        if b is None or b.id != bid or b.state != 'halted': return
        if choice == 'extend':
            if size == 'unlimited': b.unlimited = True
            else:
                for k in LKEYS: b.limits[k] += (size or 1) * self.unit[k]
            b.state = 'open'
            b.extra = [(k, v) for k, v in b.extra if k != 'halt']
            self.refresh(skey, bid)
            self.deps['releaseHeld'].append(self.r.chance(50))
        else:
            self.close(b, skey, 'user-end')

    def maybe_close(self, skey):
        s = self.sess.get(skey)
        b = s.bud if s is not None else None
        if b is None or b.state != 'open': return
        c = self.clocks.get(b.id)
        if c is not None and len(c.runs) > 0: return
        quiet = self.r.chance(60)
        self.deps['isQuiet'].append(quiet)
        if quiet: self.close(b, skey, 'quiet')

    def boot_restore(self, skey):
        s = self.sess.get(skey)
        if s is not None and s.bud is not None: self.drop(s.bud.id)

    def forget_session(self, skey):
        for bid in [k for k, c in self.clocks.items() if c.skey == skey]: self.drop(bid)

    def dispose(self):
        for c in self.clocks.values():
            if c.timer is not None: self.cancel(c.timer)
        self.clocks.clear()

    def advance(self, ms):
        target = self.now + ms
        while True:
            due = sorted((d, t) for t, d in self.timers.items() if d <= target)
            if not due: break
            d, t = due[0]
            del self.timers[t]
            self.now = d
            self.fire(*self.tokinfo[t])
        self.now = target

    def deliver(self, tok):
        self.timers.pop(tok, None)
        self.fire(*self.tokinfo[tok])


# ---- scenario context ---------------------------------------------------------------------------------------------------------
def rand_unit(r, small_time=False):
    if r.chance(45) and not small_time: u = dict(UNIT)
    else: u = {'timeMinutes': r.pick([1, 2, 3] if small_time else [1, 2, 10, 30, 120]), 'delegations': r.pick([1, 5, 24]), 'commands': r.pick([1, 2, 3]),
               'reviews': r.pick([1, 2, 3]), 'checks': r.pick([1, 2, 4])}
    if not small_time and r.chance(15): u['defaultUnlimited'] = True
    return u


class Ctx:
    def __init__(self, r, small_time=False):
        self.r, self.steps, self.n, self.bn, self.small_time = r, [], 0, 0, small_time
        self.deps = {'runningDelegationIds': [], 'isQuiet': [], 'releaseHeld': []}
        self.unit = rand_unit(r, small_time)
        self.init_unit = dict(self.unit)
        self.uuids = [f'{r.next32():08x}-5555-4555-8555-{i:012x}' for i in range(1, 41)] if r.chance(15) else None
        self.sim = Sim(r, self.deps, self.unit, self.uuids)
        self.instances, self.clock_start, self.init_jv = None, None, []

    # -- world --------------------------------------------------------------------------------------------------------------
    def new_bid(self):
        self.bn += 1
        return f'{self.r.next32():08x}-0000-4000-8000-{self.bn:012x}'

    def rand_bud(self, state=None):
        r = self.r
        st = state or r.wpick([(50, 'open'), (25, 'halted'), (25, 'closed')])
        scale = r.pick([1, 1, 2, 4])
        limits = {k: self.unit[k] * scale for k in LKEYS}
        if self.small_time: limits['timeMinutes'] = r.pick([1, 2, 3])
        b = Bud(self.new_bid(), st, limits, origin=r.pick(ORIGINS))
        for k in KEYS:
            lm = limits[k]
            b.used[k] = r.pick([0, 0, 1, max(0, lm - 1), lm, min(lm, 2)])
        b.used['tools'], b.used['failures'], b.used['retries'] = r.pick([0, 0, 3]), r.pick([0, 0, 1]), r.pick([0, 0, 2])
        lmt = limits['timeMinutes'] * MIN
        b.used['activeMs'] = r.pick([0, 0, 0, 30000, max(0, lmt - 1000), max(0, lmt - 1), lmt] if self.small_time else [0, 0, 0, 59999, 60000, 125000, 30000])
        if r.chance(8) and st == 'open': b.unlimited = True
        elif r.chance(5): b.unl_undef = True
        if st == 'open' and r.chance(12): b.extra.append(('activeSince', ISO0))
        if st == 'halted':
            b.halt_reason = r.pick(REASONS)
            b.extra.append(('halt', O(reason=b.halt_reason, at=ISO0, stoppedDelegationIds=A(*delegation_ids(r)))))
        if st == 'closed':
            b.ended_how = r.pick(['quiet', 'quiet', 'user-end', 'user-message', None])
            if b.ended_how is not None: b.extra.append(('ended', O(how=b.ended_how, at=ISO0)))
        if r.chance(25): b.extra.append(('card', O(deliverable='d', done=A('x'), state=r.pick(['working', 'done']), by=O(sessionId='s1', turnId='t1'), updatedAt=ISO0)))
        if r.chance(25): b.extra.append(('autoContinue', O(sessionId=r.pick(['s1', 's2', 's3']), userTurnId='u1', count=r.pick([0, 1, 2, 5]))))
        c = r.below(100)
        if c < 8: b.extra.append(('follows', self.new_bid()))
        elif c < 13: b.extra.append(('follows', U))
        return b

    def add_session(self, sid=None):
        r, n = self.r, len(self.sim.sess)
        if sid is None:
            sid = f's{n + 1}' if n < 3 else f's{r.between(4, 99)}x'
            if r.chance(20): sid = r.pick(UNICODE_IDS)
        skey = skey_of(sid)
        if skey in self.sim.sess: return
        self.sim.sess[skey] = Sess(skey, sid, self.rand_bud())

    def snapshot_world(self):
        """Freeze the initial sessions (the model mutates its budgets as the script is generated)."""
        self.init_jv = [s.jv() for s in self.sim.sess.values()]

    def world_final(self):
        w = {'sessions': self.init_jv, 'unit': self.init_unit}
        if self.uuids: w['uuids'] = self.uuids
        if self.clock_start is not None: w['clockStart'] = self.clock_start
        d = {k: v for k, v in self.deps.items() if v}
        if d: w['deps'] = d
        return w

    # -- steps --------------------------------------------------------------------------------------------------------------
    def call(self, op, *args, i=None):
        self.n += 1
        step = {'ev': 'call', 'req': f'r{self.n}', 'op': op, 'args': list(args)}
        if i: step['i'] = i
        self.steps.append(step)

    def inst(self): return self.r.pick(self.instances) if self.instances else None
    def live(self, plain=False): return [s for s in self.sim.sess.values() if s.plain or not plain]
    def bogus_bid(self): return f'ffffffff-0000-4000-8000-{self.r.between(0, 3):012x}'

    def pick_ref(self):
        """(skey, sidjv, budgetId): mostly valid, sometimes a retained stale reference, a wrong budget id or a missing session."""
        r, sim = self.r, self.sim
        live = self.live()
        roll = r.below(100)
        if roll < 62 and live:
            s = r.pick(live)
            return s.skey, s.sidjv, (s.bud.id if s.bud is not None else self.bogus_bid())
        if roll < 77 and sim.stale: return r.pick(sim.stale)
        if roll < 88 and live:
            s = r.pick(live)
            return s.skey, s.sidjv, self.bogus_bid()
        sid = f'missing{r.between(0, 3)}'
        if roll < 95 and live:
            s = r.pick(live)
            return skey_of(sid), sid, (s.bud.id if s.bud is not None else self.bogus_bid())
        return skey_of(sid), sid, self.bogus_bid()

    @staticmethod
    def refjv(t): return O(sessionId=t[1], budgetId=t[2])

    def plain_session(self):
        live = self.live(plain=True)
        return self.r.pick(live) if live else None

    # -- emitters (each may emit nothing when its precondition does not hold) -----------------------------------------------------
    def e_attach(self):
        s = self.plain_session()
        if s is None: return
        size = self.r.pick([None, None, U, 1, 2, 4, 8, 'unlimited'])
        self.sim.attach(s, None if size is U else size)
        self.call('attach', {'sess': s.sidjv}, self.r.pick(ORIGINS), *([] if size is None else [size]))

    def e_replace(self):
        s = self.plain_session()
        if s is None: return
        r = self.r
        size, origin, k = r.pick([U, 1, 2, 4, 8, 'unlimited']), r.pick(ORIGINS), r.below(3)
        args = [] if k == 0 else [size] if k == 1 else [size, origin]
        self.sim.replace(s, None if (k == 0 or size is U) else size)
        self.call('replaceForUserMessage', {'sess': s.sidjv}, *args)

    def e_reopen(self):
        t = self.pick_ref()
        self.sim.reopen(t[0], t[2])
        self.call('reopen', self.refjv(t))

    def e_current(self): self.call(self.r.pick(['current', 'isHalted', 'hasCard']), self.refjv(self.pick_ref()))

    def what_for(self, t):
        r = self.r
        s, b = self.sim.cur(t[0], t[2])
        what = {}
        for k in r.pick([['commands'], ['delegations', 'commands'], ['reviews', 'checks'], list(KEYS), ['retries'], ['checks'], []]):
            if k == 'retries': continue
            rem = (b.limits[k] - b.used[k]) if b is not None else 1
            what[k] = r.pick([0, 1, 2, rem, rem + 1, max(rem - 1, 0), -1, None, U, 5])
        if r.chance(25): what['retries'] = r.pick([0, 1, -1, None, 3])
        return what

    def e_charge(self):
        t = self.pick_ref()
        what = self.what_for(t)
        self.sim.charge(t[0], t[2], {k: (None if v is U else v) for k, v in what.items()})
        self.call('charge', self.refjv(t), O(**what))

    def e_note_failure(self): self.call('noteFailure', self.refjv(self.pick_ref()))

    def e_note_tool(self):
        r = self.r
        self.call('noteTool', self.refjv(self.pick_ref()), r.pick(['t1', 't2', 'té', 't' + EM]), r.pick(['a1', 'a2', 'a3', 'a1', 'a-' + EM]))

    def e_forget_turn(self): self.call('forgetTurn', self.r.pick(['t1', 't2', 'té', 't' + EM]))

    def e_run(self):
        r, t = self.r, self.pick_ref()
        key = r.pick(['k0', 'k1', 'k2', 'k3'])
        c = r.wpick([(45, 'start'), (30, 'wait'), (25, 'end')])
        if c == 'start':
            self.sim.run_started(t[0], t[2], key)
            self.call('runStarted', self.refjv(t), key, i=self.inst())
        elif c == 'wait':
            w = r.chance(60)
            self.sim.run_waiting(t[0], t[2], key, w)
            self.call('runWaiting', self.refjv(t), key, w, i=self.inst())
        else:
            self.sim.run_ended(t[0], t[2], key)
            self.call('runEnded', self.refjv(t), key, i=self.inst())

    def e_halt(self, big=False):
        t, reason = self.pick_ref(), self.r.pick(REASONS)
        if big: self.sim.next_ids = [self.r.pick(ID_POOL) for _ in range(self.r.pick([64, 200]))]
        self.sim.halt(t[0], t[2], reason)
        self.call('halt', self.refjv(t), reason)

    def e_maybe_close(self):
        s = self.plain_session()
        if s is None: return
        self.sim.maybe_close(s.skey)
        self.call('maybeClose', {'sess': s.sidjv})

    def e_raise(self):
        t, size = self.pick_ref(), self.r.pick([1, 2, 2, 4, 8, 'unlimited', 'unlimited'])
        self.sim.raise_(t[0], t[2], size)
        self.call('raise', self.refjv(t), size)

    def e_decide(self):
        r = self.r
        halted = [s for s in self.sim.sess.values() if s.bud is not None and s.bud.state == 'halted']
        if r.chance(30) or not (halted or self.live()): t = self.pick_ref()
        else:
            s = r.pick(halted) if halted and r.chance(85) else r.pick(self.live())
            t = (s.skey, s.sidjv, s.bud.id if s.bud is not None else self.bogus_bid())
        choice, size = r.pick(['extend', 'extend', 'extend', 'end']), r.pick([None, None, U, 1, 2, 4, 8, 'unlimited'])
        self.sim.decide(t[0], t[2], choice, None if size is U else size)
        self.call('decide', t[1], t[2], choice, *([] if size is None else [size]))

    def e_advance(self):
        ms = self.r.pick([0, 1, 1000, 59999, 60000, 30000, 120000, 5000])
        self.sim.advance(ms)
        self.steps.append({'ev': 'advance', 'ms': ms})

    def e_advance_safe(self):
        """Q3: only advances that land exactly on the next due timer or short of it, so token numbering stays unambiguous."""
        r, sim = self.r, self.sim
        nxt = min(sim.timers.values()) if sim.timers else None
        if nxt is not None and r.chance(55): ms = max(0, nxt - sim.now)
        elif nxt is not None: ms = r.pick([0, 1, max(0, (nxt - sim.now) // 2), max(0, nxt - sim.now - 1)])
        else: ms = r.pick([0, 1000, 60000, 30000])
        sim.advance(ms)
        self.steps.append({'ev': 'advance', 'ms': ms})

    def e_set_time(self):
        """Q3: a forward jump (overshoot) is followed by explicit delivery of every overdue timer in (due, token) order; a backward jump fires nothing."""
        r, sim = self.r, self.sim
        if r.chance(35): t = sim.now - r.pick([1000, 5000, 61000])
        else: t = sim.now + r.pick([1000, 30000, 61000, 130000])
        forward = t > sim.now
        sim.now = t
        self.steps.append({'ev': 'setTime', 'now': t})
        if not forward: return
        for due, tok in sorted((d, k) for k, d in sim.timers.items() if d < t):
            if tok not in sim.timers: continue
            self.steps.append({'ev': 'deliverTimer', 'token': f't{tok}'})
            sim.deliver(tok)

    def e_deliver(self):
        sim = self.sim
        if sim.ntok == 0: return
        tok = self.r.between(1, sim.ntok)
        step = {'ev': 'deliverTimer', 'token': f't{tok}'}
        if tok not in sim.timers: step['captured'] = True
        self.steps.append(step)
        sim.deliver(tok)

    def e_put_session(self):
        s = self.plain_session()
        if s is None: return
        if s.bud is not None: self.sim.stale.append((s.skey, s.sidjv, s.bud.id))
        s.bud = self.rand_bud() if self.r.chance(80) else None
        self.steps.append({'ev': 'putSession', 'session': s.jv()})

    def e_remove_session(self):
        live = self.live(plain=True)
        if len(live) < 2: return
        s = self.r.pick(live)
        if s.bud is not None: self.sim.stale.append((s.skey, s.sidjv, s.bud.id))
        del self.sim.sess[s.skey]
        self.steps.append({'ev': 'removeSession', 'id': s.sidjv})

    def e_set_unit(self):
        u = rand_unit(self.r, self.small_time)
        self.sim.unit = dict(u)
        self.steps.append({'ev': 'setUnit', 'unit': u})

    def run_until(self, table, lo, hi):
        n, guard = self.r.between(lo, hi), 0
        while self.n < n and guard < 200 and len(self.steps) < 20:
            guard += 1
            self.r.wpick(table)()


# ---- card generation (Q6, Q7, Q8) ---------------------------------------------------------------------------------------------
COUNT_OK = [0, 1, 2, 3, 7]
COUNT_ODD = [-1, num(1.5), NEG0, num(2 ** 53), num(1e300), num(float('nan')), num(float('inf')), num(float('-inf')), '1', None, U, 4]


def gen_targets(r):
    c = r.below(10)
    if c == 0: return U
    if c == 1: return r.pick([None, 5, 'str', O()])
    counts = [r.pick(COUNT_OK) if r.chance(75) else r.pick(COUNT_ODD) for _ in range(3)]
    pairs = [('confirmed', counts[0]), ('candidate', counts[1]), ('unknown', counts[2])]
    if r.chance(10): pairs.pop(r.below(3))
    if r.below(10) >= 3: pairs.append(('note', r.pick([U, '', 'a' * 300, 'a' * 301, '   ', ' x ', 'n', 0, None, EM * 150, u16(0xD800)])))
    return {'obj': [[k, v] for k, v in pairs]}


def gen_verify(r):
    items = []
    for _ in range(r.pick([0, 1, 1, 5, 6, 2])):
        if r.below(10) == 0: items.append(r.pick([None, 'x', 5, O(label='x'), O(label='', status='done'), O(label='x', status='bad'), O(label='a' * 301, status='planned')]))
        else: items.append(O(label=r.pick(TEXT_OK[:9]), status=r.pick(['planned', 'done', 'skipped'])))
    return A(*items)


def gen_card(r):
    bad = r.chance(45)
    kv = [('deliverable', text(r, 30 if bad else 4))]
    if r.chance(95): kv.append(('done', lines(r, 1, 25 if bad else 3) if r.chance(95) else r.pick(['x', 5, None, U])))
    if r.chance(45): kv.append(('targets', gen_targets(r)))
    if r.chance(40): kv.append(('verify', gen_verify(r) if r.chance(92) else r.pick(['x', 5, None, U])))
    if r.chance(35): kv.append(('conflicts', lines(r, 0, 20) if r.chance(92) else r.pick(['x', 5, None, U])))
    if r.chance(40): kv.append(('state', r.pick(['working', 'done', 'waiting-user', 'blocked', 'working', 'idle', '', 5, U, None])))
    if r.chance(15): kv.append(('updatedAt', 'bogus'))
    if r.chance(10): kv.append(('extra', 1))
    if r.chance(8): kv.append(('by', O(sessionId='zz', turnId='zz')))
    return {'obj': [[k, v] for k, v in kv]}


def e_set_card(c):
    r = c.r
    args = [c.refjv(c.pick_ref()), gen_card(r)]
    lang = r.pick([NOARG, 'ko', 'en', U, 'ko'])
    by = r.pick([NOARG, NOARG, NOARG, O(sessionId='s9', turnId='t9'), O(sessionId='s1', turnId=U), O(), None, U])
    if by is not NOARG: args += [U if lang is NOARG else lang, by]
    elif lang is not NOARG: args.append(lang)
    c.call('setCard', *args)


# ---- families -----------------------------------------------------------------------------------------------------------------
def begin(r, small_time=False, nsess=None, unicode_ids=False):
    c = Ctx(r, small_time)
    for _ in range(nsess or r.between(1, 3)): c.add_session()
    if unicode_ids and r.chance(25): c.add_session(sid=r.pick(UNICODE_IDS))
    if r.chance(8):
        c.clock_start = r.pick([0, 1000, T0 + 1])
        c.sim.now = c.clock_start
    return c


def fam_q1(r):
    """Opening, replacement, reopen, stale references, origins, sizes."""
    c = begin(r, unicode_ids=True)
    c.snapshot_world()
    c.run_until([(26, c.e_attach), (26, c.e_replace), (15, c.e_reopen), (6, c.e_current), (5, c.e_halt), (5, c.e_decide), (4, c.e_maybe_close), (4, c.e_put_session),
                 (3, c.e_remove_session), (4, c.e_set_unit), (2, c.e_advance)], 4, r.between(6, 16))
    return c


def fam_q2(r):
    """Charges, atomicity, tools, failures, retries."""
    c = begin(r)
    c.snapshot_world()
    c.run_until([(46, c.e_charge), (8, c.e_note_failure), (20, c.e_note_tool), (8, c.e_forget_turn), (4, c.e_current), (4, c.e_reopen), (3, c.e_halt)], 5, r.between(8, 18))
    return c


def fam_q3(r):
    """Multi-run waiting/running transitions and timer delivery (token numbering is exact: see e_advance_safe, e_set_time)."""
    c = begin(r, small_time=True)
    c.clock_start = None
    c.sim.now = T0
    for s in c.sim.sess.values():                          # clock family: mostly open, limited budgets with small time limits
        b = s.bud
        if b.state != 'open' and r.chance(80):
            b.state, b.halt_reason, b.ended_how = 'open', None, None
            b.extra = [(k, v) for k, v in b.extra if k not in ('halt', 'ended')]
    c.snapshot_world()
    c.run_until([(38, c.e_run), (20, c.e_advance_safe), (8, c.e_set_time), (10, c.e_deliver), (6, c.e_raise), (3, c.e_charge)], 5, r.between(8, 16))
    return c


def fam_q4(r):
    """Halt, quiet close, delegation order, nested halts (charge/fire/decide re-halt)."""
    c = begin(r, small_time=r.chance(40))
    c.snapshot_world()
    c.run_until([(26, c.e_halt), (18, c.e_maybe_close), (22, c.e_run), (14, c.e_charge), (5, c.e_advance), (5, c.e_current), (5, c.e_decide), (3, c.e_reopen)], 5, r.between(8, 18))
    return c


def fam_q5(r):
    """Raise/decide combinations, retained runs, queue/release order."""
    c = begin(r, small_time=r.chance(60))
    for s in c.sim.sess.values():                          # mostly halted budgets so decide has work to do
        b = s.bud
        if b.state == 'closed' and r.chance(70):
            b.state, b.ended_how, b.halt_reason = 'halted', None, r.pick(REASONS)
            b.extra = [(k, v) for k, v in b.extra if k != 'ended'] + [('halt', O(reason=b.halt_reason, at=ISO0, stoppedDelegationIds=A()))]
    c.snapshot_world()
    c.run_until([(30, c.e_decide), (26, c.e_raise), (16, c.e_run), (8, c.e_advance), (5, c.e_halt), (5, c.e_charge), (6, c.e_set_unit)], 5, r.between(8, 16))
    return c


def fam_q6(r):
    """Card validation, UTF-16, optional fields, Number classification."""
    c = begin(r)
    c.snapshot_world()
    c.run_until([(70, lambda: e_set_card(c)), (10, c.e_current), (5, c.e_advance), (4, c.e_halt)], 4, r.between(6, 14))
    return c


def ac_record(r):
    c = r.below(10)
    if c == 0: return None
    if c == 1: return U
    pairs = [('sessionId', r.pick(['s1', 's2', 's3', 'other'])), ('userTurnId', r.pick(['u1', '', 'u2'])), ('count', r.pick([0, 1, 2, 3, 99]))]
    if r.chance(30): pairs.append(('lastFrom', 'tx'))
    if r.chance(25): pairs.append(('stopped', True))
    return {'obj': [[k, v] for k, v in pairs]}


def fam_q7(r):
    """Auto-continuation ownership, restoration, cleanup, multiple instances."""
    c = begin(r, nsess=r.between(2, 3))
    if r.chance(40): c.instances = ['A', 'B']
    c.snapshot_world()

    def e_clear(): c.call('clearAutoContinue', c.refjv(c.pick_ref()))
    def e_record(): c.call('recordAutoContinue', c.refjv(c.pick_ref()), ac_record(r))

    def e_stop():
        live = c.live()
        sid = r.pick([s.sidjv for s in live] + ['nobody'])
        k = r.below(5)
        if k <= 1: c.call('stopAutoContinue', sid)
        elif k == 4: c.call('stopAutoContinue', sid, U)
        else: c.call('stopAutoContinue', sid, c.refjv(c.pick_ref()))

    def e_boot():
        s = c.plain_session()
        if s is None: return
        c.sim.boot_restore(s.skey)
        c.call('bootRestore', {'sess': s.sidjv}, i=c.inst())

    def e_forget():
        sid = r.pick([s.sidjv for s in c.live()] + ['nobody'])
        c.sim.forget_session(skey_of(sid))
        c.call('forgetSession', sid, i=c.inst())

    def e_dispose():
        c.sim.dispose()
        c.call('dispose', i=c.inst())

    c.run_until([(14, e_clear), (16, e_record), (22, e_stop), (10, e_boot), (8, e_forget), (5, e_dispose), (10, c.e_run), (5, lambda: e_set_card(c)), (3, c.e_advance),
                 (3, c.e_attach), (3, c.e_charge)], 5, r.between(8, 18))
    return c


def helper_budget(r):
    lim = {'timeMinutes': r.pick([0, 1, 120, 99999]), 'delegations': r.pick([0, 24]), 'commands': r.pick([0, 3, 99]), 'reviews': r.pick([2, 1]), 'checks': r.pick([1, 0])}
    used = {'activeMs': r.pick([0, 59999, 60000, 119999, 120000, 125000, 9007199254740991]), 'delegations': r.pick([0, 1, 24]), 'commands': r.pick([0, 2, 3, 100]),
            'reviews': r.pick([0, 1, 2]), 'checks': r.pick([0, 1, 2]), 'tools': 0, 'failures': 0, 'retries': 0}
    b = {'id': f'aaaaaaaa-0000-4000-8000-{r.between(1, 9):012x}', 'openedAt': ISO0, 'origin': r.pick(ORIGINS), 'state': r.pick(['open', 'halted', 'closed']), 'limits': lim}
    if r.chance(35): b['unlimited'] = True
    b['used'] = used
    return b


def e_helper(c):
    r = c.r
    k, lang, b, reason = r.below(8), r.pick([NOARG, 'ko', 'en', U]), helper_budget(r), r.pick(REASONS)
    tail = [] if lang is NOARG else [lang]
    if k <= 1: c.call('refusal', reason, b, *tail)
    elif k == 2: c.call('continueText', reason, b, *tail)
    elif k == 3: c.call('refusalWire', reason, b)
    elif k == 4: c.call('limitText', b, r.pick([0, 1, 3, 120, 99999]), *tail)
    elif k == 5: c.call('cardMissing', *tail)
    else:
        full = O(timeMinutes=30, delegations=5, commands=2, reviews=3, checks=4)
        c.call('budgetUnitOf', *r.pick([[], [U], [None], [full], [O(timeMinutes=30, delegations=5, commands=2, reviews=3, checks=4, defaultUnlimited=True, extra='x')]]))


def fam_q8(r):
    """Text/helpers, admitted compatibility values (null/negative amounts, {undef:1} options, odd cards), larger inputs (64/200 delegation ids, long texts)."""
    c = begin(r, unicode_ids=True)
    c.snapshot_world()
    pct = r.pick([20, 50, 80])

    def e_compat():
        k = r.below(7)
        if k == 0: c.e_attach()
        elif k == 1: c.e_charge()
        elif k == 2: c.e_note_tool()
        elif k == 3: e_set_card(c)
        elif k == 4: c.e_reopen()
        elif k == 5: c.e_halt(big=r.chance(25))
        else: c.e_current()

    c.run_until([(pct, lambda: e_helper(c)), (100 - pct, e_compat)], 4, r.between(6, 14))
    return c


FAMILY_FN = {'Q1': fam_q1, 'Q2': fam_q2, 'Q3': fam_q3, 'Q4': fam_q4, 'Q5': fam_q5, 'Q6': fam_q6, 'Q7': fam_q7, 'Q8': fam_q8}


def build(fam, seed, n):
    c = FAMILY_FN[fam](case_rng(seed, int(fam[1:]), n))
    case = {'id': f'{fam}-{seed}-{n}', 'family': fam, 'seed': seed, 'world': c.world_final(), 'script': c.steps[:20]}
    if c.instances: case['instances'] = c.instances
    return case


# ---- public API ---------------------------------------------------------------------------------------------------------------
def gen(seed=SEED, families=None, per_family=250, count=None):
    """The case list: families in Fisher-Yates order; per_family cases each, or `count` cases total round-robin over the shuffled families."""
    fams = list(families or FAMILIES)
    Rng(seed).shuffle(fams)
    out, ordinal = [], {f: 0 for f in fams}
    if count is None:
        for f in fams:
            for _ in range(per_family):
                ordinal[f] += 1
                out.append(build(f, seed, ordinal[f]))
    else:
        for i in range(count):
            f = fams[i % len(fams)]
            ordinal[f] += 1
            out.append(build(f, seed, ordinal[f]))
    return out


def perf_case(axis, n):
    """One scenario for a spec-9 axis: sessions | calls | runs | delegations | clocks (n = the axis size)."""
    def bud(i, limits=None):
        b = {'id': f'bbbbbbbb-0000-4000-8000-{i:012x}', 'openedAt': ISO0, 'origin': 'user', 'state': 'open', 'limits': {**UNIT, **(limits or {})},
             'used': {'activeMs': 0, 'delegations': 0, 'commands': 0, 'reviews': 0, 'checks': 0, 'tools': 0, 'failures': 0, 'retries': 0}}
        return b

    def ref(i): return {'sessionId': f's{i}', 'budgetId': f'bbbbbbbb-0000-4000-8000-{i:012x}'}

    steps = []
    def call(op, *args): steps.append({'ev': 'call', 'req': f'r{len(steps) + 1}', 'op': op, 'args': list(args)})

    if axis == 'sessions':
        world = {'sessions': [{'id': f's{i}', 'budget': bud(i)} for i in range(n)], 'unit': dict(UNIT)}
        fixed = n // 2
        for k in range(3): call('charge', ref(fixed), {'commands': 0, 'reviews': 0, 'retries': k})
        call('noteTool', ref(fixed), 't1', 'a1'); call('current', ref(fixed)); call('hasCard', ref(fixed)); call('noteFailure', ref(fixed))
        call('runStarted', ref(fixed), 'k1'); call('runEnded', ref(fixed), 'k1')
    elif axis == 'calls':
        world = {'sessions': [{'id': 's0', 'budget': bud(0, {k: 10 ** 9 for k in LKEYS})}], 'unit': dict(UNIT)}
        for k in range(n):
            call('charge', ref(0), {'commands': 1, 'reviews': 1})
            call('noteTool', ref(0), 't1', f'act{k}')
    elif axis == 'runs':
        world = {'sessions': [{'id': 's0', 'budget': bud(0)}], 'unit': dict(UNIT)}
        for k in range(n): call('runStarted', ref(0), f'k{k}')
        call('runWaiting', ref(0), 'k0', True)                      # the measured refresh over R = n runs
    elif axis == 'delegations':
        world = {'sessions': [{'id': 's0', 'budget': bud(0)}], 'unit': dict(UNIT), 'deps': {'runningDelegationIds': [[f'd{k}' for k in range(n)]]}}
        call('halt', ref(0), 'commands')
    elif axis == 'clocks':
        world = {'sessions': [{'id': f's{i}', 'budget': bud(i)} for i in range(n)], 'unit': dict(UNIT)}
        for i in range(n): call('runStarted', ref(i), 'k0')
        call('forgetSession', 's0')
        call('dispose')
    else:
        raise ValueError(f'unknown perf axis {axis!r}')
    return {'id': f'perf-{axis}-{n}', 'domain': 'perf', 'world': world, 'script': steps}


def main(argv):
    def opt(name, default=None): return argv[argv.index(name) + 1] if name in argv else default
    out = Path(opt('--out', str(HERE / 'work' / 'q' / 'cases.jsonl')))
    seed = int(opt('--seed', SEED))
    fams = [f.strip() for f in opt('--families').split(',')] if opt('--families') else FAMILIES
    for f in fams:
        if f not in FAMILIES: raise SystemExit(f'unknown family {f}')
    count = opt('--count')
    cases = gen(seed, fams, int(opt('--per-family', 250)), int(count) if count is not None else None)
    out.parent.mkdir(parents=True, exist_ok=True)
    with open(out, 'w', encoding='utf-8') as f:
        for case in cases: f.write(json.dumps(case, ensure_ascii=True, separators=(',', ':')) + '\n')
    from collections import Counter
    print(f'wrote {len(cases)} cases to {out} (seed {seed}): ' + ' '.join(f'{k}={v}' for k, v in sorted(Counter(c["family"] for c in cases).items())))


if __name__ == '__main__':
    main(sys.argv)
