// reads JSON cases from stdin: [{n, table:[...], mod}] ; comparator result = table[(a*mod+b) % table.length]
import { readFileSync } from 'node:fs';
const cases = JSON.parse(readFileSync(0, 'utf8'));
const out = cases.map(c => {
  const calls = [];
  const items = Array.from({ length: c.n }, (_, i) => ({ i, d: c.dates[i] }));
  let cmp;
  if (c.kind === 'grok') cmp = (a, b) => Date.parse(b.d ?? '') - Date.parse(a.d ?? '') || 0;
  else cmp = (a, b) => c.table[(a.i * 7919 + b.i * 104729 + c.salt) % c.table.length];
  items.sort((a, b) => { calls.push(a.i, b.i); return cmp(a, b); });
  return { order: items.map(x => x.i), calls };
});
process.stdout.write(JSON.stringify(out));
