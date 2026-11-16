// FS / SQLite / homedir / spawn / clock shim for the pinned OpenCode TypeScript (DESIGN section 6).
// reference-hooks.mjs redirects the node:fs/promises, node:os, node:sqlite imports of opencode.ts, the node:fs/promises and node:fs
// imports of tail.ts, and the node:child_process import of stubs.ts to the exports below. Every function delegates to the real
// Node function; around the call it adds the schedule rules (same matcher and barriers as host.rs) and writes JSONL trace records
// to fd 2 in the SAME language as the Rust host (DESIGN section 5):
//   {"trace":{v:1, seq, event, id, op, stage, handle, args, result, error}}
//   event: issue | complete | barrier-before-call | barrier-after-result-before-delivery | schedule
//   ops:   realpath lstat db_open_readonly db_query db_close mkdtemp open spawn observe kill close stat read rm clock_now
// Paths in the trace are the real fixture paths (common.normalize_trace maps roots to tokens); stage numbers are DESIGN section 5's
// stable order. This module's own builtin imports are not redirected.
//
// What is shimmed but never replaced:  sessionRows / convert / opencodePlan / waitExport / runExport stay the pinned code.
// Real SQLite (node:sqlite): the single real execution reads INTEGER as bigint (setReadBigInts) so the storage class is observable;
// the default safe-range check/conversion is then reproduced before the rows reach the pinned code (no second query, no typeof()).
// Real child: node:child_process.spawn of the pinned spawnEngine stub with the real output-file fd; its events/timers are observed
// without changing handler order. Timer registration of the active waitExport call is recorded as clock_now + observe(deadline).
import * as realFs from 'node:fs';
import * as realFsp from 'node:fs/promises';
import * as realCp from 'node:child_process';
import * as realSqlite from 'node:sqlite';
import { constants as osConstants } from 'node:os';
import { createHash } from 'node:crypto';
import { join as joinPath } from 'node:path';

export const constants = realFs.constants;

export class HarnessError extends Error {
  constructor(message) { super(message); this.name = 'HarnessError'; }
}

const EVENT = Symbol.for('opencode.reference.event');
const saved = {
  setTimeout: globalThis.setTimeout, clearTimeout: globalThis.clearTimeout, dateNow: Date.now, kill: process.kill.bind(process),
};
const SCHEMA_SQL = 'SELECT name FROM pragma_table_info(?)';
const COUNT_SQL = 'SELECT COUNT(*) AS n FROM message WHERE session_id = ?';
const SAFE = 9007199254740991n;

const STAGE = {
  input: 0, dbParent: 1, dbLstat: 2, dbRealpath: 3, dbOpen: 4, schemaSession: 5, schemaMessage: 6, select: 7, countPrepare: 8, count: 9,
  dbClose: 10, readAdmission: 11, listClock: 12, tempCreate: 13, outputOpen: 14, spawn: 15, wait: 16, term: 17, kill: 18, giveUp: 19,
  outputClose: 20, outputStat: 21, outputRead: 22, cleanup: 23, parse: 24, shape: 25, convert: 26, plan: 27, result: 28,
};
export const STAGE_NAMES = ['input', 'db-parent', 'db-lstat', 'db-realpath', 'db-open', 'schema-session', 'schema-message', 'select', 'count-prepare',
  'count', 'db-close', 'read-admission', 'list-clock', 'temp-create', 'output-open', 'spawn', 'wait', 'term', 'kill', 'give-up', 'output-close',
  'output-stat', 'output-read', 'cleanup', 'parse', 'shape', 'convert', 'plan', 'result'];

