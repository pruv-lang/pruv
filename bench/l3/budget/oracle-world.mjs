// Dependency world of the L3-10 request-budget oracle (docs/REQUEST_BUDGET_SPEC.md sections 3, 7). Imported by reference.mjs and, through
// fakes/node-crypto.ts, by the pinned class. Not run on its own; the oracle runs as
//   GROK_TS_TRANSPILER=/path/to/typescript node --experimental-strip-types --no-warnings --import ./reference-hooks.mjs reference.mjs [--time [--repeat N]] < scenarios.jsonl > observations.jsonl
// Contents: the JV codec (decode d, encode enc), one scenario world (sessions, unit, virtual clock, UUID stream, fake timers, scripted dep
// replies), the dependency fakes handed to every RequestBudgets instance, the trace recorder and the observation helpers.
// One world per scenario, shared by all instances. Everything is synchronous.

export class OracleError extends Error {
  constructor(message) { super(message); this.name = 'OracleError'; }
}

/** The world of the scenario being run (set by reference.mjs). */
export const W = { s: null };
export const DEFAULT_CLOCK_START = 1700000000000;
const REPLY_DEPS = ['runningDelegationIds', 'isQuiet', 'releaseHeld'];
const realSetTimeout = globalThis.setTimeout;
const realClearTimeout = globalThis.clearTimeout;

// ---- JV codec ---------------------------------------------------------------------------------------------------------------
// Input: plain JSON string/bool/null/number, {u16:[units]}, {undef:1}, {num:"16 hex digits"}, {arr:[...]}, {obj:[[k,v],...]},
// {sess:"id"} (the currently stored session object), plus plain JSON arrays/objects.
// Output: strings (well formed) / bool / null / safe integers plain; {u16}, {undef:1}, {num} as above; arrays as plain JSON arrays;
// objects as {obj:[[k,v],...]} in insertion order (absent keys absent, present-undefined as {undef:1}).
const MARKERS = new Set(['u16', 'undef', 'num', 'arr', 'obj', 'sess']);
const bitsView = new DataView(new ArrayBuffer(8));
const numBits = x => { bitsView.setFloat64(0, x); return bitsView.getBigUint64(0).toString(16).padStart(16, '0'); };
const bitsNum = hex => { bitsView.setBigUint64(0, BigInt(`0x${hex}`)); return bitsView.getFloat64(0); };
const isObj = v => v !== null && typeof v === 'object' && !Array.isArray(v);

export function d(jv) {
  if (jv === null || typeof jv !== 'object') return jv;
  if (Array.isArray(jv)) return jv.map(item => d(item));
  const keys = Object.keys(jv);
  if (keys.length === 1 && MARKERS.has(keys[0])) {
    const key = keys[0];
    const v = jv[key];
    switch (key) {
      case 'undef': return undefined;
      case 'u16': {
        if (!Array.isArray(v)) throw new OracleError('bad {u16}');
        let out = '';
        for (let i = 0; i < v.length; i += 8192) out += String.fromCharCode(...v.slice(i, i + 8192));
        return out;
      }
      case 'num':
        if (typeof v !== 'string' || !/^[0-9a-fA-F]{16}$/.test(v)) throw new OracleError('bad {num}');
        return bitsNum(v);
      case 'arr':
        if (!Array.isArray(v)) throw new OracleError('bad {arr}');
        return v.map(item => d(item));
      case 'obj':
        if (!Array.isArray(v)) throw new OracleError('bad {obj}');
        return Object.fromEntries(v.map(pair => { if (!Array.isArray(pair) || pair.length !== 2) throw new OracleError('bad {obj} pair'); return [d(pair[0]), d(pair[1])]; }));
      case 'sess': {
        const found = W.s.sessions.find(x => isObj(x) && x.id === v);
        if (found === undefined) throw new OracleError(`{sess:${JSON.stringify(v)}} names no stored session`);
        return found;
      }
    }
  }
  return Object.fromEntries(keys.map(k => [k, d(jv[k])]));
}

const encStr = str => (str.isWellFormed() ? str : { u16: Array.from({ length: str.length }, (_, i) => str.charCodeAt(i)) });
const encNum = x => (Number.isSafeInteger(x) && !Object.is(x, -0) ? x : { num: numBits(x) });

