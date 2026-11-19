// M6 Node reference: reads JSONL requests on stdin, writes one JSONL result per request ({"w":[words]} or {"err":"..."}).
// The expected values come from the REAL pinned helpers (reference/backend-request-budget.ts) and from plain JavaScript semantics
// (Number.isInteger, String(n), Math.floor, Date#toISOString, String#trim). Run (run_m6.py does this):
//   GROK_TS_TRANSPILER=/path/to/typescript node --experimental-strip-types --no-warnings \
//     --import <budget>/reference-hooks.mjs node_ref.mjs < requests.jsonl > results.jsonl
// i64 values arrive as decimal strings (exact), strings as arrays of UTF-16 units. Result words use the harness output encoding.
import { createInterface } from 'node:readline';
import * as B from '../reference/backend-request-budget.ts';

const KEYS = ['time', 'delegations', 'commands', 'reviews', 'checks'];   // reason 0..4
const LKEYS = ['timeMinutes', 'delegations', 'commands', 'reviews', 'checks'];
const LANGS = ['ko', 'en'];
const SAFE = 9007199254740991n;

const bytesOf = s => Array.from(Buffer.from(s, 'utf8'));
const strOf = u => { let s = ''; for (const x of u) s += String.fromCharCode(x); return s; };
const big = v => BigInt(v);

// A minimal budget: only the reason's counter and limit are set, the others are 0. Time: used is MINUTES + rem ms (so the real
// code floors activeMs/60000 back to `used`).
function mkBudget(r) {
  const used = { activeMs: 0, delegations: 0, commands: 0, reviews: 0, checks: 0 };
  const limits = { timeMinutes: 0, delegations: 0, commands: 0, reviews: 0, checks: 0 };
  if (r.reason === 0) {
    used.activeMs = r.ms !== undefined ? Number(big(r.ms)) : Number(big(r.used) * 60000n + big(r.rem ?? 0));
    limits.timeMinutes = Number(big(r.limit));
  } else {
    used[KEYS[r.reason]] = Number(big(r.used));
    limits[KEYS[r.reason]] = Number(big(r.limit));
  }
  return { unlimited: !!r.unl, used, limits };
}

// counter name and no-limit text, observed through the real texts (counter_name/no_limit are private to the TS file)
const nameRe = [/한도\((.*) 0\/0\)/, /limit \((.*) 0\/0\)/];
const textRe = [/한도\((.*)\)에 닿아/, /limit \((.*)\) was reached/];
function counterName(k, lang) {
  const t = B.continueText(KEYS[k], mkBudget({ reason: k, unl: false, used: '0', limit: '0' }), LANGS[lang]);
  const m = t.match(nameRe[lang]);
  if (!m) throw new Error(`cannot extract counter name from ${JSON.stringify(t)}`);
  return m[1];
}
// amount text "used/limit" extracted from the real refusal text
function amountOf(r, budget = mkBudget(r)) {
  const t = B.refusal(KEYS[r.reason], budget, LANGS[r.lang]);
  const m = t.match(textRe[r.lang]);
  if (!m) throw new Error(`cannot extract amount from ${JSON.stringify(t)}`);
  const name = counterName(r.reason, r.lang);
  if (!m[1].startsWith(name + ' ')) throw new Error(`unexpected refusal text ${JSON.stringify(t)}`);
  return m[1].slice(name.length + 1);
}

// card errors from the real RequestBudgets.setCard with a scripted world of one session holding one open budget
function cardError(kind, lang) {
  const budget = {
    id: 'b1', openedAt: '2023-11-14T22:13:20.000Z', origin: 'user', state: 'open',
    limits: { timeMinutes: 120, delegations: 24, commands: 3, reviews: 2, checks: 1 },
    used: { activeMs: 0, delegations: 0, commands: 0, reviews: 0, checks: 0, tools: 0, failures: 0, retries: 0 },
  };
  const sessions = new Map([['s1', { id: 's1', budget }]]);
  const noop = () => {};
  const deps = {
    now: () => 1700000000000, unit: () => budget.limits, findSession: id => sessions.get(id), save: noop, changed: noop, stopMain: noop,
    stopDelegation: noop, runningDelegationIds: () => [], stopAttachedTurns: noop, holdQueued: noop, releaseHeld: () => false, carryHeld: noop,
    queueContinue: noop, dropHeld: noop, cancelRecovery: noop, isQuiet: () => true, announce: noop,
  };
  const rb = new B.RequestBudgets(deps);
  const base = { deliverable: 'd', done: ['x'] };
  const cards = [
    null,
    { deliverable: '   ', done: ['x'] },
    { deliverable: 'd', done: [] },
    { ...base, targets: { confirmed: -1, candidate: 0, unknown: 0 } },
    { ...base, verify: 'x' },
    { ...base, conflicts: 'x' },
    { ...base, state: 'bogus' },
  ];
  const ref = { sessionId: kind === 0 ? 'nobody' : 's1', budgetId: 'b1' };
  const res = rb.setCard(ref, cards[kind] ?? base, LANGS[lang]);
  if (res.ok) throw new Error(`setCard unexpectedly succeeded for kind ${kind}`);
  return res.error;
}

function classOf(x) {
  if (Number.isNaN(x)) return 3;
  if (x === Infinity || x === -Infinity) return 4;
  if (Number.isInteger(x)) return x >= 0 ? 0 : 1;
  return 2;
}