const SQLITE_NAMES = {
  1: 'SQLITE_ERROR', 2: 'SQLITE_INTERNAL', 3: 'SQLITE_PERM', 4: 'SQLITE_ABORT', 5: 'SQLITE_BUSY', 6: 'SQLITE_LOCKED', 7: 'SQLITE_NOMEM',
  8: 'SQLITE_READONLY', 9: 'SQLITE_INTERRUPT', 10: 'SQLITE_IOERR', 11: 'SQLITE_CORRUPT', 12: 'SQLITE_NOTFOUND', 13: 'SQLITE_FULL',
  14: 'SQLITE_CANTOPEN', 15: 'SQLITE_PROTOCOL', 16: 'SQLITE_EMPTY', 17: 'SQLITE_SCHEMA', 18: 'SQLITE_TOOBIG', 19: 'SQLITE_CONSTRAINT',
  20: 'SQLITE_MISMATCH', 21: 'SQLITE_MISUSE', 22: 'SQLITE_NOLFS', 23: 'SQLITE_AUTH', 24: 'SQLITE_FORMAT', 25: 'SQLITE_RANGE', 26: 'SQLITE_NOTADB',
};

let S = fresh({});

function fresh(cfg) {
  const rules = (cfg.schedule?.rules ?? []).map(r => {
    const m = { occurrence: 1, barrier: 'before-call', ...(r.match ?? {}) };
    if (m.op === 'spawn_stdout_fd') m.op = 'spawn';
    if (r.hold !== undefined || r.return !== undefined) throw new HarnessError('schedule rule uses hold/return: not supported by the real-run host or shim');
    if (['observe', 'clock_now'].includes(m.op)) throw new HarnessError(`schedule rule on ${m.op} is not supported (child/clock rules need the scripted facade)`);
    return { match: m, after: m.barrier === 'after-result-before-delivery', actions: r.actions ?? [], error: r.error };
  });
  return {
    trace: cfg.trace !== false, artifactDir: cfg.artifactDir ?? null, homedir: cfg.homedir ?? '', nowMs: cfg.nowMs ?? null, rpcMs: cfg.rpcMs ?? 20000,
    mode: cfg.mode ?? '', rules, seq: 0, nextId: 1, nextHandle: 1, occ: new Map(), events: [], tempDirs: [], t0: performance.now(),
    nRealpath: 0, schemaPrepares: 0, files: new Map(), fds: new Map(), conns: new Map(), child: null, children: [], active: false, openStatements: 0,
  };
}

// ---- case control (used by reference.mjs) --------------------------------------------------------------------------------
export function begin(cfg) {
  S = fresh(cfg);
  S.active = true;
  globalThis.setTimeout = wrappedSetTimeout;
  globalThis.clearTimeout = wrappedClearTimeout;
  Date.now = wrappedNow;
  process.kill = wrappedKill;
}

/** Teardown after the importer returned: release handles the importer did not close, end a still-living child, restore globals. */
export async function finish() {
  S.active = false;
  globalThis.setTimeout = saved.setTimeout;
  globalThis.clearTimeout = saved.clearTimeout;
  Date.now = saved.dateNow;
  process.kill = saved.kill;
  const closes = [];
  for (const f of S.files.values()) if (!f.closed) closes.push(f.fh.close().catch(() => undefined));
  for (const c of S.conns.values()) { try { c.real.close(); } catch { /* already closed */ } }
  for (const c of S.children) {
    if (c.cp.exitCode === null && c.cp.signalCode === null && c.cp.pid) {
      process.stderr.write(JSON.stringify({ teardown: { killedChildren: 1 } }) + '\n');
      try { saved.kill(-c.cp.pid, 'SIGKILL'); } catch { /* gone */ }
      closes.push(new Promise(resolve => { c.cp.once('close', resolve); saved.setTimeout(resolve, 3000); }));
    }
  }
  await Promise.all(closes);
  await new Promise(resolve => setImmediate(resolve));
}

