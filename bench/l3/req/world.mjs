// Shared run state and every fake host call of the L3-9 request-handler oracle (SCENARIO.md sections 3, 5, 7). One module instance
// serves reference.mjs and the fakes/*.ts modules. Nothing here decides a handler outcome: the pinned handler runs for real and calls
// these fakes, which record the call (one ordered trace) and serve the scenario's script.
import { AsyncLocalStorage } from 'node:async_hooks';
import { EngineImportError } from '../entry/reference/backend-engine-import-tail.ts';
import { WireError } from '../entry/fakes/wire.ts';
import { ABSENT, OracleError, currentCase, dec as decJV, enc, nameOf, put, refOf } from '../entry/reference-primitives.mjs';

export const als = new AsyncLocalStorage();
/** An oracle-side refusal: recorded as a driver violation (the scenario is wrong, not the handler) and returned for throwing. */
function fail(message) { const e = new OracleError(message); W.s?.violations.push(message); return e; }
export const W = { s: null }; // the running scenario state (null between scenarios)
export const realSetImmediate = globalThis.setImmediate;
const NativeSet = globalThis.Set;
const RealDate = globalThis.Date;
const NativeAddListener = EventTarget.prototype.addEventListener;
const NativeRemoveListener = EventTarget.prototype.removeEventListener;

// ---- script decoding (SCENARIO.md 2.1) --------------------------------------------------------------------------------
const MARKERS = ['u16', 'undef', 'num', 'ref', 'arr', 'obj', 'absent'];
/** Lenient JV decoder: bare arrays and plain objects are accepted (records keep listed key order); a single-key marker object is a JV. */
export function d(jv) {
  if (Array.isArray(jv)) return jv.map(item => { const value = d(item); if (value === ABSENT) throw fail('absent inside an array'); return value; });
  if (jv !== null && typeof jv === 'object') {
    const keys = Object.keys(jv);
    if (keys.length === 1 && MARKERS.includes(keys[0])) return decJV(jv);
    const out = {};
    for (const key of keys) { const value = d(jv[key]); if (value !== ABSENT) put(out, key, value); }
    return out;
  }
  return decJV(jv);
}
const isDict = v => v !== null && typeof v === 'object' && !Array.isArray(v);

// ---- thrown values ---------------------------------------------------------------------------------------------------
/** The scripted thrown value of an ERR spec (built once per ref name by entry begin()); the throw is remembered for primitives. */
export function errOf(spec) {
  if (!isDict(spec) || typeof spec.ref !== 'string') throw fail(`bad ERR ${JSON.stringify(spec)}`);
  currentCase().thrownLog.push(spec.ref);
  return refOf(spec.ref);
}
const isDomAbort = e => typeof DOMException === 'function' && e instanceof DOMException && e.name === 'AbortError';

/** A thrown or rejected value as JV (SCENARIO.md 4.2). */
export function encThrown(e) {
  const s = W.s;
  if (s) for (const [handle, h] of s.handles) {
    if (h.controller.signal.aborted && Object.is(h.controller.signal.reason, e)) {
      const name = nameOf(e);
      return name === undefined ? { cancelReason: handle } : { cancelReason: handle, ref: name };
    }
  }
  const name = nameOf(e);
  if (name !== undefined) return { ref: name };
  if (e instanceof WireError) return { wire: { key: enc(e.key), message: enc(e.message) } };
  if (e instanceof EngineImportError) return { eie: { code: enc(e.code), message: enc(e.message), hasCause: 'cause' in e, ...('cause' in e ? { cause: encThrown(e.cause) } : {}) } };
  if (isDomAbort(e)) return { domAbort: 1 };
  if (e instanceof Error) return { native: { name: enc(e.name), message: enc(e.message) } };
  return enc(e);
}

