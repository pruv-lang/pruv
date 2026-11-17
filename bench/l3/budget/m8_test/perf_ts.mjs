// TS side of the M8 perf items (same shapes as driver_m8.rs run_perf / hand_m8.rs), on the pinned reference class.
//   node --experimental-strip-types --no-warnings --import ../reference-hooks.mjs perf_ts.mjs <N> <reps>
import * as B from '../reference/backend-request-budget.ts';

const N = Number(process.argv[2]);
const REPS = Number(process.argv[3]);
const pad = (i, w) => String(i).padStart(w, '0');
const bidOf = (i) => '99b568f7-0000-4000-8000-' + pad(i, 12);

const sessions = new Map();
for (let i = 0; i < N; i++) {
  const id = 'session-' + pad(i, 5);
  sessions.set(id, {
    id,
    budget: {
      id: bidOf(i), openedAt: '2023-11-14T22:13:20.000Z', origin: 'direct', state: 'open',
      limits: { timeMinutes: 1000000, delegations: 1e12, commands: 1e12, reviews: 1e12, checks: 1e12 },
      used: { activeMs: 0, delegations: 0, commands: 0, reviews: 0, checks: 0, tools: 0, failures: 0, retries: 0 },
    },
  });
}
const noop = () => {};
const deps = new Proxy({
  now: () => 1700000000000,
  findSession: (id) => sessions.get(id),
  runningDelegationIds: () => [],
  isQuiet: () => true,
  releaseHeld: () => false,
}, { get: (t, k) => (k in t ? t[k] : noop) });
const b = new B.RequestBudgets(deps);
const refs = Array.from({ length: N }, (_, i) => ({ sessionId: 'session-' + pad(i, 5), budgetId: bidOf(i) }));
const what = { commands: 1, retries: 1 };

function time(fn, calls) {
  const t = process.hrtime.bigint();
  fn();
  return Number(process.hrtime.bigint() - t) / calls;
}

for (let i = 0; i < 20000; i++) b.charge(refs[i % N], what);
let ns = time(() => { for (let i = 0; i < REPS; i++) b.charge(refs[i % N], what); }, REPS);
console.log(`charge ${N} ${ns.toFixed(1)}`);
ns = time(() => { for (let i = 0; i < REPS; i++) b.noteFailure(refs[i % N]); }, REPS);
console.log(`failure ${N} ${ns.toFixed(1)}`);
const acts = Array.from({ length: N }, (_, j) => 'activity-' + pad(j, 5));
for (const a of acts) b.noteTool(refs[0], 'turn-1', a);
ns = time(() => { for (let i = 0; i < REPS; i++) b.noteTool(refs[0], 'turn-1', acts[i % N]); }, REPS);
console.log(`tooldup ${N} ${ns.toFixed(1)}`);
for (const a of acts) b.noteTool(refs[0], 'turn-2', a);
b.forgetTurn('turn-2');
const rounds = Math.max(1, Math.floor(REPS / N));
ns = time(() => {
  for (let r = 0; r < rounds; r++) { for (const a of acts) b.noteTool(refs[0], 'turn-2', a); b.forgetTurn('turn-2'); }
}, rounds * N);
console.log(`toolgrow ${N} ${ns.toFixed(1)}`);
process.exit(0);
