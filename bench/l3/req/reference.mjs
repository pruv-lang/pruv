// Pinned-function oracle of the L3-9 engine-import request handlers (SCENARIO.md). It runs the real pinned engineImportHandlers with the
// real entry/bound/tail code; only the dependency world is scripted (world.mjs). Run as
//   node --experimental-strip-types --no-warnings --import ./reference-hooks.mjs reference.mjs < scenarios.jsonl > observations.jsonl
// One scenario JSON line per stdin line, one observation line per scenario.
import { createInterface } from 'node:readline';
import { engineImportHandlers } from './reference/backend-requests-engine-import.ts';
import { accountsCallback, begin, end } from '../entry/reference-primitives.mjs';
import { bindDefaultAccount } from './fakes/role-policy.ts';
import {
  OracleError, W, abortHandle, advance, als, createHandlers, d, defaultsFor, enc, encThrown, getHandle, installGlobals, makeDeps, quiesce, realGlobals, rec,
  restoreGlobals, settle,
} from './world.mjs';

if (process.version !== 'v26.4.0') {
  process.stderr.write(`reference.mjs needs Node v26.4.0, found ${process.version}\n`);
  process.exit(2);
}

const KINDS = { sources: 'engine-import-sources', list: 'engine-import-list', open: 'engine-import-open' };

function newState(scenario) {
  const world = scenario.world ?? {};
  return {
    world, trace: [], used: new Map(), violations: [], handles: new Map(), sigNames: new WeakMap(), sigSeq: 0, listenerSeq: 0,
    pending: new Map(), labelCount: new Map(), timers: new Map(), timerSeq: 0, now: 0, clockStart: world.clockStart ?? defaultsFor.clockStart,
    sessions: (world.sessions ?? []).map(d), sessionListCalls: 0, createdSeq: 0, createdSessions: [], openingSets: new Map(), openingEvents: [],
    uuidUsed: 0, deps: null,
  };
}

function convertPrimitive(item) { rec('prim', [item[0], ...item.slice(1)]); }

async function runScenario(scenario) {
  const s = newState(scenario);
  W.s = s;
  const state = begin({ id: scenario.id, script: scenario.world ?? {} });
  state.trace = { push: convertPrimitive };
  installGlobals();
  bindDefaultAccount(s.world.defaultAccount ?? 'default');
  const requests = [];
  const instances = new Map();
  try {
    s.deps = makeDeps();
    s.deps.accounts = accountsCallback;
    for (const name of scenario.instances ?? ['A']) instances.set(name, createHandlers(name, engineImportHandlers));
    for (const [index, step] of (scenario.script ?? []).entries()) {
      await quiesce();
      switch (step.ev) {
        case 'start': startRequest(step, instances, requests); break;
        case 'settle': settle(step.label, step.ok !== undefined || step.reject !== undefined ? { ok: step.ok, reject: step.reject } : undefined, undefined); break;
        case 'advance': rec('advance', { ms: step.ms }); await advance(step.ms); break;
        case 'cancel': abortHandle(step.handle, step.reason); break;
        case 'handle': getHandle(step.name); break;
        case 'mutate': mutate(step); break;
        case 'quiesce': break;
        default: throw new OracleError(`unknown step ${index}: ${JSON.stringify(step.ev)}`);
      }
      await quiesce();
    }
  } catch (error) {
    s.violations.push(`driver: ${error.message}`);
  }
  const obs = {
    id: scenario.id,
    requests: requests.map(r => ({ id: r.id, terminal: r.terminal, value: r.value })),
    trace: s.trace,
    createdSessions: s.createdSessions,
    openingEvents: s.openingEvents,
    finalOpening: Object.fromEntries([...instances.keys()].map(name => [name, s.openingSets.has(name) ? [...Set.prototype.values.call(s.openingSets.get(name))].map(enc) : []])),
    activeTimers: [...s.timers.values()].map(t => ({ id: t.id, delay: t.delay, due: t.due })),
    activeListeners: Object.fromEntries([...s.handles].map(([name, h]) => [name, h.active.size])),
    driver: s.violations.length > 0 ? { violations: s.violations } : null,
  };
  restoreGlobals(realGlobals);
  end();
  W.s = null;
  return obs;
}

function startRequest(step, instances, requests) {
  const handlers = instances.get(step.i ?? 'A');
  if (!handlers) throw new OracleError(`unknown instance ${step.i}`);
  const record = { id: step.req, terminal: 'pending', value: null };
  requests.push(record);
  const store = { r: step.req, i: step.i ?? 'A', fromAppAction: step.fromAppAction };
  const done = (terminal, value) => { record.terminal = terminal; record.value = value; rec('request.done', { terminal }); };
  const fail = error => { const value = encThrown(error); done(terminalOf(value), value); };
  als.run(store, () => {
    rec('request.start', { op: step.op, ...(step.input !== undefined ? { input: enc(d(step.input)) } : {}) });
    const handler = handlers[KINDS[step.op]];
    if (!handler) throw new OracleError(`unknown op ${step.op}`);
    let result;
    try {
      if (step.op === 'sources') result = handler();
      else if (step.op === 'list') result = handler(d(step.input));
      else result = handler(d(step.input), step.remote ?? false, step.tempVisible ?? false, step.caller === undefined ? undefined : d(step.caller), step.cancel === undefined ? undefined : getHandle(step.cancel).controller.signal);
    } catch (error) { fail(error); return; }
    Promise.resolve(result).then(value => done('ok', enc(value)), fail);
  });
}

/** Terminal kind of a thrown value (SCENARIO.md 4.1): wire | eie | native | thrown (a scripted value or the cancel reason, identity kept). */
function terminalOf(value) {
  if (value.wire !== undefined) return 'wire';
  if (value.eie !== undefined) return 'eie';
  if (value.native !== undefined || value.domAbort !== undefined) return 'native';
  return 'thrown';
}

function mutate(step) {
  const s = W.s;
  if (step.op === 'add') s.sessions.push(d(step.session));
  else if (step.op === 'remove') s.sessions = s.sessions.filter(x => x.id !== step.id);
  else if (step.op === 'replace') s.sessions = s.sessions.map(x => (x.id === step.id ? d(step.session) : x));
  else throw new OracleError(`unknown mutate ${step.op}`);
  rec('mutate', { op: step.op, id: step.id ?? null });
}

const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
for await (const line of lines) {
  if (line.trim() === '') continue;
  let scenario;
  try { scenario = JSON.parse(line); } catch (error) {
    process.stdout.write(JSON.stringify({ id: null, requests: [], trace: [], createdSessions: [], openingEvents: [], finalOpening: {}, activeTimers: [], activeListeners: {}, driver: { violations: [`bad scenario line: ${error.message}`] } }) + '\n');
    continue;
  }
  process.stdout.write(JSON.stringify(await runScenario(scenario)) + '\n');
}