// ---- trace ------------------------------------------------------------------------------------------------------------
/** Append a trace record: {c, r, i, a?, o?}. r and i come from the async context of the request that made the call (null for the scheduler). */
export function rec(c, a, o) {
  const store = als.getStore();
  const entry = { c, r: store?.r ?? null, i: store?.i ?? null };
  if (a !== undefined) entry.a = a;
  if (o !== undefined) entry.o = o;
  W.s.trace.push(entry);
  return entry;
}
const okOf = value => ({ ok: enc(value) });
const throwOf = e => ({ throw: encThrown(e) });

/** The n-th call takes entry n; the last entry repeats (null entries are legal). `queue` may be one entry or an array. */
function take(key, queue, fallback) {
  const list = Array.isArray(queue) ? queue : queue === undefined ? [] : [queue];
  if (list.length === 0) return fallback;
  const used = W.s.used.get(key) ?? 0;
  W.s.used.set(key, used + 1);
  return list[Math.min(used, list.length - 1)];
}
function need(key, queue, what) {
  const entry = take(key, queue, undefined);
  if (entry === undefined || entry === null) throw fail(`unscripted call: ${what}`);
  return entry;
}
function resetUsed(prefix) { for (const key of [...W.s.used.keys()]) if (key === prefix || key.startsWith(prefix + ':')) W.s.used.delete(key); }

// ---- signals and cancel handles ---------------------------------------------------------------------------------------
/** The name of an abort signal: the handle name for a scenario cancel handle, otherwise sigN at first sight (and its abort is traced). */
export function signalName(signal) {
  const s = W.s;
  if (signal === undefined) return null;
  if (!(signal instanceof AbortSignal)) return { notSignal: typeof signal };
  let name = s.sigNames.get(signal);
  if (name === undefined) {
    name = `sig${++s.sigSeq}`;
    s.sigNames.set(signal, name);
    NativeAddListener.call(signal, 'abort', () => rec('abort', { signal: name, reason: encThrown(signal.reason) }));
  }
  return name;
}

/** A scenario cancel handle (named AbortController). Listener add/remove on its signal are recorded; once listeners leave the active set when it aborts. */
export function getHandle(name) {
  const s = W.s;
  let h = s.handles.get(name);
  if (h) return h;
  const controller = new AbortController();
  const signal = controller.signal;
  h = { name, controller, active: new Map(), onceIds: new Set(), ids: new WeakMap() };
  s.handles.set(name, h);
  s.sigNames.set(signal, name);
  NativeAddListener.call(signal, 'abort', () => {
    rec('abort', { signal: name, reason: encThrown(signal.reason) });
    for (const id of h.onceIds) h.active.delete(id);
    h.onceIds.clear();
  });
  Object.defineProperty(signal, 'addEventListener', { configurable: true, writable: true, value(type, listener, options) {
    const once = isDict(options) && options.once === true;
    let id = h.ids.get(listener);
    const fresh = id === undefined;
    if (fresh) { id = ++s.listenerSeq; h.ids.set(listener, id); }
    rec('listener.add', { handle: name, type, once, id });
    if (fresh) { h.active.set(id, type); if (once) h.onceIds.add(id); }
    return NativeAddListener.call(signal, type, listener, options);
  } });
  Object.defineProperty(signal, 'removeEventListener', { configurable: true, writable: true, value(type, listener, options) {
    const id = h.ids.get(listener);
    rec('listener.remove', { handle: name, type, id: id ?? null, active: id !== undefined && h.active.has(id) });
    if (id !== undefined) { h.active.delete(id); h.onceIds.delete(id); h.ids.delete(listener); }
    return NativeRemoveListener.call(signal, type, listener, options);
  } });
  return h;
}

export function abortHandle(name, reasonSpec) {
  const h = getHandle(name);
  if (reasonSpec === undefined) h.controller.abort();
  else h.controller.abort(errOf(reasonSpec));
}

// ---- deferred calls ---------------------------------------------------------------------------------------------------
/**
 * A call whose entry has `defer: label` stays pending until {ev:"settle", label}. The n-th use of one label is named label, label#2, ...
 * `onAbort: ERR` rejects the pending call when `signal` aborts (a real reader would). The promise resolves with the outcome entry.
 */