export function eventIndexOf(err) {
  for (let e = err, n = 0; e && typeof e === 'object' && n < 8; e = e.cause, n++) if (typeof e[EVENT] === 'number') return e[EVENT];
  return null;
}
export function eventAt(i) { return S.events[i]; }
export function lastEvent(skip = () => false) {
  for (let i = S.events.length - 1; i >= 0; i--) if (!skip(S.events[i])) return S.events[i];
  return undefined;
}
export function tagError(err, stage) {
  if (err && typeof err === 'object' && err[EVENT] === undefined) { S.events.push({ op: 'observe', stage, key: null }); try { err[EVENT] = S.events.length - 1; } catch { /* frozen */ } }
}
export async function sqliteVersion() {
  const db = new realSqlite.DatabaseSync(':memory:');
  try { return db.prepare('SELECT sqlite_version() AS v').get().v; } finally { db.close(); }
}

// ---- trace ---------------------------------------------------------------------------------------------------------------
const sha = data => createHash('sha256').update(data).digest('hex');

function writeErr(text) {
  const buf = Buffer.from(text);
  let off = 0;
  while (off < buf.length) {
    try { off += realFs.writeSync(2, buf, off); } catch (e) { if (e.code !== 'EAGAIN') throw e; }
  }
}

function artifact(data) {
  const h = sha(data);
  if (S.artifactDir) {
    const p = joinPath(S.artifactDir, h);
    if (!realFs.existsSync(p)) { try { realFs.writeFileSync(p, data); } catch { /* the trace keeps length+sha anyway */ } }
  }
  return { length: data.length, sha256: h };
}

function emit(event, id, op, stage, handle, args, result, error) {
  if (!S.trace) return;
  S.seq += 1;
  writeErr(JSON.stringify({ trace: { v: 1, seq: S.seq, event, id, op, stage, handle: handle >= 1 ? handle : null, args: args ?? null, result: result ?? null, error: error ?? null } }) + '\n');
}

function typed(v) {
  if (v === null || v === undefined) return { kind: 'null' };
  if (typeof v === 'bigint') return { kind: 'integer', decimal: v.toString() };
  if (typeof v === 'number') { const b = Buffer.alloc(8); b.writeDoubleBE(v); return { kind: 'real', bits: b.toString('hex') }; }
  if (typeof v === 'string') {
    const n = Buffer.byteLength(v);
    return n > 65536 ? { kind: 'text', ...artifact(Buffer.from(v, 'utf8')) } : { kind: 'text', text: v };
  }
  if (v instanceof Uint8Array) return v.length > 4096 ? { kind: 'blob', ...artifact(Buffer.from(v)) } : { kind: 'blob', hex: Buffer.from(v).toString('hex') };
  throw new HarnessError(`cannot trace a ${typeof v} SQL value`);
}

// ---- errors --------------------------------------------------------------------------------------------------------------
export function osError(e) {
  return e && typeof e === 'object' && typeof e.code === 'string' && /^E[A-Z0-9]+$/.test(e.code) && ('errno' in e || 'syscall' in e);
}

function errInfo(e) {
  if (e?.injected) return e.injected;
  if (e?.code === 'ERR_OUT_OF_RANGE') return { class: 3, code: 'ERR_OUT_OF_RANGE', native: 0 };
  // a path/argv/env string with a NUL byte (Node validates before any syscall); the host reports the same failed completion
  if (e?.code === 'ERR_INVALID_ARG_VALUE') return { class: 3, code: 'ERR_INVALID_ARG_VALUE', native: 0 };
  if (typeof e?.errcode === 'number' || e?.code === 'ERR_SQLITE_ERROR') {
    const ext = typeof e.errcode === 'number' && e.errcode > 0 ? e.errcode : 1;
    return { class: 2, code: SQLITE_NAMES[ext & 0xff] ?? 'SQLITE_ERROR', native: ext };
  }
  if (osError(e)) return { class: 1, code: e.code, native: typeof e.errno === 'number' && e.errno !== 0 ? Math.abs(e.errno) : (osConstants.errno[e.code] ?? 0) };
  return null;
}

/** Raw cause facts of an error chain (for the envelope): the first OS / SQLite / range error in e, e.cause, ... */
export function causeFacts(e) {
  for (let x = e, n = 0; x && typeof x === 'object' && n < 8; x = x.cause, n++) {
    const info = errInfo(x);
    if (info) return info;
  }
  return null;
}