/** Encode any value. Stored session objects become {sref, v} snapshots; budgets already seen as budgets become {bref, v}. */
export function enc(v) {
  switch (typeof v) {
    case 'undefined': return { undef: 1 };
    case 'boolean': return v;
    case 'string': return encStr(v);
    case 'number': return encNum(v);
    case 'object': {
      if (v === null) return null;
      if (Array.isArray(v)) { const out = new Array(v.length); for (let i = 0; i < v.length; i++) out[i] = enc(v[i]); return out; }
      const s = W.s;
      if (s.isSess.has(v)) return encSession(v);
      if (s.isBud.has(v)) return encBudget(v);
      return encPlain(v);
    }
    default: return { other: typeof v };
  }
}

const encPlain = v => ({ obj: Object.keys(v).map(k => [encStr(k), enc(v[k])]) });

function encSession(sess) {
  const s = W.s;
  let label = s.sLabel.get(sess);
  if (label === undefined) s.sLabel.set(sess, label = `s${++s.sSeq}`);
  return { sref: label, v: { obj: Object.keys(sess).map(k => [encStr(k), k === 'budget' && isObj(sess[k]) ? encBudget(sess[k]) : enc(sess[k])]) } };
}

function encBudget(budget) {
  const s = W.s;
  s.isBud.add(budget);
  let label = s.bLabel.get(budget);
  if (label === undefined) s.bLabel.set(budget, label = `b${++s.bSeq}`);
  return { bref: label, v: encPlain(budget) };
}

/** Terminal value of a thrown call: {error:{name,message}} for Error objects, else the JV of the thrown value. */
export function encThrown(error) {
  return error instanceof Error ? { error: { name: enc(error.name), message: enc(error.message) } } : enc(error);
}

/** Encode the return value of a call step: budgets handed out by current / setCard use the {bref,v} form. */
export function encReturn(op, value) {
  const s = W.s;
  if (op === 'current' && isObj(value)) s.isBud.add(value);
  else if (op === 'setCard' && isObj(value) && isObj(value.budget)) s.isBud.add(value.budget);
  return enc(value);
}

export function registerSession(session) {
  if (isObj(session)) W.s.isSess.add(session);
  return session;
}

// ---- world state ------------------------------------------------------------------------------------------------------------
export function createState(scenario, tracing) {
  return {
    tracing, scenario, trace: [], violations: [], excluded: [], cur: { r: null, i: null }, nowOverride: [],
    now: 0, unit: undefined, sessions: [], uuids: null, uuidN: 0,
    queues: { runningDelegationIds: [], isQuiet: [], releaseHeld: [] }, throws: Object.create(null),
    depCalls: Object.create(null), perOp: Object.create(null), fireNs: 0n, calls: 0, coreNs: 0n,
    timers: new Map(), allTimers: new Map(), timerSeq: 0, timerHasUnref: true,
    isSess: new WeakSet(), isBud: new WeakSet(), sLabel: new WeakMap(), bLabel: new WeakMap(), sSeq: 0, bSeq: 0, instances: new Map(),
  };
}

/** Decode the world of the scenario into the state (W.s must already be this state). */
export function initWorld(s) {
  const world = s.scenario.world ?? {};
  s.now = d(world.clockStart ?? DEFAULT_CLOCK_START);
  s.timerHasUnref = world.timerHasUnref ?? true;
  s.sessions = (world.sessions ?? []).map(x => registerSession(d(x)));
  s.unit = d(world.unit);
  if (world.uuids !== undefined) s.uuids = world.uuids.map(x => d(x));
  const deps = world.deps ?? {};
  for (const name of REPLY_DEPS) if (deps[name] !== undefined) s.queues[name] = deps[name].map(x => d(x));
  for (const [name, list] of Object.entries(deps.throws ?? {})) s.throws[name] = new Set(list);
}

/** Record a trace record. a and o are thunks (evaluated only when tracing, immediately, so records are deep copies at record time). */
export function trace(c, a, o) {
  const s = W.s;
  if (!s.tracing) return;
  const entry = { c, r: s.cur.r, i: s.cur.i };
  if (a !== undefined) { const value = a(); if (value !== undefined) entry.a = value; }
  if (o !== undefined) { const value = o(); if (value !== undefined) entry.o = value; }
  s.trace.push(entry);
}

const count = (s, name) => (s.depCalls[name] = (s.depCalls[name] ?? 0) + 1);

// ---- UUID stream ------------------------------------------------------------------------------------------------------------
export function randomUUID() {
  const s = W.s;
  count(s, 'uuid');
  let value;
  if (s.uuids !== null) {
    if (s.uuidN >= s.uuids.length) throw new OracleError('uuid stream exhausted');
    value = s.uuids[s.uuidN++];
  } else {
    value = `00000000-0000-4000-8000-${(++s.uuidN).toString(16).padStart(12, '0')}`;
  }
  trace('uuid', undefined, () => enc(value));
  return value;
}

