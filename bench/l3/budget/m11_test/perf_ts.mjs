// M11 perf baseline: the REAL pinned RequestBudgets.setCard (reference/backend-request-budget.ts) with fake deps
// (sessions in a Map, now = a counter, save/changed no-ops).
//   node --experimental-strip-types --no-warnings --import <budget>/reference-hooks.mjs perf_ts.mjs A <U> <reps>
//   ... perf_ts.mjs B <n> <reps>
// Prints `ns <ns per call> <checksum>`. A = setCard of one card (deliverable + 5 done lines of U units padded by 2
// spaces each side) on a fresh budget each rep; B = n successive setCard calls on one budget, reps rounds.
import * as B from '../reference/backend-request-budget.ts';

const NOW0 = 1700000000000;
const [mode, aStr, repsStr] = process.argv.slice(2);
const a = Number(aStr), reps = Number(repsStr);

function world() {
  const sessions = new Map();
  sessions.set('s1', { id: 's1', budget: { id: 'b1', state: 'running', turnCount: 3 } });
  let t = NOW0;
  const deps = {
    findSession: id => sessions.get(id),
    now: () => t++,
    save: () => {},
    changed: () => {},
  };
  return { rb: new B.RequestBudgets(deps), sessions };
}

const line = '  ' + 'a'.repeat(mode === 'A' ? a : 20) + '  ';
const card = () => ({ deliverable: line, done: [line, line, line, line, line] });
const ref = { sessionId: 's1', budgetId: 'b1' };
let sum = 0;
let ns = 0;
if (mode === 'A') {
  const { rb } = world();
  const c = card();
  const t0 = process.hrtime.bigint();
  for (let i = 0; i < reps; i++) {
    const r = rb.setCard(ref, c);
    sum += r.ok ? r.budget.card.done[4].length : r.error.length;
  }
  ns = Number(process.hrtime.bigint() - t0) / reps;
} else {
  let total = 0n;
  const c = card();
  for (let k = 0; k < reps; k++) {
    const { rb } = world();
    const t0 = process.hrtime.bigint();
    for (let i = 0; i < a; i++) {
      const r = rb.setCard(ref, c);
      sum += r.ok ? 1 : 0;
    }
    total += process.hrtime.bigint() - t0;
  }
  ns = Number(total) / (a * reps);
}
console.log(`ns ${ns.toFixed(1)} ${sum}`);
