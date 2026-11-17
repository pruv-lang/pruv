// In-process JS Map/Set baseline for the M4 perf axes (same operation sequence and work boundary as driver_m4 perf).
//   node perf_ts.mjs <axis> <n> <reps>      axis = sessions | tools | clocks
//   node perf_ts.mjs selftest               scripted check of exec() against the CONTRACT result words
// Prints one line: axis=.. n=.. ts_ns_per_op=..
//
// Work boundary (same as `sol::m4_op(&mut st, op, &mut out)` in the Rust driver): every op is a plain JS array of
// integers `[code, args...]` with keys encoded `len, units...`. The timed loop calls exec(op, out) once per op; exec
// decodes and validates the words (key lengths, units 0..65535, exact op length per code, waiting 0/1, session_put
// n == remaining words), builds the key strings from the units on every call (no string caching between calls),
// applies the operation to JS Map/Set state and writes the CONTRACT result words into `out` (out[0] = number of
// result words, out[1..] = result). Op encoding (prefill and timed op list) is outside the timed region, like the
// Rust driver's prefill and encode phases. As in Rust, the timed loop replays one period of the op list cyclically
// (sessions period 1 rep, tools period n reps, clocks period 4 reps); only that period is materialised here, which
// yields the identical op stream.
//
// `out` is a Float64Array: it holds any JS integer word exactly (like the i64 words on the Rust side), element reads
// and writes stay unboxed numbers, and it is preallocated once (4096 words, as the Rust driver's minimum).

const OUT_WORDS = 4096;

// ---- state (module level, reset by newState) ----------------------------------------------------------------------
let sessions; // Map<string, number[]>
let clocks; // Map<string, {sid, since_present, since, armed, deadline, runs: Map<string, number>}>
let toolSeen; // Map<string, Set<string>>

function newState() {
  sessions = new Map();
  clocks = new Map();
  toolSeen = new Map();
}

// ---- decoding -----------------------------------------------------------------------------------------------------
let keyStr = "";

// Decodes the key `len, units...` starting at p into keyStr. Returns the end position, or -1 if invalid
// (length not an integer in range / beyond the op, or a unit that is not an integer in 0..65535).
function decodeKey(op, p) {
  const n = op.length;
  if (p >= n) return -1;
  const kn = op[p];
  if (!Number.isInteger(kn) || kn < 0 || kn > n - p - 1) return -1;
  let s = "";
  for (let j = 0; j < kn; j++) {
    const u = op[p + 1 + j];
    if (!Number.isInteger(u) || u < 0 || u > 65535) return -1;
    s += String.fromCharCode(u);
  }
  keyStr = s;
  return p + 1 + kn;
}

// Executes one op. Returns 0 ok, 2 invalid, 3 out too small (result words are in out).
function exec(op, out) {
  const code = op[0];
  if (code === 1) {
    // session_put: key, n, n words
    const e1 = decodeKey(op, 1);
    if (e1 < 0 || e1 >= op.length) return 2;
    const k = keyStr;
    const cnt = op[e1];
    if (!Number.isInteger(cnt) || cnt < 0 || cnt > 65535 || cnt !== op.length - e1 - 1) return 2;
    const blob = new Array(cnt);
    for (let i = 0; i < cnt; i++) blob[i] = op[e1 + 1 + i];
    const existed = sessions.has(k);
    sessions.set(k, blob);
    out[0] = 1;
    out[1] = existed ? 0 : 1;
    return 0;
  }
  if (code === 2 || code === 14) {
    // session_get / tool_forget: one key
    const e1 = decodeKey(op, 1);
    if (e1 < 0 || e1 !== op.length) return 2;
    const k = keyStr;
    if (code === 2) {
      const b = sessions.get(k);
      if (b === undefined) {
        out[0] = 1;
        out[1] = 0;
        return 0;
      }
      const cnt = b.length;
      if (3 + cnt > out.length) return 3;
      out[0] = 2 + cnt;
      out[1] = 1;
      out[2] = cnt;
      for (let i = 0; i < cnt; i++) out[3 + i] = b[i];
      return 0;
    }
    out[0] = 1;
    out[1] = toolSeen.delete(k) ? 1 : 0;
    return 0;
  }
  if (code === 5 || code === 9 || code === 10 || code === 13) {
    // two keys (run_set: plus waiting word)
    const e1 = decodeKey(op, 1);
    if (e1 < 0) return 2;
    const k1 = keyStr;
    const e2 = decodeKey(op, e1);
    if (e2 < 0) return 2;
    const k2 = keyStr;
    if (code === 9) {
      if (op.length - e2 !== 1) return 2;
      const w = op[e2];
      if (w !== 0 && w !== 1) return 2;
      const c = clocks.get(k1);
      out[0] = 1;
      if (c === undefined) {
        out[1] = 2;
      } else {
        out[1] = c.runs.has(k2) ? 0 : 1;
        c.runs.set(k2, w);
      }
      return 0;
    }
    if (e2 !== op.length) return 2;
    out[0] = 1;
    if (code === 5) {
      const c = clocks.get(k1);
      if (c === undefined) {
        clocks.set(k1, { sid: k2, since_present: 0, since: 0, armed: 0, deadline: 0, runs: new Map() });
        out[1] = 1;
      } else {
        c.sid = k2;
        out[1] = 0;
      }
    } else if (code === 10) {
      const c = clocks.get(k1);
      out[1] = c !== undefined && c.runs.delete(k2) ? 1 : 0;
    } else {
      let s = toolSeen.get(k1);
      if (s === undefined) {
        s = new Set();
        toolSeen.set(k1, s);
      }
      if (s.has(k2)) {
        out[1] = 0;
      } else {
        s.add(k2);
        out[1] = 1;
      }
    }
    return 0;
  }
  return 2;
}