function gate(call, entry, signal) {
  const s = W.s;
  const count = (s.labelCount.get(entry.defer) ?? 0) + 1;
  s.labelCount.set(entry.defer, count);
  const label = count === 1 ? entry.defer : `${entry.defer}#${count}`;
  return new Promise(resolve => {
    s.pending.set(label, { call, entry, resolve });
    if (entry.onAbort !== undefined && signal !== undefined) {
      const fire = () => { if (s.pending.has(label)) settle(label, { reject: entry.onAbort }, 'abort'); };
      if (signal.aborted) queueMicrotask(fire); else NativeAddListener.call(signal, 'abort', fire, { once: true });
    }
  }).then(outcome => outcome);
}

export function settle(label, override, via) {
  const s = W.s;
  const p = s.pending.get(label);
  if (!p) throw fail(`settle of unknown or settled label ${label}`);
  s.pending.delete(label);
  const outcome = override === undefined ? p.entry : { ...p.entry, ok: undefined, reject: undefined, ...override };
  rec('settle', { label, call: p.call, ...(via ? { via } : {}) }, outcome.reject !== undefined ? { reject: true } : { ok: true });
  p.resolve(outcome);
}

/** Serve an async dependency entry: sync_throw throws now; otherwise a Promise (deferred, resolving to ok or rejecting). */
function asyncServe(call, entry, signal, map = v => v) {
  if (entry.sync_throw !== undefined) throw errOf(entry.sync_throw);
  const finish = outcome => { if (outcome.reject !== undefined) throw errOf(outcome.reject); return map(outcome.ok === undefined ? undefined : d(outcome.ok)); };
  if (entry.defer === undefined) return Promise.resolve().then(() => finish(entry));
  return gate(call, entry, signal).then(finish);
}

function traced(c, a, produce) {
  let result;
  try { result = produce(); } catch (e) { rec(c, a, throwOf(e)); throw e; }
  if (result instanceof Promise) {
    const entry = rec(c, a, { pending: true });
    return result.then(v => { entry.o = okOf(v); return v; }, e => { entry.o = throwOf(e); throw e; });
  }
  rec(c, a, okOf(result));
  return result;
}

// ---- engine readers ---------------------------------------------------------------------------------------------------
function optionsRecord(options) {
  const s = W.s;
  const out = { keys: Object.keys(options), signal: signalName(options.signal), aborted: options.signal?.aborted === true,
    env: options.env === s.deps.env ? 'deps.env' : enc(options.env) };
  if ('codex' in options) out.codex = enc(options.codex);
  if ('opencode' in options) out.opencode = enc(options.opencode);
  return out;
}

/** fakes/engine-*.ts: the five readers. The entry's real listExternal/readExternal call these; the queue entry is {ok|reject|defer|onAbort}. */
export function readerCall(engine, op, source, folder, thread, isRead, options) {
  const s = W.s;
  const a = { engine, op, source: enc(source.source), folder: enc(folder), ...(isRead ? { thread: enc(thread) } : {}), options: optionsRecord(options) };
  const entry = need(`reader:${engine}.${op}`, s.world.readers?.[engine]?.[op], `reader ${engine}.${op}`);
  return traced('reader', a, () => asyncServe(`reader:${engine}.${op}`, entry, options.signal));
}

