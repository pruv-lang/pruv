// Scripted observation primitives and shared per-case oracle state for the pinned entry reference (DESIGN sections 5.1, 6).
// reference-hooks.mjs redirects the pinned files' node:os homedir and node:fs realpathSync/statSync imports here; the fake
// engine modules call engineCall; reference.mjs drives begin/end and the JV codec. One module instance serves all of them.
//
// Script semantics (DESIGN 5.1): queues are consumed in call order and the last entry repeats once exhausted. Defaults:
// homedir /home/test, cwd /work, ambient undefined, realpath/stat ENOENT, accounts {"ok":[]}, isTemporary {"ok":false}.
import { EngineImportError } from './reference/backend-engine-import-tail.ts';

/** Thrown for anything the oracle itself must refuse (unscripted process.env read, bad fixture, bad script). */
export class OracleError extends Error {
  constructor(message) { super(message); this.name = 'OracleError'; }
}

const ABSENT = Symbol('absent');
export { ABSENT };

let ctx = null; // the active case, or null between cases

// ---- JV codec (DESIGN 5.1) ------------------------------------------------------------------------------------------
const MARKERS = ['u16', 'undef', 'num', 'ref', 'arr', 'obj', 'absent'];

function isDict(v) { return v !== null && typeof v === 'object' && !Array.isArray(v); }

function numberOfBits(hex) {
  if (typeof hex !== 'string' || !/^[0-9a-fA-F]{16}$/.test(hex)) throw new OracleError(`bad num bits ${JSON.stringify(hex)}`);
  const view = new DataView(new ArrayBuffer(8));
  view.setBigUint64(0, BigInt('0x' + hex));
  return view.getFloat64(0);
}

function bitsOfNumber(n) {
  const view = new DataView(new ArrayBuffer(8));
  view.setFloat64(0, n);
  return view.getBigUint64(0).toString(16).padStart(16, '0');
}

/** The sentinel object of a name, the same identity for the whole case. */
export function refOf(name) {
  if (typeof name !== 'string') throw new OracleError(`ref name must be a string: ${JSON.stringify(name)}`);
  if (!ctx) throw new OracleError('ref used outside a case');
  let value = ctx.refs.get(name);
  if (value === undefined && !ctx.refs.has(name)) { value = {}; ctx.refs.set(name, value); }
  return value;
}

/**
 * JV -> JS value. A JSON object with none of the JV marker keys is a record: its members decode recursively and are assigned
 * in listed order (absent members are skipped). That is how session, source and row fields are written.
 */
export function dec(jv) {
  if (jv === null || typeof jv === 'boolean' || typeof jv === 'string') return jv;
  if (typeof jv === 'number') {
    if (!Number.isSafeInteger(jv)) throw new OracleError(`JSON number is not a safe integer: ${jv}`);
    return jv;
  }
  if (Array.isArray(jv)) throw new OracleError('bare JSON array is not a JV (use {"arr":[...]})');
  if (!isDict(jv)) throw new OracleError(`not a JV: ${String(jv)}`);
  const keys = Object.keys(jv);
  const marker = keys.find(key => MARKERS.includes(key));
  if (marker !== undefined && keys.length === 1) {
    const body = jv[marker];
    switch (marker) {
      case 'u16': return String.fromCharCode(...body.map(unit => { if (!Number.isInteger(unit) || unit < 0 || unit > 0xffff) throw new OracleError('bad u16 unit'); return unit; }));
      case 'undef': return undefined;
      case 'num': return numberOfBits(body);
      case 'ref': return refOf(body);
      case 'arr': return body.map(item => { const value = dec(item); if (value === ABSENT) throw new OracleError('absent inside an array'); return value; });
      case 'obj': {
        const out = {};
        for (const [raw, item] of body) {
          // A key is a JSON string or, for an ill-formed key, a {"u16":[...]} marker (the same key form the host accepts).
          const key = typeof raw === 'string' ? raw : dec(raw);
          if (typeof key !== 'string') throw new OracleError('object key must be a string or {"u16":[...]}');
          const value = dec(item); if (value !== ABSENT) put(out, key, value);
        }
        return out;
      }
      case 'absent': return ABSENT;
    }
  }
  const out = {};
  for (const key of keys) { const value = dec(jv[key]); if (value !== ABSENT) put(out, key, value); }
  return out;
}

