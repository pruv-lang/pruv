// TS side of the M7 perf items (same shapes as driver_m7.rs run_perf / hand_m7.rs), on the pinned reference class.
// The hooks route node:crypto to the oracle's UUID stream (fakes/node-crypto.ts); it is given an untraced state here, so
// randomUUID is a counter (cheaper than node's real one). The host deps are plain functions.
//   node --experimental-strip-types --no-warnings --import ../reference-hooks.mjs perf_ts.mjs <S> <reps>
import * as B from '../reference/backend-request-budget.ts';
import { W, createState } from '../oracle-world.mjs';

W.s = createState({}, false);

const S = Number(process.argv[2]);
const REPS = Number(process.argv[3]);
const now = 1700000000000;
const CARD = { deliverable: 'd', done: ['x'], state: 'done', updatedAt: '2023-11-14T22:13:20.000Z' };
const bid = (i) => i.toString(16).padStart(8, '0') + '-0000-4000-8000-000000000001';

function rec(i, state) {
  return {
    id: bid(i), openedAt: '2023-11-14T22:13:20.000Z', origin: 'user', state,
    limits: { timeMinutes: 120, delegations: 24, commands: 3, reviews: 2, checks: 1 },
    used: { activeMs: 0, delegations: 0, commands: 0, reviews: 0, checks: 0, tools: 0, failures: 0, retries: 0 },
    card: { ...CARD },
  };
}

function world(state) {
  const sessions = new Map();
  for (let i = 0; i < S; i++) {
    const id = 's' + String(i).padStart(4, '0');
    sessions.set(id, { id, budget: rec(i, state) });
  }
  const noop = () => {};
  const deps = new Proxy({
    now: () => now,
    findSession: (id) => sessions.get(id),
    unit: () => ({ timeMinutes: 120, delegations: 24, commands: 3, reviews: 2, checks: 1 }),
    runningDelegationIds: () => [],
    isQuiet: () => true,
    releaseHeld: () => false,
  }, { get: (t, k) => (k in t ? t[k] : noop) });
  const last = sessions.get('s' + String(S - 1).padStart(4, '0'));
  return { b: new B.RequestBudgets(deps), last, ref: { sessionId: last.id, budgetId: bid(S - 1) } };
}

function time(fn, calls) {
  for (let i = 0; i < 1000; i++) fn(1);
  const t = process.hrtime.bigint();
  fn(REPS);
  return Number(process.hrtime.bigint() - t) / calls;
}

let sink = 0;
{
  const { b, ref } = world('open');
  const ns = time((n) => { for (let i = 0; i < n; i++) if (b.current(ref)) sink++; }, REPS);
  console.log(`current ${S} ${ns.toFixed(1)}`);
}
{
  const { b, ref } = world('open');
  const ns = time((n) => { for (let i = 0; i < n; i++) if (b.isHalted(ref)) sink++; }, REPS);
  console.log(`ishalted ${S} ${ns.toFixed(1)}`);
}
{
  const { b, last } = world('open');
  const ns = time((n) => { for (let i = 0; i < n; i++) sink += b.attach(last, 'user').budgetId.length; }, REPS);
  console.log(`attach ${S} ${ns.toFixed(1)}`);
}
{
  const { b, last, ref } = world('open');
  const ns = time((n) => { for (let i = 0; i < n; i++) { if (b.maybeClose(last)) sink++; if (b.reopen(ref)) sink++; } }, 2 * REPS);
  console.log(`closereopen ${S} ${ns.toFixed(1)}`);
}
{
  const { b, last } = world('halted');
  const ns = time((n) => {
    for (let i = 0; i < n; i++) {
      sink += b.replaceForUserMessage(last, 2, 'user').budgetId.length;
      last.budget = rec(S - 1, 'halted');
    }
  }, REPS);
  console.log(`replace ${S} ${ns.toFixed(1)}`);
}
if (sink < 0) console.log(sink);
process.exit(0);