// ---- host calls of the handler ----------------------------------------------------------------------------------------
export function locateAcp(name) {
  const entry = take('locateAcp', W.s.world.locateAcp, { ok: { binary: '/bin/opencode' } });
  return traced('locateAcp', [enc(name)], () => asyncServe('locateAcp', entry, undefined));
}
export function makeCodexRunDeps(source, dataDir) {
  const entry = take('makeCodexRunDeps', W.s.world.makeCodexRunDeps, { ok: { ref: 'codexRun' } });
  return traced('makeCodexRunDeps', { source: enc(source.source), dataDir: enc(dataDir) }, () => asyncServe('makeCodexRunDeps', entry, undefined));
}
function tempRoot() {
  const entry = take('tempRoot', W.s.world.tempRoot, { ok: '/tmp/lair-root' });
  return traced('tempRoot', [], () => asyncServe('tempRoot', entry, undefined));
}
function grokHomeFor(account) {
  const entry = take('grokHomeFor', W.s.world.grokHomeFor, { ok: { home: '/grok/home', binary: '/grok/bin', env: { ref: 'grokEnv' } } });
  return traced('grokHomeFor', [enc(account)], () => asyncServe('grokHomeFor', entry, undefined));
}
export function grokLoadCheck(check, home, folder, thread, signal) {
  const entry = take('loadCheck', W.s.world.loadCheck, { ok: undefined });
  const a = { checkKeys: Object.keys(check), binary: enc(check.binary), env: enc(check.env), home: enc(home), folder: enc(folder), thread: enc(thread), signal: signalName(signal), aborted: signal?.aborted === true };
  return traced('loadCheck', a, () => asyncServe('loadCheck', entry, signal));
}
export function rm(path, options) {
  const entry = take('rm', W.s.world.rm, { ok: undefined });
  if (entry.sync_throw !== undefined) throw fail('rm has no sync_throw (node:fs/promises rm is async)');
  return traced('rm', { path: enc(path), options: enc(options) }, () => asyncServe('rm', entry, undefined));
}

/** copyGrokSession fake. Stage 1 (file copy) may be deferred or rejected; then verify (the handler's callback) runs when entry.verify is set,
 * with the real copy's translation (non-Abort failure -> EngineImportError('grok-load-failed', undefined, {cause}); the name test is unguarded like
 * the real code); then {path, skipped} (entry.ok). */
export function copyGrokSession(source, folder, thread, targetHome, verify, signal) {
  const entry = need('copy', W.s.world.copy, 'copyGrokSession');
  const a = { source: enc(source.source), folder: enc(folder), thread: enc(thread), targetHome: enc(targetHome), verify: typeof verify, signal: signalName(signal), aborted: signal?.aborted === true };
  if (entry.sync_throw !== undefined) { const e = errOf(entry.sync_throw); rec('copy', a, throwOf(e)); throw e; }
  const run = async () => {
    const outcome = entry.defer === undefined ? entry : await gate('copy', entry, signal);
    if (outcome.reject !== undefined) throw errOf(outcome.reject);
    if (entry.verify === true) {
      rec('copy.verify', { home: enc(targetHome) });
      try { await verify(targetHome); } catch (error) {
        if (error.name === 'AbortError') throw error;
        throw new EngineImportError('grok-load-failed', undefined, { cause: error });
      }
    }
    return outcome.ok === undefined ? { path: '/grok/home/sessions/copy', skipped: 0 } : d(outcome.ok);
  };
  const entryRec = rec('copy', a, { pending: true });
  return run().then(v => { entryRec.o = okOf(v); return v; }, e => { entryRec.o = throwOf(e); throw e; });
}

// ---- uuid and clock ----------------------------------------------------------------------------------------------------
export function randomUUID() {
  const s = W.s;
  const list = s.world.uuids ?? [];
  const n = s.uuidUsed++;
  const entry = n < list.length ? list[n] : `00000000-0000-4000-8000-${String(n - list.length + 1).padStart(12, '0')}`;
  if (isDict(entry) && 'throw' in entry) { const e = errOf(entry.throw); rec('uuid', [], throwOf(e)); throw e; }
  const value = d(entry);
  rec('uuid', [], okOf(value));
  return value;
}

export const nowMs = () => W.s.clockStart + W.s.now;
class FakeDate extends RealDate {
  constructor(...args) {
    if (args.length === 0) {
      const s = W.s;
      const clock = s.world.clock;
      if (isDict(clock) && 'throw' in clock) { const e = errOf(clock.throw); rec('now', [], throwOf(e)); throw e; }
      super(nowMs());
      rec('now', [], { ok: this.toISOString() });
    } else super(...args);
  }
  static now() { return nowMs(); }
}