function injectedError(spec, op, key) {
  const cls = spec.class ?? '';
  const code = spec.code;
  if (typeof code !== 'string') throw new HarnessError('error replacement needs code');
  let native;
  let e;
  if (cls === 'OS') {
    const num = osConstants.errno[code];
    if (num === undefined) throw new HarnessError(`unknown errno name ${code}`);
    native = spec.native ?? num;
    e = Object.assign(new Error(`${code}: injected, ${op}${key ? ` '${key}'` : ''}`), { errno: -Math.abs(native), code, syscall: op });
    e.injected = { class: 1, code, native };
  } else if (cls === 'SQLite') {
    native = spec.native ?? Number(Object.entries(SQLITE_NAMES).find(([, n]) => n === code)?.[0] ?? 1);
    e = Object.assign(new Error(`injected ${code}`), { code: 'ERR_SQLITE_ERROR', errcode: native, errstr: 'injected' });
    e.injected = { class: 2, code, native };
  } else if (cls === 'Range' || cls === 'binding-range') {
    e = Object.assign(new RangeError('injected range error'), { code });
    e.injected = { class: 3, code, native: spec.native ?? 0 };
  } else throw new HarnessError(`error class ${JSON.stringify(cls)} is not supported by the shim`);
  return e;
}

// ---- schedule: rule matching (same matcher as host.rs), actions, barriers --------------------------------------------------
function resolveRulePath(pat) {
  let out = '';
  for (let i = 0; i < pat.length;) {
    const m = /^@(\d+)/.exec(pat.slice(i));
    if (pat[i] === '@' && m) {
      const dir = S.tempDirs[Number(m[1]) - 1];
      if (dir === undefined) return null;
      const base = dir.slice(dir.lastIndexOf('/') + 1);
      if (base.length < 6) return null;
      out += base.slice(-6);
      i += m[0].length;
    } else { out += pat[i]; i += 1; }
  }
  return out;
}

function ruleHits(r, ctx) {
  const m = r.match;
  if (m.op !== ctx.op || m.occurrence !== ctx.occ) return false;
  if (m.phase !== undefined && m.phase !== (ctx.phase ?? undefined)) return false;
  if (m.sql !== undefined && (ctx.op !== 'db_query' || ctx.key !== m.sql)) return false;
  if (m.path !== undefined) {
    const x = resolveRulePath(m.path);
    if (x === null || ctx.key !== x) return false;
  }
  return true;
}

function bytesOf(r) {
  if (r.utf8 !== undefined) return Buffer.from(r.utf8, 'utf8');
  if (r.hex !== undefined) return Buffer.from(r.hex, 'hex');
  throw new HarnessError('schedule bytes spec needs utf8 or hex (the runner expands recipes to hex)');
}

function typedToJs(v) {
  switch (v.kind) {
    case 'null': return null;
    case 'integer': return BigInt(v.decimal);
    case 'real': return Buffer.from(v.bits, 'hex').readDoubleBE(0);
    case 'text': return v.text;
    case 'blob': return Buffer.from(v.hex, 'hex');
    default: throw new HarnessError('unknown typed value kind ' + v.kind);
  }
}

function wipe(p) {
  let st;
  try { st = realFs.lstatSync(p); } catch (e) { if (e.code === 'ENOENT') return; throw e; }
  if (st.isDirectory()) realFs.rmSync(p, { recursive: true, force: true }); else realFs.unlinkSync(p);
}