// ---- fake timers ------------------------------------------------------------------------------------------------------------
class FakeTimer { constructor(token) { this.token = token; } }
class FakeTimerUnref extends FakeTimer {
  unref() {
    const s = W.s;
    count(s, 'timer.unref');
    trace('timer.unref', () => enc({ token: this.token }));
    return this;
  }
}

function fakeSetTimeout(callback, delay) {
  const s = W.s;
  count(s, 'setTimeout');
  const token = `t${++s.timerSeq}`;
  const effective = delay >= 1 && delay <= 2147483647 ? delay : 1; // Node's rule
  const due = s.now + effective;
  const timer = s.timerHasUnref ? new FakeTimerUnref(token) : new FakeTimer(token);
  const entry = { token, callback, delay, due, seq: s.timerSeq, i: s.cur.i };
  s.timers.set(token, entry);
  s.allTimers.set(token, entry);
  trace('timer.set', () => enc({ token, delay, due }));
  return timer;
}

function fakeClearTimeout(timer) {
  const s = W.s;
  count(s, 'clearTimeout');
  const token = isObj(timer) && typeof timer.token === 'string' ? timer.token : null;
  if (token !== null) s.timers.delete(token);
  trace('timer.clear', () => enc({ token }));
}

export function installGlobals() { globalThis.setTimeout = fakeSetTimeout; globalThis.clearTimeout = fakeClearTimeout; }
export function restoreGlobals() { globalThis.setTimeout = realSetTimeout; globalThis.clearTimeout = realClearTimeout; }

function runTimer(entry) {
  const s = W.s;
  const saved = s.cur;
  s.cur = { r: null, i: entry.i };
  trace('timer.fire', () => enc({ token: entry.token }));
  const t0 = process.hrtime.bigint();
  try {
    entry.callback();
  } catch (error) {
    if (error instanceof OracleError) throw error;
    trace('timer.throw', undefined, () => encThrown(error));
  } finally {
    s.fireNs += process.hrtime.bigint() - t0;
    s.cur = saved;
    s.nowOverride = [];
  }
}

/** Move the clock forward by ms, firing due timers in (due, registration order); now = due before each fire. */
export function advance(ms) {
  const s = W.s;
  if (typeof ms !== 'number' || Number.isNaN(ms)) throw new OracleError('advance needs a numeric ms');
  const target = s.now + ms;
  for (;;) {
    let best = null;
    for (const timer of s.timers.values()) {
      if (timer.due <= target && (best === null || timer.due < best.due || (timer.due === best.due && timer.seq < best.seq))) best = timer;
    }
    if (best === null) break;
    s.timers.delete(best.token);
    s.now = best.due;
    runTimer(best);
  }
  s.now = target;
}

/** Run one timer callback now. Active: removed then run. Inactive: only with captured:true, else a driver violation. */
export function deliverTimer(token, captured) {
  const s = W.s;
  const active = s.timers.get(token);
  if (active !== undefined) {
    s.timers.delete(token);
    runTimer(active);
    return;
  }
  const known = s.allTimers.get(token);
  if (known === undefined) throw new OracleError(`deliverTimer: unknown token ${JSON.stringify(token)}`);
  if (captured !== true) throw new OracleError(`deliverTimer: token ${token} is not active (cleared or fired) and captured is not true`);
  runTimer(known);
}

// ---- dependency fakes -------------------------------------------------------------------------------------------------------
/** One logical dependency call: count it, maybe throw (scripted), compute the value, record it. Void effects record no reply. */
function call(name, args, produce, hasReply) {
  const s = W.s;
  const n = count(s, name);
  if (s.throws[name]?.has(n)) {
    const error = new Error(`dep-throw ${name} ${n}`);
    if (!s.excluded.includes('dep-throw')) s.excluded.push('dep-throw');
    trace(name, args.length > 0 ? () => args.map(arg => enc(arg)) : undefined, () => ({ throw: encThrown(error) }));
    throw error;
  }
  const value = produce === undefined ? undefined : produce();
  trace(name, args.length > 0 ? () => args.map(arg => enc(arg)) : undefined, hasReply ? () => enc(value) : undefined);
  return value;
}

const takeReply = (s, name, fallback) => (s.queues[name].length > 0 ? s.queues[name].shift() : fallback);

