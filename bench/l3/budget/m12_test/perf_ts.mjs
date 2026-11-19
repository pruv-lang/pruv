// Pinned TypeScript with resident JavaScript objects and local fake dependencies.
import { RequestBudgets } from '../reference/backend-request-budget.ts';

const [item, sizeArg, repsArg] = process.argv.slice(2);
const size = Number(sizeArg);
const reps = Number(repsArg);
const text = 'x'.repeat(size);
const record = { sessionId: 's1', userTurnId: text, count: 99, stopped: false };
const ref = { sessionId: 's1', budgetId: 'b1' };
const card = { state: 'working', by: { sessionId: 's1' }, deliverable: item === 'R' ? '' : text };
let saves = 0;
const worlds = Array.from({ length: Math.min(64, reps) }, () => {
  const session = { id: 's1', budget: { id: 'b1', state: 'open', card } };
  const rb = new RequestBudgets({
    findSession: id => id === session.id ? session : undefined,
    save: () => { saves++; },
    changed: () => {},
  });
  return { session, rb };
});
let ns = 0n;
let sum = 0;
for (let i = 0; i < reps; i += worlds.length) {
  const n = Math.min(worlds.length, reps - i);
  for (let j = 0; j < n; j++) {
    const b = worlds[j].session.budget;
    if (item === 'T') b.autoContinue = { sessionId: 's1', stopped: false };
    else delete b.autoContinue;
  }
  const start = process.hrtime.bigint();
  if (item === 'R') {
    for (let j = 0; j < n; j++) sum += Number(worlds[j].rb.recordAutoContinue(ref, record));
  } else {
    for (let j = 0; j < n; j++) worlds[j].rb.stopAutoContinue('s1');
  }
  ns += process.hrtime.bigint() - start;
}
console.log(`ns ${(Number(ns) / reps).toFixed(1)} ${sum + saves}`);