function runAction(a) {
  const p = a.path;
  switch (a.kind) {
    case 'replace-file': wipe(p); realFs.writeFileSync(p, bytesOf(a.bytes), { mode: a.mode ?? 0o644, flag: 'wx' }); if (a.mode !== undefined) realFs.chmodSync(p, a.mode); break;
    case 'symlink': wipe(p); realFs.symlinkSync(a.target, p); break;
    case 'hardlink': wipe(p); realFs.linkSync(a.of, p); break;
    case 'mkdir': realFs.mkdirSync(p, { mode: a.mode ?? 0o777 }); if (a.mode !== undefined) realFs.chmodSync(p, a.mode); break;
    case 'unlink': wipe(p); break;
    case 'append': realFs.appendFileSync(p, bytesOf(a.bytes)); break;
    case 'truncate': realFs.truncateSync(p, a.size ?? 0); break;
    case 'db-exec': {
      const db = new realSqlite.DatabaseSync(p);                 // a separate setup connection, never an importer statement
      try { db.prepare(a.sql).run(...(a.params ?? []).map(typedToJs)); } finally { db.close(); }
      break;
    }
    default: throw new HarnessError('unknown schedule action ' + a.kind);
  }
}

function runRules(after, ctx) {
  for (const r of S.rules) {
    if (r.after !== after || !ruleHits(r, ctx)) continue;
    emit(after ? 'barrier-after-result-before-delivery' : 'barrier-before-call', ctx.id, ctx.op, ctx.stage, ctx.handle,
      { phase: ctx.phase ?? null, path: ctx.key ?? null }, null, null);
    for (const a of r.actions) {
      emit('schedule', ctx.id, ctx.op, ctx.stage, ctx.handle, { kind: a.kind, path: a.path }, null, null);
      // `@N` (the random suffix of the Nth mkdtemp result) is also resolved in action paths; the trace keeps the path as written
      const real = { ...a };
      for (const k of ['path', 'target', 'of']) {
        if (typeof a[k] !== 'string') continue;
        real[k] = resolveRulePath(a[k]);
        if (real[k] === null) throw new HarnessError(`schedule action ${a.kind}: ${a[k]} names a temp directory that does not exist yet`);
      }
      try { runAction(real); } catch (e) { throw new HarnessError(`schedule action ${a.kind} failed: ${e.message}`); }
    }
    if (r.error) return injectedError(r.error, ctx.op, ctx.key);
  }
  return null;
}

// ---- the common primitive wrapper (sync and async) -------------------------------------------------------------------------
function begun(op, info) {
  const id = S.nextId++;
  const k = `${op}\0${info.phase ?? ''}\0${info.key ?? ''}`;
  const occ = (S.occ.get(k) ?? 0) + 1;
  S.occ.set(k, occ);
  S.events.push({ op, stage: info.stage, key: info.key ?? null });
  const ctx = { id, op, stage: info.stage, handle: info.handle ?? null, key: info.key ?? null, phase: info.phase ?? null, occ, evIndex: S.events.length - 1 };
  emit('issue', id, op, info.stage, ctx.handle, info.args, null, null);
  return ctx;
}

function failed(ctx, err) {
  if (err instanceof HarnessError) return err;
  if (err && typeof err === 'object' && err[EVENT] === undefined) { try { err[EVENT] = ctx.evIndex; } catch { /* frozen */ } }
  const info = errInfo(err);
  if (!info) throw new HarnessError(`unclassifiable error in ${ctx.op}: ${err?.stack ?? err}`);
  emit('complete', ctx.id, ctx.op, ctx.stage, ctx.handle, null, null, { class: info.class, code: info.code, native: info.native });
  return err;
}

async function run(op, info, call, describe = () => ({})) {
  const ctx = begun(op, info);
  const pre = runRules(false, ctx);
  if (pre) throw failed(ctx, pre);
  let res;
  try { res = await call(); } catch (e) { throw failed(ctx, e); }
  const post = runRules(true, ctx);
  if (post) throw failed(ctx, post);
  emit('complete', ctx.id, op, info.stage, ctx.handle, null, describe(res), null);
  return res;
}

function runSync(op, info, call, describe = () => ({})) {
  const ctx = begun(op, info);
  const pre = runRules(false, ctx);
  if (pre) throw failed(ctx, pre);
  let res;
  try { res = call(); } catch (e) { throw failed(ctx, e); }
  const post = runRules(true, ctx);
  if (post) throw failed(ctx, post);
  emit('complete', ctx.id, op, info.stage, ctx.handle, null, describe(res), null);
  return res;
}

