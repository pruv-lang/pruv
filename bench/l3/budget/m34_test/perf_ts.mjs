// In-process JS Map/Set baseline for the M4 perf axes (same operation sequence as driver_m4 perf).
//   node perf_ts.mjs <axis> <n> <reps>      axis = sessions | tools | clocks
// Prints one line: axis=.. n=.. ts_ns_per_op=..
// Prefill and key-string construction are outside the timed loop, like the Pruv driver's prefill and encode phases.

const [, , axis, nArg, repsArg] = process.argv;
const n = parseInt(nArg, 10);
const reps = parseInt(repsArg, 10);
if (!axis || !(n > 0) || !(reps > 0)) {
  console.error("usage: node perf_ts.mjs <axis> <n> <reps>");
  process.exit(2);
}

let sink = 0;

function runOnce(count) {
  if (axis === "sessions") {
    // Map<string, object>; per rep: get + put (replace) on one fixed session.
    const m = new Map();
    for (let i = 0; i < n; i++) m.set("s" + i, { w: [11, 22, 33, 44, 55, 66, 77, 88] });
    const key = "s" + (n >> 1);
    const t0 = process.hrtime.bigint();
    for (let r = 0; r < count; r++) {
      const v = m.get(key);
      sink += v === undefined ? 0 : v.w.length;
      m.set(key, { w: [11, 22, 33, 44, 55, 66, 77, 88] });
    }
    return { ns: process.hrtime.bigint() - t0, ops: count * 2 };
  }
  if (axis === "tools") {
    // Map<turnId, Set<activityId>>; per rep: note a seen activity, note a new one in a fresh turn, forget that turn.
    const tm = new Map();
    const noteTool = (turn, act) => {
      let s = tm.get(turn);
      if (s === undefined) {
        s = new Set();
        tm.set(turn, s);
      }
      if (s.has(act)) return 0;
      s.add(act);
      return 1;
    };
    const acts = [];
    for (let i = 0; i < n; i++) acts.push("a" + i);
    for (let i = 0; i < n; i++) noteTool("t0", acts[i]);
    const t0 = process.hrtime.bigint();
    for (let r = 0; r < count; r++) {
      sink += noteTool("t0", acts[r % n]);
      sink += noteTool("x", "y");
      sink += tm.delete("x") ? 1 : 0;
    }
    return { ns: process.hrtime.bigint() - t0, ops: count * 3 };
  }
  if (axis === "clocks") {
    // Map<budgetId, {runs: Map}>; per rep: run_set + run_del on one clock.
    const cm = new Map();
    for (let i = 0; i < n; i++) cm.set("c" + i, { sid: "s0", since: 0, armed: 0, deadline: 0, runs: new Map() });
    const c = "c" + (n >> 1);
    const rks = ["r0", "r1", "r2", "r3"];
    const t0 = process.hrtime.bigint();
    for (let r = 0; r < count; r++) {
      const rk = rks[r % 4];
      const cl = cm.get(c);
      if (cl === undefined) {
        sink += 2;
      } else {
        sink += cl.runs.has(rk) ? 0 : 1;
        cl.runs.set(rk, 1);
      }
      const cl2 = cm.get(c);
      sink += cl2 !== undefined && cl2.runs.delete(rk) ? 1 : 0;
    }
    return { ns: process.hrtime.bigint() - t0, ops: count * 2 };
  }
  console.error("unknown axis " + axis);
  process.exit(2);
}

// Warm-up on a separate state so the JIT has compiled the loops, then the measured run.
runOnce(Math.min(reps, 5000));
runOnce(Math.min(reps, 5000));
const res = runOnce(reps);
console.error("sink=" + sink);
console.log(`axis=${axis} n=${n} ts_ns_per_op=${(Number(res.ns) / res.ops).toFixed(1)}`);
