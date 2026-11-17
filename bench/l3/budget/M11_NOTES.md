# L3-10 M11 notes: work cards

Source of truth: `reference/backend-request-budget.ts` lines 140-160 (validators) and 476-518 (`setCard`, `hasCard`);
spec §4.7, §6.1, §9, §15 rows 18-21 and KD-1.

## What is implemented

`bud_card.pruv` (depends on `crate::ent_u16`, `crate::bud_num`, `crate::bud_text`, `crate::bud_date`):

- Full card validation in the TS order `missing -> deliverable -> done -> targets -> verify -> conflicts -> state`,
  first error wins.
- Projection of the stored card: trimmed deliverable/done/labels/conflicts, `updatedAt = ISO(now)`, the three target
  counts, `note` only when it is a nonempty string (raw, untrimmed), statuses and state unchanged, and
  `by = {sessionId, turnId}` only when `by` is truthy. Unknown fields and the supplied `updatedAt`/`by` are ignored;
  absent optional fields replace rather than merge (§15 row 21).
- `setCard` decision including whether `now()` was consumed (`used_now`, KD-1 / §15 row 18): false for missing and for
  deliverable/done errors, true for optional-field errors and for success.
- `hasCard` = `Boolean(budget.card)` with JS truthiness (`NaN`, `+-0`, `""`, `null`, `false`, undefined are false).
- Language errors: `card_error(kind, lang)` from `bud_text`; `lang_code` gives en only for the exact string `"en"`,
  ko otherwise (§6.1).
- `put_card`: the budget object with the `card` value replaced in place, or the pair appended at the end (TS
  property-order behaviour of `budget.card = next`).

`m11_test/` (test only, not product):

- `root.pruv`: entries `m11_init/grow/load/set_card/query/dump` and perf entries `m11_perf_card` (item A) and
  `m11_perf_chain` (item C). It contains a **stand-in** for the M7 session table (`bud_world::session_put` /
  `bud_map` lookups, blob = budget V). CURRENT, closed, isHalted, the stored budget and the result value are decided
  in Pruv (root + `bud_card`); the Rust harness only moves words and supplies `now_ms`.
- `harness.rs`, `hand.rs` (hand Rust baseline doing the same work), `perf_ts.mjs` (pinned TS in-process),
  `run_m11.py` (build, Q replay, perf; report `work/m11/report.json`).

## Sub-operation interface (all `pub` in `bud_card.pruv`)

| Operation | Result |
|---|---|
| `v_end(v, at) -> i64` | end offset of the value at `at`; -1 if malformed / out of range (depth <= 32) |
| `obj_get(v, at, key: str) -> i64` | offset of the value of the last pair with `key` in the object at `at`, else -1 |
| `text_ok(v, at) -> bool` | validText: string, trimmed UTF-16 length 1..300, no allocation |
| `lines_ok(v, at, min, max) -> bool` | validLines: array, count in min..max checked first, every element `text_ok` |
| `check_required(v, at) -> i64` | 0 ok, 1 deliverable, 2 done |
| `check_optional(v, at) -> i64` | 0 ok, or first failing of 3 targets, 4 verify, 5 conflicts, 6 state |
| `validate(v, at) -> i64` | `check_required` then `check_optional` |
| `build(v, at, by, by_at, now_ms) -> [i64]` | stored card V, key order deliverable, done, updatedAt, targets?, verify?, conflicts?, state?, by? (size pass then write pass) |
| `lang_code(v, at) -> i64` | 1 iff the value is the string `"en"` |
| `truthy(v, at) -> bool` | JS truthiness |
| `set_card(found, closed, v, at, lang, by, by_at, now_ms) -> SetCard` | `SetCard { kind, used_now, card, error }`, kind -1 ok, 0 missing, 1..6 field |
| `has_card(budget, at) -> bool` | `Boolean(budget.card)` |
| `put_card(budget, at, card) -> [i64]` | budget with `card` replaced in place or appended; empty array if the budget is malformed |

The caller (M7 CURRENT, here the stand-in) passes `found`/`closed` and, on kind -1, stores `put_card(...)`, then saves
and signals changed. `trim_s`/`trim_e` (trim bounds without allocation) are marked as candidates for `bud_text`.