function kindOf(s) {
  if (s.isFile()) return 1;
  if (s.isDirectory()) return 2;
  if (s.isSymbolicLink()) return 3;
  if (s.isFIFO()) return 4;
  if (s.isSocket()) return 5;
  if (s.isCharacterDevice()) return 6;
  if (s.isBlockDevice()) return 7;
  return 8;
}
const statResult = s => ({ kind: kindOf(s), size: Number(s.size) });

// ---- node:fs/promises ------------------------------------------------------------------------------------------------------
export function realpath(path) {
  const stage = ++S.nRealpath === 1 ? STAGE.dbParent : STAGE.dbRealpath;
  return run('realpath', { stage, key: path, args: { path } }, () => realFsp.realpath(path), r => ({ path: r }));
}
export function lstat(path) {
  return run('lstat', { stage: STAGE.dbLstat, key: path, args: { path } }, () => realFsp.lstat(path), statResult);
}
export function stat(path) {
  return run('stat', { stage: STAGE.outputStat, key: path, args: { path } }, () => realFsp.stat(path), statResult);
}
export function mkdtemp(prefix) {
  return run('mkdtemp', { stage: STAGE.tempCreate, key: prefix, args: { prefix } }, async () => {
    const p = await realFsp.mkdtemp(prefix);
    S.tempDirs.push(p);
    return p;
  }, p => ({ path: p }));
}
export function rm(path, options) {
  if (!options || options.recursive !== true || options.force !== true) throw new HarnessError('rm must be recursive+force');
  return run('rm', { stage: STAGE.cleanup, key: path, args: { path, recursive: true, force: true } }, () => realFsp.rm(path, options));
}
export function readFile(path, encoding) {
  if (encoding !== 'utf8') throw new HarnessError('readFile must ask for utf8');
  let raw;
  return run('read', { stage: STAGE.outputRead, key: path, args: { path } }, async () => { raw = await realFsp.readFile(path); return raw.toString('utf8'); },
    () => ({ data: artifact(raw) }));
}

class OutFile {
  constructor(fh, path) {
    this.fh = fh; this.path = path; this.id = S.nextHandle++; this.closed = false;
    S.files.set(this.id, this); S.fds.set(fh.fd, this);
  }
  get fd() { return this.fh.fd; }
  close() {
    if (this.closed) return Promise.resolve();
    return run('close', { stage: STAGE.outputClose, key: this.path, handle: this.id, args: {} }, async () => { await this.fh.close(); this.closed = true; });
  }
}

export function open(path, flags, mode) {
  if (flags !== 'wx') throw new HarnessError(`open flags ${flags} are not part of the OpenCode flow`);
  return run('open', { stage: STAGE.outputOpen, key: path, args: { path, flags: 'wx', mode } },
    async () => new OutFile(await realFsp.open(path, flags, mode), path), f => ({ handle: f.id }));
}

// ---- node:os ---------------------------------------------------------------------------------------------------------------
export function homedir() { return S.homedir; }

// ---- node:sqlite -----------------------------------------------------------------------------------------------------------
function safeCell(v, column) {
  if (typeof v === 'bigint') {
    if (v > SAFE || v < -SAFE) {
      throw Object.assign(new RangeError(`The value of column "${column}" is too large to be represented as a JavaScript number`), { code: 'ERR_OUT_OF_RANGE' });
    }
    return Number(v);
  }
  return v;
}

class Statement {
  constructor(db, real, sql, id, stage) { this.db = db; this.real = real; this.sql = sql; this.id = id; this.stage = stage; }
}