// ---- op encoding (outside the timed region) -----------------------------------------------------------------------
function pushKey(v, s) {
  v.push(s.length);
  for (let i = 0; i < s.length; i++) v.push(s.charCodeAt(i));
}

function opSessionPut(k, blob) {
  const v = [1];
  pushKey(v, k);
  v.push(blob.length);
  for (const x of blob) v.push(x);
  return v;
}

function opK(code, k) {
  const v = [code];
  pushKey(v, k);
  return v;
}

function opKK(code, a, b) {
  const v = [code];
  pushKey(v, a);
  pushKey(v, b);
  return v;
}

function opRunSet(c, rk, w) {
  const v = [9];
  pushKey(v, c);
  pushKey(v, rk);
  v.push(w);
  return v;
}

// ---- perf ---------------------------------------------------------------------------------------------------------
let sink = 0;

function runCheck(op, out, what) {
  const s = exec(op, out);
  if (s !== 0) {
    console.error("perf: " + what + " failed with status " + s);
    process.exit(1);
  }
}

function runOnce(axis, n, reps) {
  newState();
  const out = new Float64Array(OUT_WORDS);
  const blob = [11, 22, 33, 44, 55, 66, 77, 88];

  // Prefill (not timed).
  if (axis === "sessions") {
    for (let i = 0; i < n; i++) runCheck(opSessionPut("s" + i, blob), out, "prefill session_put");
  } else if (axis === "tools") {
    for (let i = 0; i < n; i++) runCheck(opKK(13, "t0", "a" + i), out, "prefill tool_note");
  } else if (axis === "clocks") {
    for (let i = 0; i < n; i++) runCheck(opKK(5, "c" + i, "s0"), out, "prefill clock_put");
  } else {
    console.error("unknown axis " + axis);
    process.exit(2);
  }

  // Op list for one period of the cycle (not timed). width = ops per rep; same order as driver_m4.rs.
  let per, width;
  if (axis === "sessions") {
    per = 1;
    width = 2;
  } else if (axis === "tools") {
    per = n;
    width = 3;
  } else {
    per = 4;
    width = 2;
  }
  per = Math.min(per, reps);
  const cycle = [];
  if (axis === "sessions") {
    const k = "s" + (n >> 1);
    for (let r = 0; r < per; r++) {
      cycle.push(opK(2, k));
      cycle.push(opSessionPut(k, blob));
    }
  } else if (axis === "tools") {
    for (let r = 0; r < per; r++) {
      cycle.push(opKK(13, "t0", "a" + (r % n)));
      cycle.push(opKK(13, "x", "y"));
      cycle.push(opK(14, "x"));
    }
  } else {
    const c = "c" + (n >> 1);
    for (let r = 0; r < per; r++) {
      const rk = "r" + (r % 4);
      cycle.push(opRunSet(c, rk, 1));
      cycle.push(opKK(10, c, rk));
    }
  }
  const nops = reps * width;
  const clen = cycle.length;

  const t0 = process.hrtime.bigint();
  let s = 0;
  let j = 0;
  for (let i = 0; i < nops; i++) {
    const st = exec(cycle[j], out);
    j++;
    if (j === clen) j = 0;
    if (st !== 0) {
      console.error("perf: op failed in timed loop (status " + st + ")");
      process.exit(1);
    }
    s += out[0] + out[1];
  }
  const ns = process.hrtime.bigint() - t0;
  sink += s;
  return { ns, ops: nops };
}

