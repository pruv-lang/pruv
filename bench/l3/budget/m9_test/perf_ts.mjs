// TS side of the M9 perf items (same shapes as driver_m9.rs run_perf / hand_m9.rs), on the pinned reference class.
// Real node timers (setTimeout / clearTimeout), as the TS code uses them; the limit is large so no timer elapses.
//   node --experimental-strip-types --no-warnings --import ../reference-hooks.mjs perf_ts.mjs <R> <reps>
import * as B from '../reference/backend-request-budget.ts';

const R = Number(process.argv[2]);
const REPS = Number(process.argv[3]);
let now = 1700000000000;

function world() {
  const ref = { sessionId: 'session-perf', budgetId: '99b568f7-0000-4000-8000-000000000001' };
  const session = {
    id: ref.sessionId,
    budget: {
      id: ref.budgetId, openedAt: '2023-11-14T22:13:20.000Z', origin: 'direct', state: 'open',
      limits: { timeMinutes: 1000000, delegations: 5, commands: 2, reviews: 1, checks: 1 },
      used: { activeMs: 0, delegations: 0, commands: 0, reviews: 0, checks: 0, tools: 0, failures: 0, retries: 0 },
    },
  };
  const noop = () => {};
  const deps = new Proxy({
    now: () => now,
    findSession: (id) => (id === session.id ? session : undefined),
    runningDelegationIds: () => [],
    isQuiet: () => true,
    releaseHeld: () => false,
  }, { get: (t, k) => (k in t ? t[k] : noop) });
  return { b: new B.RequestBudgets(deps), ref };
}

function time(fn, calls) {
  const t = process.hrtime.bigint();
  fn();
  return Number(process.hrtime.bigint() - t) / calls;
}

{
  const { b, ref } = world();
  const keys = Array.from({ length: R }, (_, i) => 'run-' + String(i).padStart(4, '0'));
  for (const k of keys) b.runStarted(ref, k);
  for (const k of keys.slice(0, R - 1)) b.runWaiting(ref, k, true);
  const last = keys[R - 1];
  for (let i = 0; i < 20000; i++) { b.runWaiting(ref, last, true); b.runWaiting(ref, last, false); }
  const ns = time(() => { for (let i = 0; i < REPS; i++) { b.runWaiting(ref, last, true); b.runWaiting(ref, last, false); } }, 2 * REPS);
  console.log(`refresh ${R} ${ns.toFixed(1)}`);
}
{
  const { b, ref } = world();
  for (let i = 0; i < 20000; i++) { b.runStarted(ref, 'run-0000'); b.runEnded(ref, 'run-0000'); }
  const ns = time(() => { for (let i = 0; i < REPS; i++) { b.runStarted(ref, 'run-0000'); b.runEnded(ref, 'run-0000'); } }, 2 * REPS);
  console.log(`cycle 1 ${ns.toFixed(1)}`);
}
{
  const { b, ref } = world();
  b.runStarted(ref, 'run-0000');
  const fire = b.fire.bind(b);
  const clocks = b.clocks;
  // host side of a delivery: the elapsed timer leaves the scheduler, then the callback runs
  const ns = time(() => { for (let i = 0; i < REPS; i++) { now += 1; clearTimeout(clocks.get(ref.budgetId).timer); fire(ref); } }, REPS);
  console.log(`fire 1 ${ns.toFixed(1)}`);
}
process.exit(0);