/**
 * Set an own data property, like JSON.parse does: a plain assignment of "__proto__" would call the prototype setter (and drop
 * a non-object value) instead of creating a key. Assignment order semantics (integer keys first) are unchanged.
 */
export function put(target, key, value) {
  Object.defineProperty(target, key, { value, writable: true, enumerable: true, configurable: true });
}

/** A well-formed string as a JSON string, otherwise {"u16":[...]}. */
export function encString(s) {
  if (s.isWellFormed()) return s;
  const units = [];
  for (let i = 0; i < s.length; i += 1) units.push(s.charCodeAt(i));
  return { u16: units };
}

/** JS value -> JV. Registered refs and thrown values map back to their names by Object.is. */
export function enc(value) {
  if (value === null || typeof value === 'boolean') return value;
  if (value === undefined) return { undef: 1 };
  if (typeof value === 'string') return encString(value);
  if (typeof value === 'number') return Number.isSafeInteger(value) && !Object.is(value, -0) ? value : { num: bitsOfNumber(value) };
  if (typeof value === 'bigint') return { opaque: `bigint:${value}` };
  const name = nameOf(value);
  if (name !== undefined) return { ref: name };
  if (Array.isArray(value)) return { arr: value.map(enc) };
  if (typeof value === 'object' && Object.getPrototypeOf(value) === Object.prototype) return { obj: Object.keys(value).map(key => [key, enc(value[key])]) };
  return { opaque: value?.constructor?.name ?? typeof value };
}

/** The ref name of a registered sentinel, error or primitive thrown value (Object.is), else undefined. */
export function nameOf(value) {
  if (!ctx) return undefined;
  // A thrown primitive has no identity of its own: the host names every thrown value by its ERR ref, so the most recent
  // scripted throw of an Object.is-equal primitive supplies the name (IMPL_NOTES_W4fix).
  if (value === null || (typeof value !== 'object' && typeof value !== 'function')) {
    for (let i = ctx.thrownLog.length - 1; i >= 0; i -= 1) {
      const name = ctx.thrownLog[i];
      if (Object.is(ctx.refs.get(name), value)) return name;
    }
  }
  for (const [name, known] of ctx.refs) if (Object.is(known, value)) return name;
  return undefined;
}

// ---- thrown-value (ERR) builder --------------------------------------------------------------------------------------
function isErr(x) { return isDict(x) && typeof x.class === 'string' && typeof x.ref === 'string'; }

function buildErr(spec) {
  const message = spec.message === undefined ? undefined : dec(spec.message);
  let error;
  switch (spec.class) {
    case 'Error': error = new Error(message ?? ''); break;
    case 'TypeError': error = new TypeError(message ?? ''); break;
    case 'EngineImportError': error = new EngineImportError(dec(spec.code), message); break;
    case 'plain': {
      error = {};
      if (spec.name !== undefined) error.name = dec(spec.name);
      if (spec.message !== undefined) error.message = message;
      break;
    }
    case 'primitive': return dec(spec.value);
    default: throw new OracleError(`unknown ERR class ${spec.class}`);
  }
  if (spec.class !== 'plain' && spec.name !== undefined) error.name = dec(spec.name);
  if (spec.errcode !== undefined) error.code = dec(spec.errcode);
  if (spec.rename !== undefined) error.name = spec.rename;
  return error;
}

function walkErrs(node, visit) {
  if (Array.isArray(node)) { for (const item of node) walkErrs(item, visit); return; }
  if (!isDict(node)) return;
  if (isErr(node)) visit(node);
  for (const key of Object.keys(node)) walkErrs(node[key], visit);
}

/** The thrown value of an ERR spec, built once per ref name. */
function errValue(spec) {
  if (!isErr(spec)) throw new OracleError(`bad ERR ${JSON.stringify(spec)}`);
  if (ctx.refs.has(spec.ref)) return ctx.refs.get(spec.ref);
  const value = buildErr(spec);
  ctx.refs.set(spec.ref, value);
  return value;
}

/** Serve a scripted thrown value and remember which ref threw it (needed for primitives). */
function thrown(spec) {
  const value = errValue(spec);
  ctx.thrownLog.push(spec.ref);
  return value;
}