## Behaviour classification (items not already in §15)

1. `card` is `null` or `undefined`: TS throws `TypeError` at `card.deliverable`; Pruv returns the deliverable error.
   **Accidental** (no guard in B). Excluded-domain like other throws (§15 #26); Q6 never generates it (`gen_card`
   always produces an object). Recorded as **KD-M11-1**.
2. Card that is a non-object primitive or an array: TS reads `undefined` fields and fails on deliverable; Pruv does the
   same. **Accidental**, same result, compared.
3. A verify item that is truthy but not an object (string, number): TS fails on `label`; Pruv fails the same field (4).
   **Accidental**, same result.
4. Duplicate keys in one encoded object: last pair wins (`obj_get`), as a JS object literal would. Not producible from
   JS values; encoding-only, excluded-domain.
5. `now` is supplied by the host on every call; Pruv decides whether it was consumed (`used_now`) and the trace counts
   one `now` only then. **Intended** (KD-1 design: one `now` per public call; the host owns the clock side effect).
6. Positions and sizes saturate at 2^40 words, which is above the §8.4 input bound (2^24 words). Resource bound, not a
   behaviour.

## Q results (Q6, 300 cases, pinned TS oracle)

| Seed | Pair | exact | known-difference | not-implemented (prefix-ok) | fail |
|---|---|---|---|---|---|
| 20261117 | py/rust | 213 | 0 | 87 | 0 |
| 20261117 | rs/rust | 213 | 0 | 87 | 0 |
| 777 | rs/rust | 209 | 0 | 91 | 0 |