// ---- timers (logical) --------------------------------------------------------------------------------------------------
function fakeSetTimeout(cb, delay, ...args) {
  const s = W.s;
  const id = ++s.timerSeq;
  const ms = Number(delay) >= 1 ? Math.trunc(Number(delay)) : 1;
  const store = als.getStore();
  s.timers.set(id, { id, due: s.now + ms, delay: ms, cb, args, snap: AsyncLocalStorage.snapshot(), r: store?.r ?? null, i: store?.i ?? null });
  rec('timer.set', { id, delay: ms, due: s.now + ms });
  return id;
}
function fakeClearTimeout(id) {
  const s = W.s;
  const had = s.timers.delete(id);
  rec('timer.clear', { id: typeof id === 'number' ? id : enc(id), active: had });
}
export function installGlobals() {
  globalThis.setTimeout = fakeSetTimeout;
  globalThis.clearTimeout = fakeClearTimeout;
  globalThis.Date = FakeDate;
}
export function restoreGlobals(real) {
  globalThis.setTimeout = real.setTimeout;
  globalThis.clearTimeout = real.clearTimeout;
  globalThis.Date = RealDate;
  globalThis.Set = NativeSet;
}
export const realGlobals = { setTimeout: globalThis.setTimeout, clearTimeout: globalThis.clearTimeout };

export async function quiesce() { await new Promise(r => realSetImmediate(r)); await new Promise(r => realSetImmediate(r)); }

/** Advance the logical clock; timers fire in (due, id) order with the microtasks run to quiescence after each. */
export async function advance(ms) {
  const s = W.s;
  const target = s.now + ms;
  for (;;) {
    let next;
    for (const t of s.timers.values()) if (t.due <= target && (!next || t.due < next.due || (t.due === next.due && t.id < next.id))) next = t;
    if (!next) break;
    s.timers.delete(next.id);
    s.now = Math.max(s.now, next.due);
    next.snap(() => { rec('timer.fire', { id: next.id, at: s.now }); next.cb(...next.args); });
    await quiesce();
  }
  s.now = target;
}

// ---- opening-set instrument (SCENARIO.md 6.3) --------------------------------------------------------------------------
/** A native Set that records add/delete/has; installed as globalThis.Set only while one instance's engineImportHandlers() runs. */
export function recordingSetFor(instance) {
  return class RecordingSet extends NativeSet {
    constructor() { super(); W.s.openingSets.set(instance, this); }
    has(key) { const result = super.has(key); event('has', key, result); return result; }
    add(key) { const had = super.has(key); const out = super.add(key); event('add', key, !had); return out; }
    delete(key) { const result = super.delete(key); event('delete', key, result); return result; }
  };
}
function event(op, key, result) {
  const store = als.getStore();
  const s = W.s;
  s.openingEvents.push({ i: store?.i ?? null, r: store?.r ?? null, op, key: enc(key), result });
  rec(`opening.${op}`, { key: enc(key) }, { ok: result });
}
export function createHandlers(instance, engineImportHandlers) {
  const s = W.s;
  globalThis.Set = recordingSetFor(instance);
  try { return engineImportHandlers(s.deps); } finally { globalThis.Set = NativeSet; }
}

// ---- the dependency world (EngineImportDeps) ---------------------------------------------------------------------------
function valueOrThrow(c, a, entry) {
  if (isDict(entry) && 'throw' in entry) { const e = errOf(entry.throw); rec(c, a, throwOf(e)); throw e; }
  const value = d(entry);
  rec(c, a, okOf(value));
  return value;
}

