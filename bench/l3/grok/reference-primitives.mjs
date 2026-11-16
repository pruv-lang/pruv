// FS / entropy / spawn shim for the pinned Grok TypeScript (DESIGN §5 "FS shim boundary").
// reference-hooks.mjs redirects the node:fs, node:fs/promises, node:crypto imports of grok.ts / tail.ts and the
// node:child_process import of stubs.ts to the exports below. Every function delegates to the real Node
// function; around the call it adds the schedule rules (barriers, race actions, error injection, holds) and
// writes trace lines to fd 2: {"trace":{event,op,path,id,slot,handle,offset,count,mode,errno,kind,dev,ino,bytes,digest}}
// with logical paths ($SRC / $DST). This module's own builtin imports are not redirected.
import * as realFs from 'node:fs';
import * as realFsp from 'node:fs/promises';
import * as realCrypto from 'node:crypto';
import * as realCp from 'node:child_process';
import { basename, normalize } from 'node:path';
import { constants as osConstants } from 'node:os';
import { createHash } from 'node:crypto';

export const constants = realFs.constants;

export const STAGES = [
  'input', 'realHome', 'chain', 'readdir', 'candidate-dir', 'file-lstat', 'containment', 'open',
  'identity-fstat', 'size-fstat', 'read', 'close', 'parse', 'summary', 'sort', 'active',
  'first-target-lstat', 'entropy', 'parent-mkdir', 'temp-mkdir', 'exclusive-create', 'write', 'second-target-lstat', 'rename',
  'spawn', 'initialize', 'session/load', 'temp-cleanup', 'target-cleanup', 'caller-cleanup', 'result',
];

export class HarnessError extends Error {
  constructor(message) { super(message); this.name = 'HarnessError'; }
}

const EVENT = Symbol.for('grok.reference.event');
let S = fresh({});

function fresh(cfg) {
  const schedule = cfg.schedule ?? {};
  const roots = [];
  for (const [real, logical] of [[cfg.sourceHome, '$SRC'], [cfg.targetHome, '$DST']]) {
    if (typeof real !== 'string' || real === '') continue;
    const root = normalize(real).replace(/\/+$/, '');
    if (root !== '') roots.push([root, logical]);
  }
  roots.sort((a, b) => b[0].length - a[0].length);
  return {
    mode: cfg.mode ?? '', trace: cfg.trace !== false, roots, schedule,
    rules: (schedule.rules ?? []).map(r => ({ ...r, match: { occurrence: 1, barrier: 'before-call', ...r.match } })),
    readdirs: new Map((schedule.readdir ?? []).map(r => [r.path, r.names])),
    entropy: schedule.entropyHex ?? [], entropyIndex: 0, blobs: schedule.blobs ?? {},
    nextId: 1, nextHandle: 1, occ: new Map(), seen: new Map(), events: [], handles: new Map(),
    waiters: [], pending: new Set(), targetLstats: new Map(), acp: { spawned: false, spawnFailed: false, init: false, load: false },
  };
}

// ---- case control (used by reference.mjs) ------------------------------------------------------------------
export function begin(cfg) { S = fresh(cfg); }

/** Wait until no primitive is still running (stream teardown may finish after the importer returned). */
export async function settle() {
  for (let i = 0; i < 3; i++) {
    while (S.pending.size) await Promise.allSettled([...S.pending]);
    await new Promise(resolve => setImmediate(resolve));
  }
}

export function eventIndexOf(err) {
  for (let e = err, n = 0; e && typeof e === 'object' && n < 8; e = e.cause, n++) if (typeof e[EVENT] === 'number') return e[EVENT];
  return null;
}
export function eventAt(i) { return S.events[i]; }
export function lastEvent(skip) {
  for (let i = S.events.length - 1; i >= 0; i--) if (!skip(S.events[i])) return S.events[i];
  return undefined;
}
export function acpStage() {
  const a = S.acp;
  if (a.spawnFailed || !a.spawned) return 'spawn';
  return a.load ? 'session/load' : a.init ? 'initialize' : 'spawn';
}

// ---- paths, trace -------------------------------------------------------------------------------------------
function logical(path) {
  if (typeof path !== 'string') return path == null ? null : String(path);
  const p = normalize(path);
  for (const [root, name] of S.roots) {
    if (p === root) return name;
    if (p.startsWith(root + '/')) return name + p.slice(root.length);
  }
  return p;
}
function physical(path) {
  if (typeof path !== 'string') return path;
  for (const [root, name] of S.roots) {
    if (path === name) return root;
    if (path.startsWith(name + '/')) return root + path.slice(name.length);
  }
  return path;
}