Not-implemented = cases with a `halt` op (M10). Only the calls before the first halt are replayed and compared; they
are reported as `prefix-ok` and never count as passes. No host code fills them in. Compared per request: the result
value (budget wrappers must name the session's budget), trace counts of findSession / now / save / changed, and
`finalSessions` at the end. Over seed 20261117 the replay drives 1844 setCard, 85 current, 76 hasCard and 74
isHalted calls. The implementing worker reports that injected errors (wrong error text, extra `now`, wrong key order) were
reported as fails; this was not re-run for these notes.

## Performance (rs/rust, `rustc -O`, median of 7 alternating runs, ns per call)

Before timing, `handcheck` asserts that `hand.rs` writes exactly the same result words as the Pruv entry (a minimal
card and a card with every optional field, three successive calls each); `run_m11.py --perf` aborts otherwise.

| item | size | Pruv ns | hand Rust ns | TS ns | Pruv/hand (before 2026-11-17) |
|---|---|---|---|---|---|
| A validate+build, U units | 100 | 433.6 | 550.3 | 631.1 | 0.79 (1.02) |
| A | 300 | 591.9 | 614.3 | 729.8 | 0.96 (1.09-1.17) |
| A (deliverable > 300: error path) | 1000 | 19.9 | 20.7 | 191.4 | 0.96 (1.41) |
| B n successive setCard, one budget | 100 | 683.3 | 766.6 | 643.2 | 0.89 (1.41) |
| B | 500 | 679.4 | 760.9 | 654.5 | 0.89 (1.40) |
| B | 1000 | 687.0 | 765.0 | 654.7 | 0.90 (1.41) |
| C = B without stand-in store / out words | 100 | 485.5 | 608.8 | - | 0.80 (1.03) |
| C | 1000 | 490.3 | 612.4 | - | 0.80 (1.02) |

All items are within the 1.1x bar (tightened 2026-11-17).

Before fixing `put_words`, item A was 1.66-2.59x. Its size pass walked word by word instead of advancing the
position.

What changed for the 1.1x bar (results unchanged: Q6 exact 213, prefix-ok 87, `handcheck` same words):

- **Runner (`run_m11.py --perf`).**
  - A runs `max(2000, 2000000 // U)` reps instead of a fixed count. At U = 1000 the old 200 reps of a 20 ns call
    timed mostly cold start.
  - Successive runs alternate the order of the Pruv and hand binaries. Pruv always ran first before, while caches
    and the clock were still cold.
  - These two changes alone moved A1000 from 1.41 to about 1.15.
- **A error path and A300 (`bud_card.pruv`).**
  - The steady-state gap was the generated `v_end_d` / `obj_get` / `key_is` / `trim_*` doing every V-word read
    through the total `rd`, and recursing into `v_end_d` for every array element.
  - The hand code reads the slice directly in one inlined loop.
  - Now: one range check, then plain proven reads (the old `rd` code is the fallback), and string elements are
    skipped inline in the element loop.
  - `build` did the six `obj_get` walks twice (size pass and write pass). The new `card_at` does them once and
    `put_card_words` takes the offsets.
  - A scratch build with `#[inline(always)]` on the generated `obj_get` / `text_ok` / `check_required` also reached
    parity. So the original residue was rustc not inlining the generated non-inline functions, and the source
    changes above remove the need for it.
- **B (`m11_test/root.pruv`).**
  - `blob_of` copied the budget word by word through `bud_map::blob_unit` (field lookups per unit). It now reads
    `blob_off` once and copies the region with one range check.
  - `emit_all` wrote each result word through the per-word `emit`. It now copies with one range check when the
    words fit.
  - The per-unit path stays as the fallback in both.

Item B is now 0.89-0.90x hand Rust and about 1.05x TS. The TS figure mutates the budget inside a live `Map` with no
stand-in store, so it does less work. C (no stand-in) is 0.75x TS. Per-call time is flat in n at all three sizes
(no growth with repeated cards).

## Complexity check against §9

- `setCard`: `O(U + card structure)`. Text checks and trimming touch each read unit once (`text_ok` does not
  allocate); `lines_ok` and `verify_ok` check the count before scanning, so length failures short-circuit; the first
  failing field stops validation. Each `obj_get` walks the card's top-level pairs with `v_end`, which skips strings in
  O(1) but walks nested arrays/objects; there is a fixed number of lookups (about 13 including `build`), so the cost
  is linear in the card's structural words with a constant factor. Deviation from the letter of §9 ("inspected input
  elements"): a large unknown nested field is skipped structurally, in O(its element count), which TS does not pay.
  It stays linear in the encoded input, which the adapter already validates in O(input words).
- `build` and `put_card`: one size pass and one write pass, `O(output)`; `put_card` copies the budget once.
- `hasCard`, `current`, `isHalted`: `O(key length)` plus the store lookup; `has_card` does one `obj_get` on the
  budget's top level (fixed field count).
- Measured: A grows with U (536.8 -> 663.7 ns for 100 -> 300 units); B and C are flat over n = 100..1000.

## Not implemented

- `halt` (M10): Q cases containing it are prefix-only (see above).
- M7 session lifecycle / CURRENT: replaced here by the test stand-in in `m11_test/root.pruv`.

## Audit

One audit (Codex gpt-6-astra, static): no behaviour difference in `bud_card.pruv` for the in-domain cases (error order,
`used_now`, trim set and UTF-16 lengths, raw note, `-0`/`2^53`/`1e300` counts, verify, stored key order, optional
replacement, `by` projection, in-place/append card, truthiness, language texts); no judgment found in the host.
Two required findings, both in the perf baseline, fixed:

1. `hand.rs` `build` wrote the targets object without the `targets` key (not exercised by the perf card). Fixed, and
   `handcheck` now compares a card with every optional field.
2. Item B was not the same work: hand returned only the kind while Pruv validates the input shapes and writes the
   result words. Hand `Store::set_card` now does both; `handcheck` asserts equal words. B moved from 1.47-1.49x to
   1.40-1.41x.

Advisory, recorded and not changed:

- `hand.rs` stores `by` on `is_some()` rather than JS truthiness and formats only 4-digit years; the perf inputs use
  `by = None` and current dates, so the timed work is unaffected.
- The stand-in copies the whole budget on every lookup, so `hasCard`/`current` timing through it includes the budget
  size; and the Q comparison checks values and per-request effect counts, not object identity or effect order.
  Both belong to the M7 store and product integration.

Confirmation audit (same reviewer, scope = the two fixes): both FIXED, no new required finding.
