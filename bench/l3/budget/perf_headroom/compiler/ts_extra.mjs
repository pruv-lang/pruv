// TS side of the two items m6_test/node_ref.mjs has no perf loop for. JSONL in ({name, n, args, reps}), JSONL out ({ns, sum}).
// p_amt: `${used}/${limit}` as the reference helper builds the amount; p_wr: copy 64 units into a 64-unit array.
// Same loop, input alternation and consumption as node_ref.mjs perf (1 warm-up, min over reps).
import { createInterface } from 'node:readline';

const eat = t => t.length + ((t.charCodeAt(0) + t.charCodeAt(t.length >> 1) + t.charCodeAt(t.length - 1)) & 1);

function perf(r) {
  const n = r.n;
  const a = r.args.map(Number);
  let body;
  if (r.name === 'p_amt') {
    const [used, limit] = a;
    body = () => { let s = 0; for (let i = 0; i < n; i++) s += eat(`${used + (i & 1)}/${limit}`); return s; };
  } else if (r.name === 'p_wr') {
    const x = a[0];
    body = () => {
      const src = [];
      for (let k = 0; k < 64; k++) src.push((x + k) & 65535);
      const o = new Array(64).fill(0);
      let s = 0;
      for (let i = 0; i < n; i++) {
        for (let k = 0; k < src.length; k++) o[k] = src[k];
        s += (o[(i * 7) % 64] & 1) + 1;
        src[i % 64] = (src[i % 64] + 1) & 65535;
      }
      return s;
    };
  } else throw new Error(`unknown perf op ${r.name}`);
  body();
  let best = Infinity, sum = 0;
  for (let k = 0; k < (r.reps ?? 3); k++) {
    const t0 = performance.now();
    sum = body();
    best = Math.min(best, performance.now() - t0);
  }
  return { ns: Math.round(best * 1e6), sum };
}

const rl = createInterface({ input: process.stdin, crlfDelay: Infinity });
for await (const line of rl) {
  if (!line.trim()) continue;
  let out;
  try { out = perf(JSON.parse(line)); } catch (e) { out = { err: String(e && e.message || e) }; }
  process.stdout.write(JSON.stringify(out) + '\n');
}