const sha = data => createHash('sha256').update(data).digest('hex');

function kindOf(s) {
  if (s.isFile()) return 'file';
  if (s.isDirectory()) return 'dir';
  if (s.isSymbolicLink()) return 'symlink';
  if (s.isFIFO()) return 'fifo';
  if (s.isSocket()) return 'socket';
  if (s.isCharacterDevice()) return 'char';
  if (s.isBlockDevice()) return 'block';
  return 'other';
}

function writeErr(text) {
  const buf = Buffer.from(text);
  let off = 0;
  while (off < buf.length) {
    try { off += realFs.writeSync(2, buf, off); } catch (e) { if (e.code !== 'EAGAIN') throw e; }
  }
}

function emit(event, op, id, f = {}) {
  S.seen.set(`${event}|${op}|${f.path ?? null}`, (S.seen.get(`${event}|${op}|${f.path ?? null}`) ?? 0) + 1);
  if (S.trace) {
    const t = {
      event, op, path: f.path ?? null, id, slot: null, handle: f.handle ?? null, offset: f.offset ?? null, count: f.count ?? null,
      mode: f.mode ?? null, errno: f.errno ?? null, kind: f.kind ?? null, dev: f.dev ?? null, ino: f.ino ?? null,
      bytes: f.bytes ?? null, digest: f.digest ?? null,
    };
    writeErr(JSON.stringify({ trace: t }) + '\n');
  }
  notify();
}

// ---- schedule: holds, actions, injected errors -------------------------------------------------------------
function condMet(c, heldCount) {
  if (c.held !== undefined) return heldCount >= c.held;
  return (S.seen.get(`${c.event}|${c.op}|${c.path ?? null}`) ?? 0) >= (c.occurrence ?? 1);
}
function notify() {
  const heldCount = S.waiters.length;
  const ready = S.waiters.filter(w => condMet(w.cond, heldCount));
  if (!ready.length) return;
  S.waiters = S.waiters.filter(w => !ready.includes(w));
  for (const w of ready) { clearTimeout(w.timer); w.resolve(); }
}
function hold(rule) {
  return new Promise((resolve, reject) => {
    const w = { cond: rule.releaseWhen ?? {}, resolve };
    w.timer = setTimeout(() => { S.waiters = S.waiters.filter(x => x !== w); reject(new HarnessError(`hold ${rule.hold} never released`)); }, 120_000);
    S.waiters.push(w);
    notify();
  });
}

function bytesOf(r) {
  if (r.blob !== undefined) return bytesOf(S.blobs[r.blob]);
  if (r.utf8 !== undefined) return Buffer.from(r.utf8, 'utf8');
  if (r.hex !== undefined) return Buffer.from(r.hex, 'hex');
  if (r.repeat !== undefined) return Buffer.concat(Array.from({ length: r.count }, () => bytesOf(r.repeat)));
  if (r.concat !== undefined) return Buffer.concat(r.concat.map(bytesOf));
  throw new HarnessError('bad bytes recipe ' + JSON.stringify(r));
}

function runAction(a) {
  const p = physical(a.path);
  switch (a.kind) {
    case 'mkdir': realFs.mkdirSync(p, { mode: a.mode ?? 0o777 }); if (a.mode !== undefined) realFs.chmodSync(p, a.mode); break;
    case 'unlink': realFs.rmSync(p, { recursive: true, force: true }); break;
    case 'replace-file':
      realFs.rmSync(p, { recursive: true, force: true });
      realFs.writeFileSync(p, bytesOf(a.bytes), { mode: a.mode ?? 0o644 });
      if (a.mode !== undefined) realFs.chmodSync(p, a.mode);
      break;
    case 'symlink': realFs.rmSync(p, { recursive: true, force: true }); realFs.symlinkSync(physical(a.target), p); break;
    case 'append': realFs.appendFileSync(p, bytesOf(a.bytes)); break;
    case 'truncate': realFs.truncateSync(p, a.size ?? 0); break;
    default: throw new HarnessError('unknown race action ' + a.kind);
  }
}

