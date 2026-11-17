// L3-10 M5 perf, TS side: the same mechanics measured on the oracle world (oracle-world.mjs, tracing off).
//   node m5_test/perf_ts.mjs <n,n,...> <reps>     -> one JSON line per (measure, n): {measure, n, ns}
// ns = per-op time, min over reps after one warm-up.
// timer_cycle: n x (setTimeout then clearTimeout); timer_fire: n setTimeout with distinct delays (setup), then one timed
// advance firing all (empty callbacks); store_find: n stored sessions, n findSession calls on distinct ids.
import { W, advance, createState, initWorld, installGlobals, makeDeps, restoreGlobals } from '../oracle-world.mjs';

const sizes = (process.argv[2] ?? '100,500,1000').split(',').map(Number);
const reps = Number(process.argv[3] ?? 5);

function fresh(sessions) {
  const s = createState({ world: { sessions } }, false);
  W.s = s;
  initWorld(s);
  return s;
}

function timeIt(setup, body, n) {
  let best = Infinity;
  for (let r = 0; r <= reps; r++) {
    const ctx = setup();
    const t0 = process.hrtime.bigint();
    body(ctx);
    const ns = Number(process.hrtime.bigint() - t0) / n;
    if (r > 0 && ns < best) best = ns;
  }
  return best;
}

const sessionsOf = n => Array.from({ length: n }, (_, k) => ({ id: `s${k}`, budget: { id: `b${k}`, state: 'open' } }));
const noop = () => {};
let sink = 0;

installGlobals();
try {
  for (const n of sizes) {
    const cycle = timeIt(() => fresh([]), () => {
      for (let k = 0; k < n; k++) { const t = setTimeout(noop, 1000 + k); clearTimeout(t); }
    }, n);
    // As on the host, the timers are registered in setup and only the advance is timed.
    const fire = timeIt(() => { const s = fresh([]); for (let k = 0; k < n; k++) setTimeout(noop, 1 + k); return s; }, () => {
      advance(n + 1);
    }, n);
    const find = timeIt(() => ({ s: fresh(sessionsOf(n)), deps: makeDeps() }), ({ deps }) => {
      for (let k = 0; k < n; k++) { const x = deps.findSession(`s${(k * 7) % n}`); if (x !== undefined) sink += x.id.length; }
    }, n);
    for (const [measure, ns] of [['timer_cycle', cycle], ['timer_fire', fire], ['store_find', find]]) {
      process.stdout.write(JSON.stringify({ measure, n, ns: Math.round(ns * 10) / 10 }) + '\n');
    }
  }
} finally {
  restoreGlobals();
}
if (sink < 0) process.stdout.write('');
