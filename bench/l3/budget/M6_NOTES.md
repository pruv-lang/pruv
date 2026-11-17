# L3-10 M6 notes: request-budget text, number and date helpers

Source of truth: `reference/backend-request-budget.ts` lines 60-160, `shared-engine-text.ts` `engineText`, spec §0, §2.3, §6, §15.

## Modules

| File | Depends on | Contents |
|---|---|---|
| `bud_num.pruv` | none | `safe_max`, `is_safe`, `NumR`, `add_safe`/`sub_safe`/`mul_safe`, `max2`, `floor_div`, `minutes_of`, `count_class`/`count_ok_bits`/`count_ok_int`, `Unit`, `unit_default`, `unit_of` |
| `bud_text.pruv` | `crate::ent_u16`, `crate::bud_num` | `lang_of`, `lang_excluded`, `int_text`, `counter_name`, `reason_name`, `no_limit`, `limit_text`, `amount_text`, `refusal`, `continue_text`, `card_missing`, `card_error`, `Wire`, `refusal_wire`, `text_len_ok`, `raw_len_ok` |
| `bud_date.pruv` | `crate::ent_u16` | `date_min`, `date_max`, `date_ok`, `iso` |

Encodings: string = `[i64]` of UTF-16 units; lang 0 = ko, 1 = en; reason 0 time, 1 delegations, 2 commands, 3 reviews, 4 checks;
card error kind 0 missing, 1 deliverable, 2 done, 3 targets, 4 verify, 5 conflicts, 6 state. SAFE = 9007199254740991.

Interface notes:
- `lang_excluded` is marked `! alloc` (it compares against literals, which allocate in code). `lang_of` is not, and compares units directly.
- `iso` and the numeric helpers state their `requires` as explicit bounds (equivalent to `date_ok` / `is_safe`).
- `bud_text` decodes UTF-8 literals into UTF-16 units with a private two-pass decoder (O(output length), BMP only).
- `iso` shifts by 700 whole 400-year cycles (8835946560000000 ms) so all divisions are on nonnegative numbers.
- `mul_safe` guards with `|a| > SAFE / |b|` before multiplying. Results are `NumR { ok: false, v: 0 }` when outside the safe domain.
- `count_class`: 0 accepted (+0, -0, positive integers of any magnitude incl. 2^53 and 1e300), 1 negative integer, 2 non-integer finite (fractions, subnormals), 3 NaN, 4 +-Infinity.

## Behaviour classification (spec §0)

1. Texts, counter names, ko default, and ja/zh/keep/""/other language -> ko fallback (`entry[lang] ?? entry.ko`): **intended**, compared.
2. Language strings equal to `Object.prototype` property names (`constructor`, `__proto__`, `toString`, ...): **accidental** (JS returns the inherited property); excluded-domain via `lang_excluded`.
3. `String(limit)`, `String(used)` for safe integers and `Math.floor(activeMs / 60000)`: **intended**, compared. For safe integers the double division never rounds across an integer boundary, so exact floor division equals JS (the half-ulp at 2^37..2^38 is 2^-16 ≈ 1.53e-5 < 1/60000).
4. Arithmetic results outside the safe range: JS rounds doubles silently = **accidental**; Pruv reports excluded-domain (`NumR.ok == false`), per §15.2.
5. `Number.isInteger` card counts: -0, 2^53, 1e300 accepted; NaN, ±Infinity, fractions, negatives rejected: **intended** (§2.3); must not become `isSafeInteger`.
6. ISO text for the full JS Date range incl. extended years (`+275760-09-13T00:00:00.000Z`, `-271821-04-20T00:00:00.000Z`): **intended**.
7. Unlimited wire keeps the numeric limit (`refusalWire` limit as given): **intended** (§15 row 12).
8. Note length raw vs trimmed (`targets.note` untrimmed <= 300, text fields trimmed 1..300): **accidental**, kept (§15 row 19).
9. `budgetUnitOf` of a non-typed / partial setting: **excluded-domain** (only undefined and complete five-field settings are modelled).

## Known differences

- **KD-M6-1**: arithmetic whose exact result leaves the safe integral domain -> excluded-domain in Pruv (JS rounds silently).
- **KD-M6-2**: languages that are `Object.prototype` property names -> excluded-domain in Pruv (JS returns the inherited property).
- **KD-M6-3** (performance, not behaviour): String(int) is above TS at n = 1000 (about 30 vs 24 ns); profiled: the
  per-call result allocation and free are about 80% of the cost. See Performance.

## Open spec questions

- Should `lang_excluded` be callable from non-alloc code? It needs `! alloc` for its literals; a table-free variant would need a hand-coded character trie.
- `Wire.limit` and `used` for non-safe values are unconstrained in `refusal_wire` (limit is passed through); `used` must be safe. Confirm the driver never passes unsafe limits.
- `iso` returns an empty array on a defect of the guarded calendar walk (never expected in range); confirm this is acceptable as a defect signal.

## Commands