// ---- selftest -----------------------------------------------------------------------------------------------------
function selftest() {
  newState();
  const out = new Float64Array(OUT_WORDS);
  const steps = [
    // [description, op, expected status, expected result words]
    ["session_put new", opSessionPut("ab", [5, 6, 7]), 0, [1]],
    ["session_get", opK(2, "ab"), 0, [1, 3, 5, 6, 7]],
    ["session_put replace", opSessionPut("ab", [9]), 0, [0]],
    ["session_get after replace", opK(2, "ab"), 0, [1, 1, 9]],
    ["session_get absent", opK(2, "zz"), 0, [0]],
    ["session_put empty blob", opSessionPut("e", []), 0, [1]],
    ["session_get empty blob", opK(2, "e"), 0, [1, 0]],
    ["run_set without clock", opRunSet("c1", "r1", 1), 0, [2]],
    ["clock_put new", opKK(5, "c1", "s1"), 0, [1]],
    ["clock_put existing", opKK(5, "c1", "s2"), 0, [0]],
    ["run_set new", opRunSet("c1", "r1", 1), 0, [1]],
    ["run_set again", opRunSet("c1", "r1", 0), 0, [0]],
    ["run_del present", opKK(10, "c1", "r1"), 0, [1]],
    ["run_del absent", opKK(10, "c1", "r1"), 0, [0]],
    ["run_del no clock", opKK(10, "c9", "r1"), 0, [0]],
    ["tool_note new", opKK(13, "t", "a"), 0, [1]],
    ["tool_note seen", opKK(13, "t", "a"), 0, [0]],
    ["tool_note second activity", opKK(13, "t", "b"), 0, [1]],
    ["tool_forget present", opK(14, "t"), 0, [1]],
    ["tool_forget absent", opK(14, "t"), 0, [0]],
    ["tool_note after forget", opKK(13, "t", "a"), 0, [1]],
    ["key unit 70000", [2, 1, 70000], 2, null],
    ["key unit -1", [2, 1, -1], 2, null],
    ["key length beyond op", [2, 1, 97, 5], 2, null],
    ["wrong length: trailing word", [2, 1, 97, 7], 2, null],
    ["wrong length: tool_note trailing", [13, 1, 97, 1, 98, 7], 2, null],
    ["wrong length: tool_note missing key", [13, 1, 97], 2, null],
    ["run_set waiting 2", [9, 1, 97, 1, 98, 2], 2, null],
    ["run_set missing waiting", [9, 1, 97, 1, 98], 2, null],
    ["session_put n mismatch", [1, 1, 97, 3, 1], 2, null],
    ["session_put missing n", [1, 1, 97], 2, null],
    ["unknown code", [99], 2, null],
    ["empty op", [], 2, null],
  ];
  for (const [what, op, wantStatus, wantRes] of steps) {
    out.fill(0);
    const st = exec(op, out);
    if (st !== wantStatus) {
      console.log("selftest mismatch: " + what + ": status " + st + ", expected " + wantStatus);
      process.exit(1);
    }
    if (wantRes !== null) {
      const got = [];
      for (let i = 0; i < out[0]; i++) got.push(out[1 + i]);
      if (out[0] !== wantRes.length || got.join(",") !== wantRes.join(",")) {
        console.log("selftest mismatch: " + what + ": result [" + got.join(",") + "] (out[0]=" + out[0] + "), expected [" + wantRes.join(",") + "]");
        process.exit(1);
      }
    }
  }
  console.log("selftest ok");
}

// ---- main ---------------------------------------------------------------------------------------------------------
if (process.argv[2] === "selftest") {
  selftest();
} else {
  const [, , axis, nArg, repsArg] = process.argv;
  const n = parseInt(nArg, 10);
  const reps = parseInt(repsArg, 10);
  if (!axis || !(n > 0) || !(reps > 0)) {
    console.error("usage: node perf_ts.mjs <axis> <n> <reps> | selftest");
    process.exit(2);
  }
  // Warm-up on separate state so the JIT has compiled exec and the loop, then the measured run.
  runOnce(axis, n, Math.min(reps, 5000));
  runOnce(axis, n, Math.min(reps, 5000));
  const res = runOnce(axis, n, reps);
  console.error("sink=" + sink);
  console.log(`axis=${axis} n=${n} ts_ns_per_op=${(Number(res.ns) / res.ops).toFixed(1)}`);
}