function injected(spec, op, realPath) {
  const code = spec.code ?? 'EIO';
  const num = spec.errno ?? osConstants.errno[code] ?? 5;
  const text = { ENOENT: 'no such file or directory', EACCES: 'permission denied', EEXIST: 'file already exists', EIO: 'i/o error',
    ENOSPC: 'no space left on device', EPERM: 'operation not permitted', ENOTDIR: 'not a directory', EISDIR: 'illegal operation on a directory' }[code] ?? code.toLowerCase();
  return Object.assign(new Error(`${code}: ${text}, ${op}${realPath ? ` '${realPath}'` : ''}`), { errno: -Math.abs(num), code, syscall: op, ...(realPath ? { path: realPath } : {}) });
}

/** Barrier of one rule: trace line, race actions, hold (never for sync primitives), injected error. */
function barrierNow(rule, kind, op, id, lp, realPath) {
  emit(`barrier-${kind}`, op, id, { path: lp });
  for (const a of rule.actions ?? []) runAction(a);
  return () => { if (rule.error && (kind === 'before-call' || rule.error)) throw injected(rule.error, op, realPath); };
}
async function barrier(rule, kind, op, id, lp, realPath) {
  const after = barrierNow(rule, kind, op, id, lp, realPath);
  if (rule.hold) await hold(rule);
  after();
}

// ---- stage label of a primitive (position in the flow, DESIGN §2 stage table) --------------------------------
function labelFor(op, lp, handle) {
  const p = lp ?? '';
  switch (op) {
    case 'readdir': return 'readdir';
    case 'entropy': return 'entropy';
    case 'mkdir': return p.includes('.lair-import-') ? 'temp-mkdir' : 'parent-mkdir';
    case 'rename': return 'rename';
    case 'rm': return p.includes('.lair-import-') ? 'temp-cleanup' : S.mode === 'caller-cleanup' ? 'caller-cleanup' : 'target-cleanup';
    case 'create': return 'exclusive-create';
    case 'write': return 'write';
    case 'spawn': return 'spawn';
    case 'open': return 'open';
    case 'read': return 'read';
    case 'realpath': return p === '$SRC' ? 'realHome' : 'containment';
    case 'fstat': return handle.fstats === 1 ? 'identity-fstat' : 'size-fstat';
    case 'close': return handle.kind === 'read' && handle.fstats === 1 && handle.reads === 0 ? 'identity-fstat' : 'close';
    case 'lstat': {
      if (p.startsWith('$DST')) {
        const n = (S.targetLstats.get(p) ?? 0) + 1;
        S.targetLstats.set(p, n);
        return n === 1 ? 'first-target-lstat' : 'second-target-lstat';
      }
      if (S.mode === 'small-json' || p === '$SRC/active_sessions.json') return 'file-lstat';
      const n = p.split('/').length - 1;           // components below $SRC
      if (n <= 2) return 'chain';
      if (n === 3) return S.mode === 'list' ? 'candidate-dir' : 'chain';
      return 'file-lstat';
    }
    default: return 'input';
  }
}

// ---- the common primitive wrapper ---------------------------------------------------------------------------
function begun(op, info) {
  const id = S.nextId++;
  const lp = info.lpath ?? null;
  const key = `${op}\0${lp}`;
  const occ = (S.occ.get(key) ?? 0) + 1;
  S.occ.set(key, occ);
  const evIndex = S.events.length;
  S.events.push({ op, path: lp, stage: labelFor(op, lp, info.handle) });
  emit('issue', op, id, { path: lp, handle: info.handle?.id, offset: info.offset, count: info.count, mode: info.mode });
  const rules = S.rules.filter(r => r.match.op === op && (r.match.path === undefined || r.match.path === lp) && r.match.occurrence === occ);
  return { id, lp, evIndex, rules };
}
function failed(op, info, b, err) {
  if (err && typeof err === 'object' && err[EVENT] === undefined) { try { err[EVENT] = b.evIndex; } catch { /* frozen */ } }
  emit('complete', op, b.id, { path: b.lp, handle: info.handle?.id, errno: err?.code ?? 'EIO' });
  return err;
}

async function run(op, info, call, describe = () => ({})) {
  const b = begun(op, info);
  const job = (async () => {
    try {
      for (const r of b.rules) if (r.match.barrier === 'before-call') await barrier(r, 'before-call', op, b.id, b.lp, info.realPath);
      const result = await call();
      for (const r of b.rules) if (r.match.barrier === 'after-result-before-delivery') await barrier(r, 'after-result-before-delivery', op, b.id, b.lp, info.realPath);
      emit('complete', op, b.id, { path: b.lp, handle: info.handle?.id, ...describe(result) });
      return result;
    } catch (err) { throw failed(op, info, b, err); }
  })();
  const tracked = job.catch(() => undefined);
  S.pending.add(tracked);
  tracked.finally(() => S.pending.delete(tracked));
  return job;
}

