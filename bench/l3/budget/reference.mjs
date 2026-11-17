// Pinned-class oracle of the L3-10 request budget (docs/REQUEST_BUDGET_SPEC.md section 7). It runs the real pinned RequestBudgets class and
// helpers (reference/backend-request-budget.ts, transpiled by reference-hooks.mjs); only the dependency world is scripted (oracle-world.mjs).
// Run as
//   GROK_TS_TRANSPILER=/path/to/typescript node --experimental-strip-types --no-warnings --import ./reference-hooks.mjs reference.mjs [--time [--repeat N]] < scenarios.jsonl > observations.jsonl
// One scenario JSON line per stdin line; one observation line per scenario (default), or one timing line per scenario (--time, default 5 repeats).
import { createInterface } from 'node:readline';
import * as B from './reference/backend-request-budget.ts';
import {
  OracleError, W, activeTimers, advance, clocksOf, createState, d, deliverTimer, enc, encReturn, encThrown, initWorld, installGlobals, makeDeps, putSession,
  registerSession, removeSession, restoreGlobals, setDepReply, setTime, setUnit, snapshot, toolSeenOf, trace,
} from './oracle-world.mjs';

if (process.version !== 'v26.4.0') {
  process.stderr.write(`reference.mjs needs Node v26.4.0, found ${process.version}\n`);
  process.exit(2);
}

const argv = process.argv.slice(2);
const TIME_MODE = argv.includes('--time');
const repeatAt = argv.indexOf('--repeat');
const REPEAT = repeatAt >= 0 ? Number(argv[repeatAt + 1]) : 5;
if (!Number.isInteger(REPEAT) || REPEAT < 1) {
  process.stderr.write('--repeat needs a positive integer\n');
  process.exit(2);
}

const METHODS = new Set([
  'current', 'isHalted', 'attach', 'replaceForUserMessage', 'reopen', 'charge', 'noteFailure', 'noteTool', 'forgetTurn', 'runStarted', 'runWaiting',
  'runEnded', 'halt', 'maybeClose', 'raise', 'decide', 'setCard', 'hasCard', 'clearAutoContinue', 'recordAutoContinue', 'stopAutoContinue', 'bootRestore',
  'forgetSession', 'dispose',
]);
const HELPERS = new Set(['limitText', 'refusalWire', 'refusal', 'continueText', 'cardMissing', 'budgetUnitOf']);
// Calls whose first argument is a session: registered so that it is encoded as an {sref, v} snapshot even when passed as a literal.
const SESSION_FIRST = new Set(['attach', 'replaceForUserMessage', 'maybeClose', 'bootRestore']);
const isObj = v => v !== null && typeof v === 'object' && !Array.isArray(v);

function callStep(s, step, instances, requests) {
  const op = step.op;
  const helper = HELPERS.has(op);
  if (!helper && !METHODS.has(op)) throw new OracleError(`unknown op ${JSON.stringify(op)}`);
  const name = step.i ?? 'A';
  const instance = instances.get(name);
  if (!helper && instance === undefined) throw new OracleError(`unknown instance ${JSON.stringify(name)}`);
  if (step.args !== undefined && !Array.isArray(step.args)) throw new OracleError('call args must be an array');
  if (step.now !== undefined && !Array.isArray(step.now)) throw new OracleError('call now must be an array');
  const args = (step.args ?? []).map(x => d(x));
  const nowList = (step.now ?? []).map(x => d(x));
  if (SESSION_FIRST.has(op)) registerSession(args[0]);

  const record = { id: step.req, instance: name, op, terminal: 'pending', value: null };
  requests.push(record);
  s.cur = { r: step.req, i: name };
  s.nowOverride = nowList;
  trace('call.start', () => enc({ op, args }));

  const fn = helper ? B[op] : instance[op];
  let terminal;
  let result;
  let error;
  const t0 = process.hrtime.bigint();
  try { result = fn.apply(helper ? undefined : instance, args); terminal = 'ok'; } catch (e) { error = e; terminal = 'thrown'; }
  const ns = process.hrtime.bigint() - t0;

  s.calls += 1;
  s.coreNs += ns;
  const per = s.perOp[op] ?? (s.perOp[op] = { n: 0, ns: 0n });
  per.n += 1;
  per.ns += ns;
  if (s.tracing) record.value = terminal === 'ok' ? encReturn(op, result) : encThrown(error);
  record.terminal = terminal;
  trace('call.end', () => enc({ terminal }));
  s.cur = { r: null, i: null };
  s.nowOverride = [];
  if (error instanceof OracleError) throw error; // a driver failure inside the call (e.g. UUID stream exhausted) ends the scenario
}