const DESCRIPTION = {
  ENOENT: 'no such file or directory', EACCES: 'permission denied', ELOOP: 'too many symbolic links encountered', ENOTDIR: 'not a directory',
  EPERM: 'operation not permitted', EIO: 'i/o error', ENAMETOOLONG: 'name too long', EMFILE: 'too many open files', EISDIR: 'illegal operation on a directory',
};
const ERRNO = { ENOENT: -2, EACCES: -13, ELOOP: -62, ENOTDIR: -20, EPERM: -1, EIO: -5, ENAMETOOLONG: -63, EMFILE: -24, EISDIR: -21 };

function errnoError(entry, syscall, path) {
  const cached = entry.ref === undefined ? undefined : ctx.refs.get(entry.ref);
  if (cached instanceof Error) return cached;
  const code = entry.errno;
  const error = new Error(`${code}: ${DESCRIPTION[code] ?? 'error'}, ${syscall} '${path}'`);
  error.errno = ERRNO[code] ?? -1;
  error.code = code;
  error.syscall = syscall;
  error.path = path;
  if (entry.ref !== undefined) ctx.refs.set(entry.ref, error);
  return error;
}

// ---- per-case state --------------------------------------------------------------------------------------------------
/** Entry of a call-order queue: the n-th call takes entry n, the last entry repeats. */
function take(key, queue, fallback) {
  const list = Array.isArray(queue) ? queue : queue === undefined ? [] : [queue];
  if (list.length === 0) return fallback;
  const used = ctx.used.get(key) ?? 0;
  ctx.used.set(key, used + 1);
  return list[Math.min(used, list.length - 1)];
}

/** A scripted success string is a JV like every other script string, so a {"u16":[...]} (lone surrogates) is decoded. */
function scriptedString(jv, what) {
  const value = dec(jv);
  if (typeof value !== 'string') throw new OracleError(`scripted ${what} result must be a string`);
  return value;
}

/** A scripted dev/ino: a decimal string or a non-negative integer. Canonical decimal text comes from BigInt, so leading zeros
 * normalize ("01" and "1" name the same number); the host does the same (DESIGN 5.1). Anything else is refused. */
function decimalOf(raw, what) {
  const text = typeof raw === 'number' && Number.isSafeInteger(raw) ? String(raw) : raw;
  if (typeof text !== 'string' || !/^[0-9]+$/.test(text)) throw new OracleError(`stat ${what} must be a decimal digit string: ${JSON.stringify(raw)}`);
  return BigInt(text);
}

function serve(entry) {
  if (isDict(entry) && 'throw' in entry) throw thrown(entry.throw);
  return entry.ok;
}

const realProcess = { cwd: process.cwd, envDescriptor: Object.getOwnPropertyDescriptor(process, 'env') };

function envProxy() {
  const refuse = what => { const error = new OracleError(`unscripted process.env ${what}`); ctx.violations.push(error.message); throw error; };
  return new Proxy({}, {
    get(_target, key) {
      if (typeof key === 'symbol') return undefined;
      if (key !== 'CODEX_HOME') return refuse(`read of ${key}`);
      ctx.trace.push(['ambient', 'CODEX_HOME']);
      const entry = take('ambient', ctx.script.ambient, { ok: { undef: 1 } });
      if (isDict(entry) && 'throw' in entry) throw thrown(entry.throw);
      return dec(entry.ok);
    },
    has(_target, key) { return refuse(`has ${String(key)}`); },
    ownKeys() { return refuse('ownKeys'); },
    getOwnPropertyDescriptor(_target, key) { return refuse(`descriptor ${String(key)}`); },
    set(_target, key) { return refuse(`write ${String(key)}`); },
    deleteProperty(_target, key) { return refuse(`delete ${String(key)}`); },
    defineProperty(_target, key) { return refuse(`define ${String(key)}`); },
  });
}