export class DatabaseSync {
  constructor(path, options) {
    if (!options || options.readOnly !== true) throw new HarnessError('DatabaseSync must open readOnly');
    this.path = path;
    this.real = runSync('db_open_readonly', { stage: STAGE.dbOpen, key: path, args: { path } }, () => new realSqlite.DatabaseSync(path, { readOnly: true }),
      () => ({ connection: this.id = S.nextHandle++ }));
    S.conns.set(this.id, this);
    this.closedFlag = false;
  }
  prepare(sql) {
    const stage = sql === SCHEMA_SQL ? (S.schemaPrepares++ === 0 ? STAGE.schemaSession : STAGE.schemaMessage) : sql === COUNT_SQL ? STAGE.countPrepare : STAGE.select;
    let id = 0;
    let names = [];
    const real = runSync('db_query', { stage, key: sql, phase: 'prepare', handle: this.id, args: { phase: 'prepare', sql } }, () => {
      const r = this.real.prepare(sql);
      names = r.columns().map(c => c.name);
      return r;
    }, () => ({ statement: id = S.nextHandle++, columns: names }));
    const st = new Statement(this, real, sql, id, stage);
    const facade = {
      all: (...params) => runStatement(st, 'all', params),
      get: (...params) => runStatement(st, 'get', params),
    };
    return facade;
  }
  close() {
    if (this.closedFlag) return;
    runSync('db_close', { stage: STAGE.dbClose, key: this.path, handle: this.id, args: {} }, () => { this.real.close(); this.closedFlag = true; });
  }
}

function runStatement(st, phase, params) {
  const stage = st.sql === COUNT_SQL ? STAGE.count : st.stage;
  const info = { stage, key: st.sql, phase, handle: st.id, args: { phase, sql: st.sql, params: params.map(typed) } };
  let names = [];
  let traced = [];
  return runSync('db_query', info, () => {
    st.real.setReadBigInts(true);
    let rows;
    try {
      names = st.real.columns().map(c => c.name);
      rows = phase === 'get' ? [st.real.get(...params)].filter(r => r !== undefined) : st.real.all(...params);
    } finally { st.real.setReadBigInts(false); }
    traced = rows.map(r => names.map(n => typed(r[n])));
    const out = rows.map(r => {
      const o = Object.create(null);
      for (const n of names) o[n] = safeCell(r[n], n);        // reproduces the default safe-range check; any unsafe INTEGER fails the whole query
      return o;
    });
    return phase === 'get' ? out[0] : out;
  }, () => ({ columns: names, rows: traced }));
}

// ---- Date.now / timers / process.kill (active only during a serial case) ----------------------------------------------------
function mono() { return Math.floor(performance.now() - S.t0); }

function wrappedNow() {
  if (!S.active) return saved.dateNow();
  return runSync('clock_now', { stage: STAGE.listClock, args: { clock: 'wall' } }, () => S.nowMs ?? saved.dateNow(), ms => ({ ms }));
}

/** A recorded wait timer of the pinned waitExport call: registration -> clock_now + observe issue; firing -> observe complete. */
function wrappedSetTimeout(fn, ms, ...rest) {
  const c = S.child;
  if (!S.active || !c || c.done || typeof fn !== 'function') return saved.setTimeout(fn, ms, ...rest);
  const index = c.regs++;
  const want = [S.rpcMs, 1500, 1000][index];
  if (want === undefined || ms !== want) throw new HarnessError(`unexpected timer #${index} with ${ms} ms during the export wait (expected ${want})`);
  const stage = STAGE.wait + index;
  const idc = S.nextId++;
  emit('issue', idc, 'clock_now', stage, null, { clock: 'monotonic' }, null, null);
  const start = mono();
  emit('complete', idc, 'clock_now', stage, null, null, { ms: start }, null);
  const ido = S.nextId++;
  const rec = { id: ido, stage, deadline: start + ms, fired: false, cancelled: false };
  c.pending = rec;
  emit('issue', ido, 'observe', stage, c.id, { deadlineMs: rec.deadline }, null, null);
  const t = saved.setTimeout(() => {
    if (rec.cancelled) return;
    rec.fired = true;
    if (c.pending === rec) {
      c.pending = null;
      emit('complete', ido, 'observe', stage, c.id, null, observation(c, 0), null);
    }
    fn(...rest);
  }, ms);
  rec.timer = t;
  c.timers.add(rec);
  return t;
}

