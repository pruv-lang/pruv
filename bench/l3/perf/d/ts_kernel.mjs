// Leg D in JS (node v26): the same kernels as kernel.pruv / hand.rs over plain number arrays (int, str), plus `obj`: the
// real handler shape -- perf_req.py session objects walked with the pinned sessionThreads generator and excludedFromBound
// (bench/l3/entry/reference/backend-engine-import-bound.ts, transcribed) and string ===.
//   node ts_kernel.mjs <int|str|obj> <datafile(.str.txt for obj)> <reps>
import { readFileSync } from 'node:fs';

const K = 8;

function boundInt(sess, k, items) {
  const w = 1 + 2 * k, out = [];
  for (let j = 0; j + 1 < items.length; j += 2) {
    const e = items[j], t = items[j + 1];
    let found = -1;
    for (let b = 0, s = 0; b + w <= sess.length && found < 0; b += w, s++) {
      if (sess[b] !== 0) continue;
      for (let c = 0; c < k; c++) if (sess[b + 1 + 2 * c] === e && sess[b + 2 + 2 * c] === t) { found = s; break; }
    }
    out.push(found);
  }
  return out;
}

function same(pool, a, b, l) {
  if (a < 0 || b < 0 || l < 0 || a + l > pool.length || b + l > pool.length) return false;
  for (let u = 0; u < l; u++) if (pool[a + u] !== pool[b + u]) return false;
  return true;
}

function boundStr(sess, k, items, pool) {
  const w = 1 + 3 * k, out = [];
  for (let j = 0; j + 2 < items.length; j += 3) {
    const e = items[j], io = items[j + 1], il = items[j + 2];
    let found = -1;
    for (let b = 0, s = 0; b + w <= sess.length && found < 0; b += w, s++) {
      if (sess[b] !== 0) continue;
      for (let c = 0; c < k; c++) {
        const p = b + 1 + 3 * c;
        if (sess[p] === e && sess[p + 2] === il && same(pool, sess[p + 1], io, il)) { found = s; break; }
      }
    }
    out.push(found);
  }
  return out;
}

// --- obj: the real shapes ---
function* sessionThreads(session) {
  const origin = session.externalThread;
  if (origin) {
    yield { engine: origin.engine, thread: origin.thread };
    for (const fork of origin.forks ?? []) yield { engine: origin.engine, thread: fork };
  }
  const cli = session.terminal?.cliSession;
  if (cli) yield { engine: session.engine ?? 'codex', thread: cli };
  for (const [slot, bound] of Object.entries(session.engineThreads ?? {})) {
    if (bound?.threadId) yield { engine: slot.split(':', 1)[0], thread: bound.threadId };
  }
}
const excludedFromBound = (session, isTemporary) => session.temporary === true || isTemporary(session.id);

function boundObj(sessions, isTemporary, items) {
  return items.map(({ engine, thread }) => {
    for (const session of sessions) {
      if (excludedFromBound(session, isTemporary)) continue;
      for (const bound of sessionThreads(session)) if (bound.engine === engine && bound.thread === thread) return session.id;
    }
    return undefined;
  });
}

function objData(S, M) {
  const sessions = [];
  for (let i = 0; i < S; i++) sessions.push({ id: `s${i}`, title: `T${i}`,
    externalThread: { engine: 'claude', source: 'claude:home', thread: `o${i}`, mode: 'fork', importedAt: '2023-11-14T22:13:20.000Z', forks: [0, 1, 2].map(k => `f${i}_${k}`) },
    terminal: { cliSession: `c${i}` },
    engineThreads: { 'claude:w1': { threadId: `e${i}_1` }, 'claude:w2': { threadId: `e${i}_2` }, 'claude:w3': { threadId: `e${i}_3` } } });
  const table = {};  // world.mjs isTemporary: table lookup, false when absent
  const isTemporary = id => (typeof id === 'string' && Object.hasOwn(table, id) ? table[id] : false);
  const items = [];
  for (let j = 0; j < M; j++) items.push({ engine: 'claude', thread: `t${j}` });
  return { sessions, isTemporary, items };
}

const [variant, file, repsArg] = process.argv.slice(2);
const reps = Number(repsArg);
const rows = readFileSync(file, 'utf8').split('\n').filter(l => l.length).map(l => l.split(' ').map(Number));
let call;
if (variant === 'int') call = () => boundInt(rows[0], K, rows[1]);
else if (variant === 'str') call = () => boundStr(rows[0], K, rows[1], rows[2]);
else if (variant === 'obj') {
  const S = rows[0].length / (1 + 3 * K), M = rows[1].length / 3;
  const { sessions, isTemporary, items } = objData(S, M);
  call = () => boundObj(sessions, isTemporary, items);
} else throw new Error('variant');
for (let i = 0; i < 3; i++) call();
const times = [];
let checksum = 0;
for (let i = 0; i < reps; i++) {
  const t0 = process.hrtime.bigint();
  const out = call();
  times.push(Number(process.hrtime.bigint() - t0) / 1e6);
  checksum = out.reduce((a, x) => a + (typeof x === 'number' ? x : -1), 0);
}
times.sort((a, b) => a - b);
console.log(JSON.stringify({ variant, impl: 'ts', best_ms: +times[0].toFixed(3), median_ms: +times[times.length >> 1].toFixed(3), checksum }));