/** Start a case: install the process patches and the scripted state. */
export function begin(c) {
  if (ctx) throw new OracleError('begin inside a case');
  const script = c.script ?? {};
  ctx = { c, script, trace: [], refs: new Map(), used: new Map(), violations: [], rows: new Map(), thrownLog: [] };
  walkErrs(script, spec => { if (!ctx.refs.has(spec.ref)) ctx.refs.set(spec.ref, buildErr(spec)); });
  process.cwd = () => {
    ctx.trace.push(['cwd']);
    return scriptedString(serve(take('cwd', ctx.script.cwd, { ok: '/work' })), 'cwd');
  };
  Object.defineProperty(process, 'env', { value: envProxy(), configurable: true, writable: true, enumerable: true });
  return ctx;
}

/** End the case: restore the process patches; returns the trace and the violations. */
export function end() {
  process.cwd = realProcess.cwd;
  Object.defineProperty(process, 'env', realProcess.envDescriptor);
  const done = ctx;
  ctx = null;
  return done;
}

export function currentCase() { return ctx; }

// ---- node:os / node:fs primitives ------------------------------------------------------------------------------------
export function homedir() {
  ctx.trace.push(['homedir']);
  return scriptedString(serve(take('homedir', ctx.script.homedir, { ok: '/home/test' })), 'homedir');
}

function pathCall(op, syscall, path) {
  ctx.trace.push([op, enc(path)]);
  const table = ctx.script[op] ?? {};
  const queue = Object.hasOwn(table, path) ? table[path] : undefined;
  const entry = take(`${op}:${path}`, queue, { errno: 'ENOENT' });
  if (entry.errno !== undefined) throw errnoError(entry, syscall, path);
  return entry;
}

export function realpathSync(path) {
  return scriptedString(pathCall('realpath', 'realpath', path).ok, 'realpath');
}

export function statSync(path) {
  const entry = pathCall('stat', 'stat', path);
  // dev and ino are BigInt so String(info.dev) and === behave like the decimal text of the script (see IMPL_NOTES_W3)
  return { dev: decimalOf(entry.ok.dev, 'dev'), ino: decimalOf(entry.ok.ino, 'ino') };
}

// ---- callbacks served from the script --------------------------------------------------------------------------------
function rowOf(row) {
  if (row === null) return null;
  if (isDict(row) && Object.keys(row).length === 1 && 'undef' in row) return undefined;
  return dec(row);
}

/** The accounts callback of importSources. */
export function accountsCallback(engine) {
  ctx.trace.push(['accounts', engine]);
  const entry = ctx.script.accounts?.[engine] ?? { ok: [] };
  if ('throw' in entry) throw thrown(entry.throw);
  const rows = isDict(entry.ok) && 'arr' in entry.ok ? entry.ok.arr : entry.ok;
  return rows.map(rowOf);
}

/** The isTemporary callback of excludedFromBound / boundSessionIndex. */
export function isTemporaryCallback(id) {
  ctx.trace.push(['isTemporary', enc(id)]);
  return dec(serve(take('isTemporary', ctx.script.isTemporary, { ok: false })));
}

// ---- engine fake backend ---------------------------------------------------------------------------------------------
function rowsOf(entry) {
  let built = ctx.rows.get(entry);
  if (!built) {
    built = entry.rows.map(spec => {
      const row = refOf(spec.ref);
      for (const key of Object.keys(spec)) {
        if (key === 'ref') continue;
        const value = dec(spec[key]);
        if (value !== ABSENT) put(row, key, value);
      }
      return row;
    });
    ctx.rows.set(entry, built);
  }
  return built;
}

/** Called by fakes/engine-*.ts: record the call, serve the engine script or forward entry. */
export function engineCall(route, op, source, folder, thread, isRead, options) {
  const sn = nameOf(source), on = nameOf(options);
  const record = { source: sn !== undefined ? { ref: sn } : enc(source), folder: enc(folder) };
  if (isRead) record.thread = enc(thread);
  record.options = on !== undefined ? { ref: on } : enc(options);
  ctx.trace.push(['engine', route, op, record]);
  const entry = take('engine', ctx.script.engine ?? ctx.c.forward, undefined);
  if (entry === undefined) throw new OracleError('engine call without an engine script');
  if ('sync_throw' in entry) throw thrown(entry.sync_throw);
  if ('reject' in entry) return Promise.resolve().then(() => { throw thrown(entry.reject); });
  if ('record' in entry) return refOf(entry.record);
  if ('rows' in entry) return rowsOf(entry).slice();
  throw new OracleError(`bad engine script entry ${JSON.stringify(entry)}`);
}
