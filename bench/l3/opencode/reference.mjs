// Runs the pinned TypeScript OpenCode exports, one JSON config per stdin line, one JSON envelope per stdout line.
//   GROK_TS_TRANSPILER=... node --experimental-strip-types --import ./reference-hooks.mjs reference.mjs < cases.jsonl
// Config (the same line the Rust host reads, see host.rs header):
//   {mode: list|read|rows|dataDir|folderKey|plan, folder, thread, sourceHome, homedir, env:{mode:inherited|full, values},
//    binary, tempRoot, nowMs, rpcMs, schedule, planJson, trace, artifactDir}
//   binary/tempRoot: absent key = missing, "" = empty (both mean engine-missing for read). env.mode inherited = the parent environment
//   (options.env undefined), full = exactly env.values. rpcMs overrides IMPORT_LIMITS.rpcMs for the case only (restored in finally).
// First stdout line: {"meta":{"node","v8","sqlite"}}. Refuses to run unless Node is 26.4.0.
// Envelope (identical keys to the Rust host's render_result):
//   ok:    list/rows {status, mode, rows}; read {status, mode, turns, omittedBefore, title?, model?, lastPlan?};
//          dataDir/folderKey {status, mode, value}; plan {status, mode, present, value?}
//   error: {status:"error", mode, error:{class, code, name, detail, stage, rawCode, native}}
//          class: EngineImportError | RawOsError | OtherError | HarnessError (CoreError never comes from TS); code: EngineImportError
//          code or null; stage: DESIGN section 5 stage name, taken from the primitive that raised (or the decision after the last one);
//          rawCode/native: first OS / SQLite / range cause in the error chain; detail: columns|json|shape or a diagnostic (never compared).
// Trace records go to stderr (reference-primitives.mjs).
import { createInterface } from 'node:readline';

const NODE_WANTED = '26.4.0';
const prims = await import('./reference-primitives.mjs');
let sqlite = null;
try { sqlite = await prims.sqliteVersion(); } catch { /* reported as null */ }
process.stdout.write(JSON.stringify({ meta: { node: process.versions.node, v8: process.versions.v8, sqlite } }) + '\n');
if (process.versions.node !== NODE_WANTED) {
  process.stderr.write(`reference.mjs needs Node ${NODE_WANTED}, this is ${process.versions.node}\n`);
  process.exit(3);
}

const opencode = await import('../grok/reference/opencode.ts');
const tail = await import('../grok/reference/tail.ts');
const planModule = await import('../grok/reference/plan.ts');

function detailOf(e) {
  const prefix = `${e.code}: `;
  return typeof e.message === 'string' && e.message.startsWith(prefix) ? e.message.slice(prefix.length) : '';
}

/** Stage name of a failure (DESIGN section 5 order). */
function stageOf(e, spec) {
  const tagged = prims.eventIndexOf(e);
  if (tagged !== null) return prims.eventAt(tagged).stage;
  if (e instanceof tail.EngineImportError) {
    const detail = detailOf(e);
    if (detail === 'json') return 24;
    if (detail === 'shape') return 25;
    // withDb's finally closes the DB before the error surfaces; the decision follows the last schema query.
    if (detail === 'columns') return prims.lastEvent(ev => ev.op === 'db_close')?.stage ?? 5;
    if (e.code === 'timeout') return 16;
    if (e.code === 'read-failed' && /^exit /.test(detail)) return 16;
    if (e.code === 'not-found') return typeof spec.thread === 'string' && tail.validThread(spec.thread) ? 11 : 0;
    if (e.code === 'engine-missing') return 11;      // binary/tempRoot truthiness; spawn throw and child error are tagged above
  }
  return prims.lastEvent()?.stage ?? 0;
}

function describe(e, spec) {
  const facts = prims.causeFacts(e);
  const error = {
    class: 'OtherError', code: null, name: e?.name ?? null, detail: String(e?.message ?? e), stage: prims.STAGE_NAMES[stageOf(e, spec)] ?? 'result',
    rawCode: facts ? facts.code : null, native: facts ? facts.native : 0,
  };
  if (e instanceof tail.EngineImportError) { error.class = 'EngineImportError'; error.code = e.code; error.detail = detailOf(e) || error.detail; }
  else if (e instanceof prims.HarnessError) { error.class = 'HarnessError'; error.stage = 'result'; }
  else if (prims.osError(e)) { error.class = 'RawOsError'; error.rawCode = e.code; error.native = facts.native; }
  return error;
}

function okEnvelope(mode, fields) { return { status: 'ok', mode, ...fields }; }

async function dispatch(spec, signal) {
  const source = { source: 'q', engine: 'opencode', home: spec.sourceHome, kind: 'home' };
  const env = spec.env?.mode === 'full' ? spec.env.values ?? {} : undefined;
  const options = { signal, ...(env ? { env } : {}), opencode: { binary: spec.binary, tempRoot: spec.tempRoot } };
  switch (spec.mode) {
    case 'list':
      return okEnvelope('list', { rows: await opencode.listOpencode(source, spec.folder, options) });
    case 'rows':
      return okEnvelope('rows', { rows: await opencode.readOpencodeRows(spec.sourceHome, env ?? process.env) });
    case 'read': {
      const r = await opencode.readOpencode(source, spec.folder, spec.thread, options);
      return okEnvelope('read', {
        turns: r.turns, omittedBefore: r.omittedBefore, ...(r.title ? { title: r.title } : {}), ...(r.model ? { model: r.model } : {}),
        ...(r.lastPlan ? { lastPlan: r.lastPlan } : {}),
      });
    }
    case 'dataDir':
      return okEnvelope('dataDir', { value: opencode.opencodeDataDir(env ?? process.env) });
    case 'folderKey':
      return okEnvelope('folderKey', { value: opencode.folderKey(spec.folder) });
    case 'plan': {
      const plan = planModule.opencodePlan(JSON.parse(spec.planJson));
      return okEnvelope('plan', plan === undefined ? { present: false } : { present: true, value: plan });
    }
    default:
      throw new prims.HarnessError(`unknown mode ${spec.mode}`);
  }
}

async function runCase(spec) {
  const before = tail.IMPORT_LIMITS.rpcMs;
  prims.begin(spec);
  let out;
  try {
    tail.IMPORT_LIMITS.rpcMs = spec.rpcMs ?? 20000;
    out = await dispatch(spec, new AbortController().signal);
  } catch (e) {
    out = { status: 'error', mode: spec.mode, error: describe(e, spec) };
  } finally {
    tail.IMPORT_LIMITS.rpcMs = before;
    await prims.finish();
  }
  return out;
}

for await (const line of createInterface({ input: process.stdin })) {
  if (!line.trim()) continue;
  const spec = JSON.parse(line);
  let out;
  try { out = await runCase(spec); } catch (e) { out = { status: 'error', mode: spec.mode, error: describe(e, spec) }; }
  process.stdout.write(JSON.stringify(out) + '\n');
}
// exit only after stdout drains: a bare exit cuts a large last result short on a pipe
process.stdout.write('', () => process.exit(0));