function runSync(op, info, call, describe) {
  const b = begun(op, info);
  try {
    for (const r of b.rules) if (r.match.barrier === 'before-call') { if (r.hold) throw new HarnessError('hold on a sync primitive'); barrierNow(r, 'before-call', op, b.id, b.lp, info.realPath)(); }
    const result = call();
    for (const r of b.rules) if (r.match.barrier !== 'before-call') { if (r.hold) throw new HarnessError('hold on a sync primitive'); barrierNow(r, 'after-result-before-delivery', op, b.id, b.lp, info.realPath)(); }
    emit('complete', op, b.id, { path: b.lp, ...describe(result) });
    return result;
  } catch (err) { throw failed(op, {}, b, err); }
}

const statFields = s => ({ kind: kindOf(s), dev: s.dev, ino: s.ino });

// ---- node:fs/promises ---------------------------------------------------------------------------------------
export function lstat(path) { return run('lstat', { lpath: logical(path), realPath: path }, () => realFsp.lstat(path), statFields); }
export function realpath(path) { return run('realpath', { lpath: logical(path), realPath: path }, () => realFsp.realpath(path)); }
export function mkdir(path, options) {
  return run('mkdir', { lpath: logical(path), realPath: path, mode: options?.mode }, () => realFsp.mkdir(path, options));
}
export function rename(from, to) { return run('rename', { lpath: logical(from), realPath: from }, () => realFsp.rename(from, to)); }
export function rm(path, options) { return run('rm', { lpath: logical(path), realPath: path }, () => realFsp.rm(path, options)); }

export async function readdir(path, options) {
  const lp = logical(path);
  const order = S.readdirs.get(lp);
  const entries = await run('readdir', { lpath: lp, realPath: path }, async () => {
    const real = await realFsp.readdir(path, options);
    if (!order) return real;
    const nameOf = e => (typeof e === 'string' ? e : e.name);
    const have = real.map(nameOf).sort();
    if (JSON.stringify(have) !== JSON.stringify([...order].sort())) throw new HarnessError(`readdir schedule for ${lp} differs from the real names: ${JSON.stringify(have)}`);
    const byName = new Map(real.map(e => [nameOf(e), e]));
    return order.map(n => byName.get(n));
  }, list => ({
    count: list.length,
    digest: sha(list.map(e => (typeof e === 'string' ? e : `${e.name}\t${kindOf(e)}`)).join('\n')),
  }));
  return entries;
}

export function open(path, flags) {
  return run('open', { lpath: logical(path), realPath: path }, async () => new RefHandle(await realFsp.open(path, flags), path, 'read'),
    h => ({ handle: h.id }));
}

function createFile(path, flags, mode) {
  return run('create', { lpath: logical(path), realPath: path, mode }, async () => new RefHandle(await realFsp.open(path, flags, mode), path, 'write'),
    h => ({ handle: h.id, mode }));
}