```
python3 bench/l3/budget/m6_test/gen_text.py                   # after changing a template text in bud_text.pruv
python3 bench/l3/budget/m6_test/run_m6.py                     # build py/rust and rs/rust, 300 seeded cases vs Node
python3 bench/l3/budget/m6_test/run_m6.py --perf --no-build   # 100/500/1000-call timing vs the Node helpers
```

Only the py/rust and rs/rust pairs are run; the LLVM pairs are not part of M6.

## Results

300 cases (seed 20261117: class 50, int 45, floor 40, iso 45, arith 30, text 60, trim 30): py/rust 300/300 exact,
rs/rust 300/300 exact. Every expected value comes from Node: the real pinned helpers for texts, wire, card errors and
language selection (`lang_excluded` is observed through `cardMissing`, not re-stated), plain JS for numbers and dates.

Driver note: time-reason cases pass `used` to the Pruv side as activeMs (`used*60000 + rem`, clamped to the safe range),
the same value the TS budget holds, so the minute floor runs on both sides.

## Performance

ns per call, min of 3 reps after one warm-up, input alternating by +0/+1 per call. TS is a Node loop over the real
helper. Both sides consume every result the same way (length plus parity of the first, middle and last unit) and the
driver flags any checksum mismatch (none in the run below), so neither side can skip producing the text; `charCodeAt`
also makes V8 flatten its rope strings. The refusal perf input is the commands reason (no minute floor in the loop).

| op | n = 100 / 500 / 1000 | py/rust | rs/rust | TS |
|---|---|---|---|---|
| iso | | 36.2 / 37.4 / 32.9 | 32.5 / 27.5 / 30.6 | 440 / 352 / 337 |
| refusal | | 51.7 / 49.2 / 47.0 | 43.8 / 47.6 / 47.5 | 165 / 86 / 74 |
| String(int) | | 31.2 / 36.9 / 32.5 | 28.3 / 32.6 / 32.3 | 52 / 53 / 24 |

Per-call cost does not grow with n (1000/100 ratio 0.9-1.14). iso and refusal are below TS at every size; String(int)
is below TS at 100 and 500 and above it at 1000 (KD-M6-3).

History: the first refusal decoded the UTF-8 template literals and concatenated four times per call (350-540 ns, 6-7x
TS). refusal and continue_text now size the output once and write pre-packed UTF-16 constants generated by
`m6_test/gen_text.py` from the `dec("...")` literals of `refusal_cold`/`continue_cold` (the single source of the texts;
`run_m6.py` fails if the generated region is stale). An earlier table showed String(int) at 4-5 ns: the Pruv loop only
summed lengths, so LLVM dropped the unused allocation; the audit found the asymmetry and both loops now read the content.

### KD-M6-3: String(int) at n = 1000

After V8's optimizing tier kicks in, `String(n)` costs about 21-25 ns per call (also with a distinct input per call, so
it is not V8's number-string cache). Pruv's int_text is about 27-32 ns.

Profiled 2026-11-17 (Apple M4, macOS 26, `sample` over the py/rust harness running `p_int 1000000`): of the 2178
samples inside `p_int`, 1435 are in `free` (`_xzm_free`, most of it in `mach_absolute_time` called by the xzone
allocator), about 300 in `calloc` (`_xzm_xzone_malloc_tiny`, `__bzero`, stubs) and 491 in the loop itself
(int_len + digit writes). So the per-call heap allocation and release of the 13-word result is about 80% of the cost;
the digit arithmetic is about 20%. V8 bump-allocates the string in its young generation.

Hand-edited copies of the generated Rust, same harness and checksum, measured in the same run (ns/call at n = 1000):

| variant | ns |
|---|---|
| as generated | 27-32 |
| unsigned `/` and `%` where the operand is proven non-negative | 27-30 (within noise) |
| `Vec::with_capacity` + `resize` instead of `vec![0; n]` (malloc instead of calloc) | 31-32 (no gain) |
| the loop's result buffer reused across iterations (no allocation per call) | 11.7-12.1 |

An earlier prototype with a comparison-only length and two digits per division reached only 28 ns. Only removing
the per-call allocation gets below TS. The Rust backend cannot make the allocation itself cheaper without replacing
the host's global allocator (not acceptable for generated library code), so the remaining options are new code
generation features in both compilers, to be designed before implementing:

1. Destination passing for loop-local array results: when a loop body binds `let r: [T] = f(..)` and `r` only is
   read (len, index) and dies in the iteration, and `f` builds its result with one `alloc` that it returns, emit an
   `f_into(&mut buf, ..)` variant and reuse one buffer hoisted out of the loop. This is what the 12 ns row measures;
   it removes the cost for callers that consume a result in a loop, not for a single isolated call.
2. Stack results for bounded length: when `f` has a small proven upper bound on `len(result)` (int_text: 17) and the
   call site does not let the result escape, pass a stack array instead of a `Vec`. This also helps single calls,
   but needs an escape check at the call site and a length bound in the contract.

Until one of these lands, String(int) stays above TS at n = 1000 on this machine.