export function makeDeps() {
  const s = W.s;
  const deps = {
    dataDir: d(s.world.dataDir ?? '/data'),
    env: d(s.world.env ?? {}),
    accounts: null, // set by reference.mjs to the entry accountsCallback (L3-8 script)
    grokAccounts() { return valueOrThrow('grokAccounts', [], s.world.grokAccounts ?? []); },
    sessionList() {
      const call = ++s.sessionListCalls;
      const script = take('sessionList', s.world.sessionList, null);
      if (script !== null && script.throw !== undefined && script.throwAt === undefined) { const e = errOf(script.throw); rec('sessionList', { call }, throwOf(e)); throw e; }
      rec('sessionList', { call, size: s.sessions.length });
      const snapshot = [...s.sessions];
      return { [Symbol.iterator]() {
        let index = 0;
        return {
          next() {
            if (script !== null && script.throwAt === index) { const e = errOf(script.throw); rec('sessionList.next', { call, index }, throwOf(e)); throw e; }
            if (index >= snapshot.length) { rec('sessionList.done', { call, consumed: index }); return { done: true, value: undefined }; }
            const session = snapshot[index];
            rec('sessionList.next', { call, index, id: enc(session.id) });
            index += 1;
            return { done: false, value: session };
          },
          return(value) { rec('sessionList.return', { call, consumed: index }); return { done: true, value }; },
        };
      } };
    },
    isTemporary(id) {
      const table = s.world.isTemporary ?? {};
      const raw = typeof id === 'string' && Object.hasOwn(table, id) ? table[id] : false;
      return valueOrThrow('isTemporary', [enc(id)], raw);
    },
    checkFolder(path) {
      const table = s.world.checkFolder ?? {};
      const queue = Object.hasOwn(table, path) ? table[path] : Object.hasOwn(table, '*') ? table['*'] : undefined;
      const entry = take(`checkFolder:${path}`, queue, { ok: path });
      if (entry.throw !== undefined) { const e = errOf(entry.throw); rec('checkFolder', [enc(path)], throwOf(e)); throw e; }
      const value = d(entry.ok);
      rec('checkFolder', [enc(path)], okOf(value));
      return value;
    },
    modelFor(engine, account, wanted) {
      const entry = take('modelFor', s.world.modelFor, undefined);
      const a = { engine: enc(engine), account: enc(account), wanted: enc(wanted) };
      if (entry !== undefined && entry !== null && entry.throw !== undefined) { const e = errOf(entry.throw); rec('modelFor', a, throwOf(e)); throw e; }
      const value = entry !== undefined && entry !== null && 'ok' in entry ? d(entry.ok) : (wanted || 'default-model');
      rec('modelFor', a, okOf(value));
      return value;
    },
    createImported(seed) {
      const entry = take('createImported', s.world.createImported, { mode: 'ok' });
      const a = { seedKeys: Object.keys(seed), engine: enc(seed.engine), ...('account' in seed ? { account: enc(seed.account) } : {}), model: enc(seed.model), folder: enc(seed.folder) };
      const mode = entry.mode ?? 'ok';
      if (mode === 'fail-before-fill') { const e = errOf(entry.throw); rec('createImported', a, throwOf(e)); throw e; }
      s.createdSeq += 1;
      const id = entry.id !== undefined ? d(entry.id) : `new-${s.createdSeq}`;
      const session = d(entry.session ?? s.world.newSession ?? { title: '', turns: [] });
      put(session, 'id', id);
      const created = { id: enc(id), seedKeys: a.seedKeys, seed: { engine: a.engine, ...('account' in a ? { account: a.account } : {}), model: a.model, folder: a.folder }, before: enc(session) };
      seed.fill(session);
      created.after = enc(session);
      s.createdSessions.push(created);
      if (mode === 'fail-after-fill') { created.outcome = 'fail-after-fill'; const e = errOf(entry.throw); rec('createImported', a, throwOf(e)); throw e; }
      created.outcome = 'ok';
      s.sessions.push(session);
      rec('createImported', a, okOf(id));
      return id;
    },
    fromAppAction() {
      const store = als.getStore();
      const raw = store && store.fromAppAction !== undefined ? store.fromAppAction : (s.world.fromAppAction ?? false);
      return valueOrThrow('fromAppAction', [], raw);
    },
    tempRoot,
    grokHomeFor,
  };
  return deps;
}

export const defaultsFor = { clockStart: 1_700_000_000_000 };
export { ABSENT, OracleError, WireError, EngineImportError, enc, nameOf };