/** The deps object of one instance. All instances share the one world; the trace `i` comes from the current call/timer context. */
export function makeDeps() {
  return {
    now: () => call('now', [], () => { const s = W.s; return s.nowOverride.length > 0 ? s.nowOverride.shift() : s.now; }, true),
    unit: () => call('unit', [], () => W.s.unit, true),
    findSession: id => call('findSession', [id], () => W.s.sessions.find(x => isObj(x) && x.id === id), true),
    save: session => call('save', [session]),
    changed: () => call('changed', []),
    stopMain: sessionId => call('stopMain', [sessionId]),
    stopDelegation: (sessionId, delegationId) => call('stopDelegation', [sessionId, delegationId]),
    runningDelegationIds: ref => call('runningDelegationIds', [ref], () => {
      const list = takeReply(W.s, 'runningDelegationIds', []);
      return Array.isArray(list) ? list.slice() : list; // a fresh array every call
    }, true),
    stopAttachedTurns: ref => call('stopAttachedTurns', [ref]),
    holdQueued: (session, budgetId) => call('holdQueued', [session, budgetId]),
    releaseHeld: (session, budgetId) => call('releaseHeld', [session, budgetId], () => takeReply(W.s, 'releaseHeld', false), true),
    carryHeld: (owner, oldBudgetId, newRef) => call('carryHeld', [owner, oldBudgetId, newRef]),
    queueContinue: (session, budgetId, reason, wake) => call('queueContinue', [session, budgetId, reason, wake]),
    dropHeld: (session, budgetId) => call('dropHeld', [session, budgetId]),
    cancelRecovery: session => call('cancelRecovery', [session]),
    isQuiet: session => call('isQuiet', [session], () => takeReply(W.s, 'isQuiet', false), true),
    announce: session => call('announce', [session]),
  };
}

// ---- script events other than call/advance/deliverTimer ---------------------------------------------------------------------
export function setTime(value) {
  const s = W.s;
  s.now = value;
  trace('setTime', () => enc({ now: value }));
}

export function setUnit(unit) {
  const s = W.s;
  s.unit = unit;
  trace('setUnit', () => enc(unit));
}

export function putSession(session) {
  const s = W.s;
  if (!isObj(session) || typeof session.id !== 'string') throw new OracleError('putSession needs an object session with a string id');
  registerSession(session);
  const index = s.sessions.findIndex(x => isObj(x) && x.id === session.id);
  if (index >= 0) s.sessions[index] = session; else s.sessions.push(session);
  trace('putSession', () => enc(session));
}

export function removeSession(id) {
  const s = W.s;
  s.sessions = s.sessions.filter(x => !(isObj(x) && x.id === id));
  trace('removeSession', () => enc({ id }));
}

export function setDepReply(dep, replies) {
  const s = W.s;
  if (!REPLY_DEPS.includes(dep)) throw new OracleError(`setDepReply: ${JSON.stringify(dep)} has no reply queue`);
  if (!Array.isArray(replies)) throw new OracleError('setDepReply needs a replies array');
  s.queues[dep] = [...replies];
  trace('setDepReply', () => enc({ dep, replies }));
}

export function snapshot(label) {
  const s = W.s;
  trace('snapshot', () => enc({ label }), () => enc(worldSnapshot(s)));
}

// ---- observation helpers ----------------------------------------------------------------------------------------------------
/** Private maps of one instance (TS private is not runtime private). */
export function clocksOf(instance) {
  return [...instance['clocks']].map(([id, clock]) => [id, {
    sessionId: clock.sessionId,
    runs: [...clock.runs].map(([key, run]) => [key, { waiting: run.waiting }]),
    ...(clock.since !== undefined ? { since: clock.since } : {}),
    ...(clock.timer !== undefined ? { timer: clock.timer.token } : {}),
  }]);
}

export function toolSeenOf(instance) {
  return [...instance['toolSeen']].map(([turnId, ids]) => [turnId, [...ids]]);
}

/** Active timers in firing order. */
export function activeTimers(s) {
  return [...s.timers.values()].sort((a, b) => (a.due < b.due ? -1 : a.due > b.due ? 1 : a.seq - b.seq)).map(t => ({ token: t.token, delay: t.delay, due: t.due, i: t.i }));
}

function worldSnapshot(s) {
  return {
    now: s.now,
    unit: s.unit,
    sessions: s.sessions,
    clocks: Object.fromEntries([...s.instances].map(([name, inst]) => [name, clocksOf(inst)])),
    toolSeen: Object.fromEntries([...s.instances].map(([name, inst]) => [name, toolSeenOf(inst)])),
    timers: activeTimers(s),
  };
}