/** The FileHandle facade: observes stat / read / close, readFile built from primitive reads, streams via fd callbacks. */
class RefHandle {
  constructor(fh, path, kind) {
    this.fh = fh; this.path = path; this.lpath = logical(path); this.kind = kind;
    this.id = S.nextHandle++; this.pos = 0; this.fstats = 0; this.reads = 0; this.closed = false; this.closing = null;
    S.handles.set(this.id, this);
  }
  stat() {
    this.fstats++;
    return run('fstat', { lpath: this.lpath, realPath: this.path, handle: this }, () => this.fh.stat(), statFields);
  }
  async read(buffer, offset, length, position) {
    this.reads++;
    const at = position ?? this.pos;
    const res = await run('read', { lpath: this.lpath, realPath: this.path, handle: this, offset: at, count: length },
      () => this.fh.read(buffer, offset, length, at),
      r => ({ bytes: r.bytesRead, digest: sha(buffer.subarray(offset, offset + r.bytesRead)) }));
    if (position == null) this.pos = at + res.bytesRead;
    return res;
  }
  async readFile(encoding) {
    const enc = typeof encoding === 'string' ? encoding : encoding?.encoding;
    const chunks = [];
    for (;;) {
      const buf = Buffer.allocUnsafe(65536);
      const { bytesRead } = await this.read(buf, 0, buf.length, null);
      if (bytesRead === 0) break;
      chunks.push(buf.subarray(0, bytesRead));
    }
    const data = Buffer.concat(chunks);
    return enc ? data.toString(enc) : data;
  }
  async write(buffer, offset, length, position) {
    const at = position ?? this.pos;
    const res = await run('write', { lpath: this.lpath, realPath: this.path, handle: this, offset: at, count: length },
      () => this.fh.write(buffer, offset, length, at),
      r => ({ bytes: r.bytesWritten, digest: sha(buffer.subarray(offset, offset + r.bytesWritten)) }));
    if (position == null) this.pos = at + res.bytesWritten;
    return res;
  }
  async writev(buffers, position) {
    const at = position ?? this.pos;
    const all = Buffer.concat(buffers);
    const res = await run('write', { lpath: this.lpath, realPath: this.path, handle: this, offset: at, count: all.length },
      () => this.fh.writev(buffers, at), r => ({ bytes: r.bytesWritten, digest: sha(all.subarray(0, r.bytesWritten)) }));
    if (position == null) this.pos = at + res.bytesWritten;
    return res;
  }
  close() {
    if (this.closed) return Promise.resolve();
    this.closing ??= (async () => {
      try { await run('close', { lpath: this.lpath, realPath: this.path, handle: this }, () => this.fh.close()); }
      finally { this.closed = true; await this.fh.close().catch(() => undefined); }
    })();
    return this.closing;
  }
  createReadStream(opts = {}) {
    return realFs.createReadStream(null, {
      fd: this.id, start: opts.start, end: opts.end, highWaterMark: opts.highWaterMark, autoClose: true, emitClose: true,
      fs: {
        read: (fd, buffer, offset, length, position, cb) => { this.read(buffer, offset, length, position).then(r => cb(null, r.bytesRead, buffer), cb); },
        close: (fd, cb) => { this.close().then(() => cb(null), cb); },
      },
    });
  }
}

// ---- node:fs ------------------------------------------------------------------------------------------------
export function createWriteStream(path, opts = {}) {
  const handle = fd => S.handles.get(fd);
  return realFs.createWriteStream(path, {
    flags: opts.flags, mode: opts.mode, start: opts.start,
    fs: {
      open: (p, flags, mode, cb) => { createFile(p, flags, mode).then(h => cb(null, h.id), cb); },
      write: (fd, buffer, offset, length, position, cb) => { handle(fd).write(buffer, offset, length, position).then(r => cb(null, r.bytesWritten, buffer), cb); },
      writev: (fd, buffers, position, cb) => { handle(fd).writev(buffers, position).then(r => cb(null, r.bytesWritten, buffers), cb); },
      close: (fd, cb) => { handle(fd).close().then(() => cb(null), cb); },
    },
  });
}

// ---- node:crypto --------------------------------------------------------------------------------------------
export function randomBytes(size) {
  return runSync('entropy', { count: size }, () => {
    const hex = S.entropy[S.entropyIndex++];
    if (hex === undefined) return realCrypto.randomBytes(size);
    const buf = Buffer.from(hex, 'hex');
    if (buf.length !== size) throw new HarnessError(`scheduled entropy has ${buf.length} bytes, asked for ${size}`);
    return buf;
  }, buf => ({ count: size, bytes: buf.length, digest: sha(buf) }));
}

// ---- node:child_process (stubs.ts) --------------------------------------------------------------------------
export function spawn(command, args, options) {
  const id = S.nextId++;
  const lp = basename(command);
  const argsText = JSON.stringify(args);
  S.events.push({ op: 'spawn', path: lp, stage: 'spawn' });
  emit('issue', 'spawn', id, { path: lp, count: args.length, digest: sha(argsText) });
  S.acp.spawned = true;
  const child = realCp.spawn(command, args, options);
  child.once('spawn', () => emit('complete', 'spawn', id, { path: lp, count: args.length, digest: sha(argsText) }));
  child.once('error', err => { S.acp.spawnFailed = true; emit('complete', 'spawn', id, { path: lp, errno: err.code ?? 'EIO' }); });
  if (child.stdin) {
    const write = child.stdin.write.bind(child.stdin);
    child.stdin.write = (chunk, ...rest) => {
      const text = typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString('utf8');
      if (text.includes('"method":"initialize"')) S.acp.init = true;
      if (text.includes('"method":"session/load"')) S.acp.load = true;
      return write(chunk, ...rest);
    };
  }
  return child;
}