function runStep(s, step, instances, requests) {
  if (!isObj(step)) throw new OracleError('script step is not an object');
  switch (step.ev) {
    case 'call': callStep(s, step, instances, requests); break;
    case 'advance': {
      const ms = d(step.ms);
      trace('advance', () => enc({ ms }));
      advance(ms);
      break;
    }
    case 'setTime': setTime(d(step.now)); break;
    case 'deliverTimer': deliverTimer(step.token, step.captured); break;
    case 'setUnit': setUnit(d(step.unit)); break;
    case 'putSession': putSession(d(step.session)); break;
    case 'removeSession': removeSession(step.id); break;
    case 'setDepReply': setDepReply(step.dep, Array.isArray(step.replies) ? step.replies.map(x => d(x)) : step.replies); break;
    case 'snapshot': snapshot(step.label); break;
    default: throw new OracleError(`unknown step ev ${JSON.stringify(step.ev)}`);
  }
}

/** Run one scenario against a fresh world. Globals are restored before returning; W.s stays set for the caller (cleared by it). */
function execute(scenario, tracing) {
  const s = createState(scenario, tracing);
  W.s = s;
  const requests = [];
  const instances = new Map();
  installGlobals();
  try {
    initWorld(s);
    for (const name of scenario.instances ?? ['A']) instances.set(name, new B.RequestBudgets(makeDeps()));
    s.instances = instances;
    for (const step of scenario.script ?? []) runStep(s, step, instances, requests);
  } catch (error) {
    s.violations.push(`driver: ${error instanceof Error ? error.message : String(error)}`);
  } finally {
    restoreGlobals();
  }
  return { s, requests, instances };
}

function observe(scenario) {
  const { s, requests, instances } = execute(scenario, true);
  const obs = {
    id: scenario.id ?? null, requests, trace: s.trace, finalSessions: [], finalClocks: {}, finalToolSeen: {}, activeTimers: [], driver: null, excluded: s.excluded,
  };
  try {
    obs.finalSessions = s.sessions.map(x => enc(x));
    obs.finalClocks = Object.fromEntries([...instances].map(([name, inst]) => [name, enc(clocksOf(inst))]));
    obs.finalToolSeen = Object.fromEntries([...instances].map(([name, inst]) => [name, enc(toolSeenOf(inst))]));
    obs.activeTimers = activeTimers(s).map(t => ({ token: t.token, delay: enc(t.delay), due: enc(t.due), i: t.i }));
  } catch (error) {
    s.violations.push(`driver: final state: ${error.message}`);
  }
  obs.driver = s.violations.length > 0 ? { violations: s.violations } : null;
  if (scenario.domain !== undefined) obs.domain = scenario.domain;
  W.s = null;
  return obs;
}

function timeScenario(scenario) {
  let best = null;
  let fireNs = null;
  let violations = [];
  for (let n = 0; n < REPEAT; n++) {
    const { s } = execute(scenario, false);
    violations = s.violations;
    if (best === null || s.coreNs < best.coreNs) best = s;
    if (fireNs === null || s.fireNs < fireNs) fireNs = s.fireNs;
    W.s = null;
  }
  return {
    id: scenario.id ?? null,
    calls: best.calls,
    coreNs: Number(best.coreNs),
    fireNs: Number(fireNs),
    perOp: Object.fromEntries(Object.entries(best.perOp).map(([op, v]) => [op, { n: v.n, ns: Number(v.ns) }])),
    depCalls: Object.fromEntries(Object.entries(best.depCalls)),
    rssBytes: process.memoryUsage().rss,
    driver: violations.length > 0 ? { violations } : null,
  };
}

function badLine(message) {
  if (TIME_MODE) return { id: null, calls: 0, coreNs: 0, fireNs: 0, perOp: {}, depCalls: {}, rssBytes: process.memoryUsage().rss, driver: { violations: [message] } };
  return { id: null, requests: [], trace: [], finalSessions: [], finalClocks: {}, finalToolSeen: {}, activeTimers: [], driver: { violations: [message] }, excluded: [] };
}

const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
for await (const line of lines) {
  if (line.trim() === '') continue;
  let scenario;
  try { scenario = JSON.parse(line); } catch (error) { process.stdout.write(JSON.stringify(badLine(`bad scenario line: ${error.message}`)) + '\n'); continue; }
  if (!isObj(scenario)) { process.stdout.write(JSON.stringify(badLine('bad scenario line: not an object')) + '\n'); continue; }
  const out = TIME_MODE ? timeScenario(scenario) : observe(scenario);
  process.stdout.write(JSON.stringify(out) + '\n');
}
