# M6 test apparatus (L3-10 request-budget text/number/date helpers)

Compares the Pruv modules `bud_num.pruv`, `bud_text.pruv`, `bud_date.pruv` (in `bench/l3/budget/`) with the pinned TypeScript
(`reference/backend-request-budget.ts`) and with plain JavaScript semantics, case by case, and measures ns/call.

## Files

| File | Role |
|---|---|
| `root.pruv` | Root declaring `ent_u16`, `bud_num`, `bud_text`, `bud_date`; `t_*` entries (plain i64 / `[i64]` signatures, `requires` copied from the callees) and `p_*` loop entries for timing (return a checksum of every output: length plus parity of the first, middle and last unit) |
| `harness.rs` | Reads `op w1 w2 ...` lines (strings are `len u1 ..`), calls `sol::t_*` / `sol::p_*`, prints result words, or `ns <ns> <checksum>` for perf ops; a panic is printed as `PANIC <msg>` |
| `node_ref.mjs` | Reads JSONL requests, writes the expected words. Uses the real exported `refusal`, `continueText`, `limitText`, `refusalWire`, `cardMissing`, and the real `RequestBudgets.setCard` for card errors; JS `Number.isInteger`, `String(n)`, `Math.floor`, `toISOString`, `trim` for the rest |
| `run_m6.py` | Build, case generation, comparison, report, perf |

Generated output (not committed): `bench/l3/budget/work/m6/` (`src/`, `py/`, `rs/` with `sol.rs`, `main.rs`, `harness`; `report.json`).

## Commands

```sh
cd bench/l3/budget/m6_test
python3 run_m6.py                       # both pairs, 300 cases, seed 20261117
python3 run_m6.py --pairs rs --count 100
python3 run_m6.py --perf                # also ns/call at n = 100, 500, 1000
python3 run_m6.py --no-build            # reuse work/m6/*/harness
```

Options: `--pairs py,rs` (py = `python3 pruvc/pruvc.py ... --backend rust`, rs = `$PRUVC_RS` or
`/Users/spica4345/Projects/pruv/pruvc-rs/target/release/pruvc`), `--count N` (exact total, default 300), `--seed S`,
`--perf`, `--no-build`. Node: `$BUDGET_NODE` or `node` (common.py); the typescript package comes from `GROK_TS_TRANSPILER`
(default in `common.TS_ENV`). Exit code: 0 all exact, 1 a mismatch, 2 build/driver failure (compiler errors are printed verbatim).

## Cases (default 300, deterministic xorshift32)

| Category | n | Kinds |
|---|---|---|
| class | 50 | `count_class` / `count_ok_bits` on raw double bits: edges (+0, -0, 1, 0.5, -1, 2^53, 2^53+2, 1e300, 5e-324, NaN, +-Inf, max), `{num}` values from fixtures K, N and from `q_gen.gen(seed, ['Q6','Q8'], 40)`, random patterns |
| int | 45 | `int_text` vs `String(n)`: 0, +-1, +-SAFE, powers of 10 +-1, random |
| floor | 40 | `minutes_of` vs `Math.floor(ms/60000)` (k*60000-1, k*60000, negatives, near SAFE); `floor_div` vs exact BigInt floor |
| iso | 45 | `iso` vs `toISOString`: 0, -1, +-8.64e15, year 0 / 9999 / 10000 / -1 boundaries, random; `date_ok` vs `isNaN(new Date(ms))` |
| arith | 30 | `add_safe` / `sub_safe` / `mul_safe` with results just inside and outside +-SAFE (BigInt on the Node side; `[0,0]` expected when outside), `max2`, `is_safe`, `count_ok_int` |
| text | 60 | `refusal`, `continue_text`, `limit_text`, `amount_text`, `refusal_wire`, `card_missing`, `card_error`, `counter_name`, `no_limit`, `lang_of`, `lang_excluded`; ko/en, unlimited true/false, used/limit like Q8 |
| trim | 30 | `text_len_ok` / `raw_len_ok` vs `s.trim().length` in 1..300 and `s.length <= 300`: lone surrogates, astral pairs, U+FEFF, U+3000, U+2028, 300/301 units after trim |

Required cases come first in a category (text: every card-error kind, `card_missing` and `no_limit` in both languages;
trim: all fixed boundary strings), then the other candidate sublists are interleaved round-robin. At the default count
every kind is covered. A smaller `--count` truncates in that order, so it can drop interleaved kinds and, once the
quota is below the required list (e.g. trim at `--count 100`), some required cases too.
Every kind is labelled `node` (real TS function or JS expression). `text.lang_excl` is observed through the real
`cardMissing`: a language naming an `Object.prototype` property makes it return a non-string, which is excluded.

How the text expectations are observed: `counter_name` / `amount_text` are private in the TS file, so the expected
strings are cut out of the real `refusal` / `continueText` output (name from a `0/0` budget, amount = the rest);
`no_limit` is the real `limitText` of an unlimited budget; `lang_of` is observed through `cardMissing(lang)`
(`'en'` text -> 1, anything else -> 0, absent -> 0, per `entry[language] ?? entry.ko`). Card errors come from
`RequestBudgets.setCard` on a scripted one-session world (kind 0 = unknown session, 1..6 = an invalid card field).
For the time reason both sides get the same `activeMs = minutes*60000 + rem` (clamped to the safe range) and floor it.

`floor.min` is compared with the JS double expression `Math.floor(ms/60000)`; where that differs from the exact floor
(only possible near SAFE) the run prints a note (`ts_inexact_minutes` in the report).

## Perf

`--perf` runs `p_iso`, `p_ref` (refusal), `p_int` for n = 100, 500, 1000: Pruv (Instant around the call, 1 warm-up + 3 reps, min)
against a Node loop of `toISOString`, the real `refusal`, `String` (1 warm-up + 3 reps, min). Prints ns/call per pair and TS,
the checksums must agree, and the ratio ns/call(1000)/ns/call(100) per op. Flags: `<pair>>ts` (Pruv slower than TS),
`GROWTH(<pair>)` (ratio > 2). Perf flags do not change the exit code.

## Template writers

`refusal`/`continue_text` in `bud_text.pruv` write pre-packed UTF-16 constants from a generated region. After changing a
template text in `refusal_cold`/`continue_cold`, `counter_name`, `reason_name` or `no_limit`, run
`python3 bench/l3/budget/m6_test/gen_text.py`. `run_m6.py` starts with `gen_text.py --check` and stops if the region is stale.