function wrappedClearTimeout(t) {
  const c = S.child;
  if (c) for (const rec of c.timers) if (rec.timer === t) { rec.cancelled = true; if (c.pending === rec) c.pending = null; }
  return saved.clearTimeout(t);
}

function observation(c, eventKind, code = null, signal = null) {
  const cp = c.cp;
  return {
    eventKind, codePresent: eventKind === 1 && code !== null ? 1 : 0, exitCode: eventKind === 1 ? (code ?? 0) : 0,
    signalNumber: eventKind === 1 && signal ? (osConstants.signals[signal] ?? 0) : 0, monoMs: mono(),
    exited: cp.exitCode !== null || cp.signalCode !== null ? 1 : 0,
  };
}

function wrappedKill(pid, signal) {
  const c = S.child;
  if (!S.active || !c || pid !== -c.cp.pid) throw new HarnessError(`process.kill(${pid}, ${signal}) is not for the known export child`);
  const num = osConstants.signals[signal];
  const stage = signal === 'SIGKILL' ? STAGE.kill : STAGE.term;
  let sent = 0;
  const err = (() => {
    try {
      runSync('kill', { stage, key: c.binary, handle: c.id, args: { signal: num, group: true } }, () => { saved.kill(pid, signal); sent = 1; }, () => ({ sent }));
      return null;
    } catch (e) { return e; }
  })();
  if (err) throw err;
  return true;
}

// ---- node:child_process (stubs.ts) -------------------------------------------------------------------------------------------
export function spawn(command, args, options) {
  const stdio = options?.stdio;
  if (!Array.isArray(stdio) || stdio[0] !== 'ignore' || typeof stdio[1] !== 'number' || stdio[2] !== 'ignore' || options.shell !== false) {
    throw new HarnessError('unexpected spawn options ' + JSON.stringify({ stdio, shell: options?.shell }));
  }
  const out = S.fds.get(stdio[1]);
  if (!out) throw new HarnessError('spawn stdout fd is not an open output file of this case');
  const fromParent = options.env === undefined || options.env === process.env;
  const entries = Object.entries(fromParent ? process.env : options.env).map(([k, v]) => [k, String(v)])
    .sort((a, b) => Buffer.compare(Buffer.from(a[0]), Buffer.from(b[0])));
  const argsView = {
    binary: command, cwd: options.cwd, argv: [...args], envMode: fromParent ? 0 : 1, env: entries, shell: 0, stdin: 'ignore', stderr: 'ignore',
    detached: options.detached === true, windowsHide: options.windowsHide === true, stdout: { kind: 'file', path: out.path },
  };
  let id = 0;
  const cp = runSync('spawn', { stage: STAGE.spawn, key: command, handle: out.id, args: argsView }, () => realCp.spawn(command, args, options),
    () => ({ child: id = S.nextHandle++ }));
  const c = { id, cp, binary: command, regs: 0, pending: null, timers: new Set(), done: false };
  S.child = c;
  S.children.push(c);
  // observers are attached before waitExport's own handlers, so they see each event first and never reorder anything
  cp.on('error', e => {
    if (c.pending) {
      const rec = c.pending;
      c.pending = null;
      tagError(e, STAGE.wait);
      const info = errInfo(e) ?? { class: 1, code: e.code ?? 'EIO', native: 0 };
      emit('complete', rec.id, 'observe', rec.stage, c.id, null, observation(c, 2), { class: info.class, code: info.code, native: info.native });
    }
    c.done = true;
  });
  cp.on('close', (code, signal) => {
    if (c.pending) {
      const rec = c.pending;
      c.pending = null;
      emit('complete', rec.id, 'observe', rec.stage, c.id, null, observation(c, 1, code, signal), null);
    }
    c.done = true;
  });
  const realKillMethod = cp.kill.bind(cp);
  cp.kill = signal => { void realKillMethod; throw new HarnessError(`child.kill(${signal}) is outside the POSIX group-signal flow`); };
  return cp;
}