function handle(r) {
  switch (r.op) {
    case 'cls': {
      const x = new Float64Array(new BigUint64Array([BigInt('0x' + r.hex)]).buffer)[0];
      const c = classOf(x);
      return { w: [c, c === 0 ? 1 : 0] };
    }
    case 'oki': return { w: [big(r.v) >= 0n ? 1 : 0] };
    case 'sfe': { const v = big(r.v); return { w: [v >= -SAFE && v <= SAFE ? 1 : 0] }; }
    case 'ari': {
      const a = big(r.a), b = big(r.b);
      if (r.o === 3) return { w: [1, Number(a > b ? a : b)] };
      const v = r.o === 0 ? a + b : r.o === 1 ? a - b : a * b;
      return { w: v >= -SAFE && v <= SAFE ? [1, Number(v)] : [0, 0] };
    }
    case 'fdv': {
      const a = big(r.a), d = big(r.d);
      let q = a / d;                                  // BigInt division truncates
      if (a % d !== 0n && a < 0n) q -= 1n;            // floor
      return { w: [Number(q)] };
    }
    case 'min': {
      const ms = big(r.ms);
      let q = ms / 60000n;
      if (ms % 60000n !== 0n && ms < 0n) q -= 1n;
      // expected = the real JS expression of the TS helpers; `exact` shows where double division differs from exact floor
      return { w: [Math.floor(Number(ms) / 60000)], exact: Number(q) };
    }
    case 'int': return { w: bytesOf(String(Number(big(r.x)))) };
    case 'iso': return { w: bytesOf(new Date(Number(big(r.ms))).toISOString()) };
    case 'dok': return { w: [Number.isNaN(new Date(Number(big(r.ms))).getTime()) ? 0 : 1] };
    case 'ref': return { w: bytesOf(B.refusal(KEYS[r.reason], mkBudget(r), LANGS[r.lang])) };
    case 'con': return { w: bytesOf(B.continueText(KEYS[r.reason], mkBudget(r), LANGS[r.lang])) };
    case 'amt': return { w: bytesOf(amountOf(r)) };
    case 'lmt': return { w: bytesOf(B.limitText({ unlimited: !!r.unl }, Number(big(r.limit)), LANGS[r.lang])) };
    case 'wir': {
      const wire = B.refusalWire(KEYS[r.reason], mkBudget(r));
      return { w: [...bytesOf(wire.key), Number(wire.args.used), Number(wire.args.limit)] };
    }
    case 'cmi': return { w: bytesOf(B.cardMissing(LANGS[r.lang])) };
    case 'cer': return { w: bytesOf(cardError(r.kind, r.lang)) };
    case 'cnt': return { w: bytesOf(counterName(r.reason, r.lang)) };
    case 'nol': return { w: bytesOf(B.limitText({ unlimited: true }, 0, LANGS[r.lang])) };
    case 'lng': {
      // language selection of the real helpers: engineText(entry, language) = entry[language] ?? entry.ko. Observed through cardMissing.
      const en = B.cardMissing('en');
      const t = r.present ? B.cardMissing(strOf(r.u)) : B.cardMissing();
      return { w: [t === en ? 1 : 0] };
    }
    case 'lex': {
      // observed through the real helper: a language naming an Object.prototype property makes engineText return the
      // inherited value instead of a string (outside the typed domain -> excluded)
      const s = strOf(r.u);
      return { w: [typeof B.cardMissing(s) === 'string' ? 0 : 1] };
    }
    case 'len': {
      const s = strOf(r.u);
      const t = s.trim().length;
      return { w: [t >= 1 && t <= 300 ? 1 : 0, s.length <= 300 ? 1 : 0] };
    }
    case 'perf': return perf(r);
    default: throw new Error(`unknown op ${r.op}`);
  }
}

// Timing: n calls of the real helper in a loop with an input alternating by +0/+1; one warm-up rep, then the min over `reps`.
// Consume UTF-8 bytes just as root.pruv does.
const eat = text => {
  const bytes = Buffer.from(text, 'utf8');
  const n = bytes.length;
  return n + ((bytes[0] + bytes[n >> 1] + bytes[n - 1]) & 1);
};
function perf(r) {
  const n = r.n;
  const a = r.args.map(big);
  let body;
  if (r.name === 'p_iso') {
    const base = Number(a[0]);
    body = () => { let s = 0; for (let i = 0; i < n; i++) s += eat(new Date(base + (i & 1)).toISOString()); return s; };
  } else if (r.name === 'p_int') {
    const base = Number(a[0]);
    body = () => { let s = 0; for (let i = 0; i < n; i++) s += eat(String(base + (i & 1))); return s; };
  } else if (r.name === 'p_ref' || r.name === 'p_amount') {
    const [reason, unl, used, limit, lang] = a.map(Number);
    const b = mkBudget({ reason, unl, used: '0', limit: String(limit), rem: 0 });
    const key = reason === 0 ? 'activeMs' : KEYS[reason];
    body = () => {
      let s = 0;
      // time: used is activeMs, the same value root.pruv p_refusal receives
      for (let i = 0; i < n; i++) {
        b.used[key] = used + (i & 1);
        const text = r.name === 'p_ref' ? B.refusal(KEYS[reason], b, LANGS[lang])
          : amountOf({ reason, lang }, b);
        s += eat(text);
      }
      return s;
    };
  } else if (r.name === 'p_card_error') {
    const [kind, lang] = a.map(Number);
    body = () => {
      let sum = 0;
      for (let i = 0; i < n; i++) sum += eat(cardError(kind + (i & 1), lang));
      return sum;
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
  try { out = handle(JSON.parse(line)); } catch (e) { out = { err: String(e && e.message || e) }; }
  process.stdout.write(JSON.stringify(out) + '\n');
}
