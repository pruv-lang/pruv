// Runs the pinned TypeScript Grok exports, one JSON case per stdin line, one JSON result per stdout line.
//   GROK_TS_TRANSPILER=... node --experimental-strip-types --import ./reference-hooks.mjs reference.mjs < cases.jsonl
// Case: {mode, sourceHome, folder, thread, targetHome, binary, sourceEngine, smallPath, smallHomeReal,
//        cleanupPath, rpcMs, schedule, trace}; mode is list | read | copy | load | small-json | caller-cleanup.
// First stdout line: {"meta":{"node":..., "v8":...}}. Refuses to run unless Node is 26.4.0.
// Result (same keys as the Rust host; N/A fields are null / [] / false / 0):
//   {status:"ok"|"error", mode, omittedBefore, rows, title, model, value, present, copySkipped,
//    error: null | {stage, class, code, errnoClass, native, raw?, name, detail}}
//   rows: list items / read turns as the TS returns them; value: copy path or small-json value;
//   stage: name from DESIGN §2 (input, realHome, ... result); class: EngineImportError | AbortError |
//   RpcResponseError | RawOsError | URIError | CoreError | OtherError | HarnessError;
//   code: EngineImportError code or null; errnoClass: none|ENOENT|EACCES|EEXIST|other; native: errno (>= 0);
//   raw: OS error code string, or an RpcResponseError's raw code JSON (key absent when the code is missing);
//   detail: message (diagnostic, never compared).
// Trace lines go to stderr (see reference-primitives.mjs).
import { createInterface } from 'node:readline';
import { constants as osConstants } from 'node:os';

const NODE_WANTED = '26.4.0';
process.stdout.write(JSON.stringify({ meta: { node: process.versions.node, v8: process.versions.v8 } }) + '\n');
if (process.versions.node !== NODE_WANTED) {
  process.stderr.write(`reference.mjs needs Node ${NODE_WANTED}, this is ${process.versions.node}\n`);
  process.exit(3);
}

const prims = await import('./reference-primitives.mjs');
const grok = await import('./reference/grok.ts');
const tail = await import('./reference/tail.ts');

const ERRNO_CLASSES = new Set(['ENOENT', 'EACCES', 'EEXIST']);

function osError(e) {
  return e && typeof e === 'object' && typeof e.code === 'string' && /^E[A-Z0-9]+$/.test(e.code) && ('errno' in e || 'syscall' in e);
}

function errnoOf(e) {
  for (let x = e, n = 0; x && typeof x === 'object' && n < 8; x = x.cause, n++) {
    if (osError(x)) {
      const native = typeof x.errno === 'number' ? Math.abs(x.errno) : (osConstants.errno[x.code] ?? 0);
      return { errnoClass: ERRNO_CLASSES.has(x.code) ? x.code : 'other', native, raw: x.code };
    }
  }
  return { errnoClass: 'none', native: 0, raw: undefined };
}

/** Stage from the trace position of the failing primitive (see labelFor in reference-primitives.mjs). */
function stageOf(e, mode) {
  if (e instanceof URIError) return 'input';
  const acp = mode === 'load' || e instanceof tail.RpcResponseError
    || (e instanceof tail.EngineImportError && ['grok-load-failed', 'engine-missing', 'timeout'].includes(e.code));
  if (acp) return prims.acpStage();
  const tagged = prims.eventIndexOf(e);
  if (tagged !== null) return prims.eventAt(tagged).stage;
  // a decision made after the last primitive: cleanup (rm) comes after the original failure, so it is skipped
  const last = prims.lastEvent(ev => ev.op === 'rm' && mode !== 'caller-cleanup');
  if (!last) return 'input';
  if (mode === 'read' && e instanceof tail.EngineImportError && e.code === 'not-found' && last.op === 'close' && /\/summary\.json$/.test(last.path ?? '')) return 'summary';
  return last.stage;
}

function describe(e, mode) {
  const { errnoClass, native, raw } = errnoOf(e);
  const error = { stage: stageOf(e, mode), class: 'OtherError', code: null, errnoClass, native, name: e?.name ?? null, detail: String(e?.message ?? e) };
  if (e instanceof tail.EngineImportError) { error.class = 'EngineImportError'; error.code = e.code; }
  else if (e instanceof tail.RpcResponseError) { error.class = 'RpcResponseError'; error.errnoClass = 'none'; error.native = 0; if (e.code !== undefined) error.raw = e.code; }
  else if (e instanceof URIError) error.class = 'URIError';
  else if (e?.name === 'AbortError') error.class = 'AbortError';
  else if (e instanceof prims.HarnessError) error.class = 'HarnessError';
  else if (osError(e)) { error.class = 'RawOsError'; error.raw = raw; }
  return error;
}

function result(mode, fields, error) {
  return {
    status: error ? 'error' : 'ok', mode, omittedBefore: false, rows: [], title: null, model: null, value: null, present: false,
    copySkipped: 0, ...fields, error: error ?? null,
  };
}

async function dispatch(spec, signal) {
  const source = { source: 'q', engine: spec.sourceEngine, home: spec.sourceHome, kind: 'home' };
  const verify = home => grok.grokLoadCheck({ binary: spec.binary, env: process.env }, home, spec.folder, spec.thread, signal);
  switch (spec.mode) {
    case 'list':
      return { rows: await grok.listGrok(source, spec.folder, { signal }) };
    case 'read': {
      const r = await grok.readGrok(source, spec.folder, spec.thread, { signal });
      return { rows: r.turns, omittedBefore: r.omittedBefore, title: r.title ?? null, model: r.model ?? null };
    }
    case 'copy': {
      const r = await grok.copyGrokSession(source, spec.folder, spec.thread, spec.targetHome, verify);
      return { value: r.path, copySkipped: r.skipped };
    }
    case 'load':
      await verify(spec.targetHome);
      return {};
    case 'small-json': {
      const v = await grok.readSmallJson(spec.smallHomeReal, spec.smallPath);
      return { present: v !== undefined, value: v === undefined ? null : v };
    }
    case 'caller-cleanup':
      await prims.rm(spec.cleanupPath, { recursive: true, force: true });   // the open caller's rm of the returned copy
      return {};
    default:
      throw new prims.HarnessError(`unknown mode ${spec.mode}`);
  }
}

async function runCase(spec) {
  prims.begin(spec);
  const before = tail.IMPORT_LIMITS.rpcMs;
  let out;
  try {
    tail.IMPORT_LIMITS.rpcMs = spec.rpcMs ?? 20000;
    out = result(spec.mode, await dispatch(spec, new AbortController().signal), null);
  } catch (e) {
    out = result(spec.mode, {}, describe(e, spec.mode));
  } finally {
    tail.IMPORT_LIMITS.rpcMs = before;
  }
  await prims.settle();
  return out;
}

for await (const line of createInterface({ input: process.stdin })) {
  if (!line.trim()) continue;
  const spec = JSON.parse(line);
  let out;
  try { out = await runCase(spec); } catch (e) { out = result(spec.mode, {}, describe(e, spec.mode)); }
  process.stdout.write(JSON.stringify(out) + '\n');
}
// exit only after stdout drains: a bare exit cuts a large last result short on a pipe
process.stdout.write('', () => process.exit(0));
