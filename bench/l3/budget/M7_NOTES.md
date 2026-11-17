# L3-10 M7 — request budget lifecycle (bud_lifecycle.pruv)

Scope: `current`, `isHalted`, OPEN, `attach`, `replaceForUserMessage`, `reopen`, CLOSE / `maybeClose`.
All decisions are in Pruv (`bud_lifecycle.pruv`); the host (`m7_test/driver_m7.rs`) only runs side effects
(fx frame: 13 timer clear, 20 changed, 21 carryHeld, 22 uuid) and answers the `isQuiet` question (status 5).
Not implemented here (reported per case as `not-implemented`, never filled in by the host):
M8 charge, M10 halt / raise / decide, M11 hasCard / setCard, M12.

## Record layout

24-word header: 0 state, 1 origin, 2 unlimited, 3–7 limits, 8–15 used, 16/17 activeSince kind/ms, 18 ended.how,
19 ended.at kind, 20/21 openedAt kind/ms, 22 ended.at ms, 23 follows; then 6 `len, units` strings: id, openedAt,
activeSince, ended.at, follows, rest. Session blob state −1 = session without a budget.

## Q results

Run: `m7_test/run_m7.py q` (Rust pruvc), `q --py` (Python pruvc), and the derived set.

| set | cases | exact | known-difference | not-implemented | fail |
|---|---|---|---|---|---|
| Q1 (Rust pruvc) | 300 | 113 | 0 | 187 | 0 |
| Q1 (Python pruvc) | 300 | 113 | 0 | 187 | 0 |
| derived lifecycle set (`work/m7/cases_life.jsonl`) | 300 | 300 | 0 | 0 | 0 |

Not-implemented reasons in Q1: hasCard (M11) 32, halt (M10) 62, decide (M10) 93. A case is not-implemented as soon
as it reaches one of these; every request before that point is still compared.
Compared requests in Q1: reopen 231, attach 415, replaceForUserMessage 445, maybeClose 79, current 32, isHalted 32.

Byte identity: `run_m7.py same` → Python and Rust pruvc emit identical Rust (139485 bytes each, after the review fixes).

## Intent / bug / chosen accident

Only intent is compared with the oracle.

- KD-M7-1 (accident): JSON key order of the stored budget object is not compared. The reference spreads
  `{...b, state, ended}` so key order depends on the path that wrote the record; the spec does not give it meaning.
  The comparison is on parsed values. No Q case is counted as known-difference because of this.
- TS fake UUID: the oracle routes `node:crypto` to a counter stream (`fakes/node-crypto.ts`); the host takes ids from
  the same stream (fx 22), so ids match exactly. Not a difference, noted because the TS perf column also uses the
  counter (cheaper than node's real `randomUUID`).

- KD-M7-2 (chosen bound): OPEN clamps each unit value to 2^53 − 1 (JS max safe integer) before multiplying by
  the size factor (≤ 8). The reference multiplies JS numbers, so it is exact only while the product stays a safe
  integer; above that the reference rounds and Pruv does not. The earlier 2^40 clamp cut values the reference keeps
  exactly (review finding 1) and was raised. No Q case reaches either bound.

No other bug or accident was found in the M7 surface.

## Performance (ns per call, median, `run_m7.py perf`)

| item | S | Pruv | hand Rust | TS | Pruv/hand |
|---|---|---|---|---|---|
| attach | 100 | 21.2 | 56.4 | 8.1 | 0.38 |
| attach | 500 | 21.0 | 53.2 | 9.3 | 0.39 |
| attach | 1000 | 21.1 | 53.4 | 6.6 | 0.40 |
| current | 100 | 40.8 | 10.0 | 24.9 | 4.08 |
| current | 500 | 40.5 | 10.1 | 24.7 | 4.01 |
| current | 1000 | 41.0 | 9.9 | 24.9 | 4.14 |
| ishalted | 100 | 27.3 | 12.1 | 25.4 | 2.26 |
| ishalted | 500 | 27.4 | 10.0 | 24.8 | 2.74 |
| ishalted | 1000 | 27.7 | 9.9 | 25.1 | 2.80 |
| closereopen | 100 | 71.7 | 18.1 | 275.3 | 3.96 |
| closereopen | 500 | 71.0 | 17.6 | 288.7 | 4.03 |
| closereopen | 1000 | 71.6 | 17.7 | 273.8 | 4.05 |
| replace | 100 | 312.4 | 209.1 | 1134.8 | 1.49 |
| replace | 500 | 316.8 | 213.1 | 1156.2 | 1.49 |
| replace | 1000 | 318.0 | 208.6 | 1165.4 | 1.52 |
| dispatch (probe) | 100/500/1000 | 7.4 / 7.6 / 7.5 | – | – | – |
| rawcall (probe) | 100/500/1000 | 5.0 / 4.9 / 4.8 | – | – | – |

Only `attach` meets the 1.1x bar. The other four items do not; the causes are below. They were not fixed in this
milestone: two are in `bud_map` (owned by another worker), one is the result contract, one is the quiet protocol.

Probes: `dispatch` is the driver's request dispatch with an empty request (code 0); `rawcall` is one `sol::t_call`
with a prepared op. Together they are the fixed per-request host cost (~5–8 ns) that every Pruv item pays and the
hand baseline does not. In silent (perf) mode the driver caches the built op when (body ptr, len, quiet, next_uuid)
match and only rewrites `op[1] = now`, so op building is not in the numbers.

### Done in this milestone

- `current`: copy loop `out[2+k] = st[off+k]` after one bound check (generated as `get_unchecked`), was ~130 ns.
- `reopen`: when `ended.at` is a number (string 3 empty), the record is patched in place (state, how, at kind, at ms)
  instead of blob_copy → rewrite → store.
- `maybeClose`: same in-place patch when activeSince and ended.at strings are empty (`view_in`, settle_at, drop_at,
  then 7 header words). closereopen went ~300 → ~72 ns.
- `attach` ref path: reads state and id from the stored record in place (`put_str` from `st`), copies only on OPEN.
  ~67 → ~21 ns, now faster than hand (hand clones the id `String` and the session id).
- unit passed as `(op, 5)` instead of `slice(op, 5, 11)`: one heap alloc less per attach/replace.
- `put_str` / `id_is`: one range check, then direct indexing; fallback to `rd`/`put_range` otherwise.

### Line-by-line: current (4.1x)

hand (`hand_m7.rs` `World::current`):
```
match self.sessions.get(&r.session_id) {            // SipHash-1-3 of 5 bytes, one probe
    Some(Session { budget: Some(b) }) if b.id == r.budget_id => Some(b),   // memcmp 36 bytes
    _ => None }                                      // returns &Budget, no copy
```
generated (`work/m7/gen.rs`, `bud_lifecycle::current` → `current_entry` → `bud_map::find`):
```
hash_key(owner, og, key)          // FNV mix per u16 unit + gf(st, owner, 3) owner-generation read
loop: rd(st, slot_pos(st,h,i))    // checked rd on the slot word, slot_pos re-reads header words 3,4
  entry_matches(...)              // field_addr(st,e) + 5 header compares + unit compare loop
blob_at / blob_n                  // field_addr again + checked rd
state_of, id_is                   // state word + 36-unit compare (direct after one bound check)
wr(out,0,4); wr(out,1,n)          // checked writes
while k < n { out[2+k] = st[off+k] }   // ~130-word copy: the contract returns the record by value
```
Split (measured by the probes and ishalted): ~5–8 ns host call, ~15 ns `bud_map::find` (≈2x a std HashMap
lookup: FNV over u16 units plus owner/generation words plus `field_addr` per probe, versus SipHash over bytes and
one pointer compare), ~13 ns for the record copy. `ishalted` (27 ns, 2.7x) is the same path without the copy, so the
copy is the difference between the two. Not fixable inside M7: the lookup is in `bud_map` (not owned; record for
the bud_map owner), and the copy is the `current` contract (out = the budget). A borrowed-view contract would
remove it but changes the host interface.

### Line-by-line: closereopen (4.0x)

hand: `maybe_close` = one `get_mut`, one `clocks.get`, the `is_quiet` closure inline, field writes; `reopen` = one
`get_mut`, id compare, two field writes.
generated: `maybeClose` is two `t_call` round trips (first call returns status 5 to ask `isQuiet`, the host answers
and calls again — the spec's quiet protocol keeps the decision in Pruv), each with `bud_map::find` for the session
plus a `bud_clock` lookup of the budget's clock; then the in-place patch. `reopen` is one call with find + id compare
+ 4-word patch. So per close+reopen the Pruv side does 3 host calls and 4 map finds where hand does 3 HashMap ops
and no calls; 3 × ~6 ns + 4 × ~15 ns ≈ the measured 72×2 ns pair.

### Line-by-line: replace (1.5x)

hand: close fields in place, `mem::take` the id, build the new `Budget` struct, one `String` clone for follows, push
effects, then the bench puts back a cloned halted `Budget`.
generated: find + view + settle/drop (as CLOSE), `fresh` builds a new record vector (header + 6 strings) and
`rewrite`/store allocate the record through `bud_store` (alloc + copy), carryHeld pushes the old id units into fx,
then the host puts the halted record back through the store (`put` = alloc + copy of ~130 words). The extra time is
the record build and the store copy; hand's `Budget` has the same fields but `String`s are moved, not copied.

## §9 complexity (`docs/REQUEST_BUDGET_SPEC.md`)

- current / isHalted: O(key) excluding output ✓ (find is O(key) expected; current's copy is output size).
- attach / replaceForUserMessage: O(key + U) ✓ (U = unit / record size).
- maybeClose: O(key + U) ✓.
- reopen: fast path O(key) ✓. Bound note: when `ended.at` is stored text (only from imported records), reopen takes
  the slow path and rewrites the record, O(record) = O(U). Still inside O(key + U), but not O(key).

## Not implemented (left for later milestones)

- M8: charge (used counters).
- M10: halt, raise, decide.
- M11: hasCard, setCard.
- M12.

## Review

Codex (gpt-6-astra, ultra) reviewed bud_lifecycle.pruv. Findings and what was done:

1. P2 — OPEN clamped unit values to 2^40 (reproduced: delegations 1099511627777 stored as 1099511627776).
   Fixed: clamp raised to 2^53 − 1, see KD-M7-2.
2. P3 — the `maybe_close` fast path did not set `bv[1] = 2` (closed) the way `bud_clock::close` does on the slow
   path. Scratch frame only, not observed by the driver. Fixed: the fast path writes `bv[1] = 2` between SETTLE
   and DROP, the same order as `close`.
3. P3 — `maybe_close` wrote `out[0] = 1` before returning status 5 (Need), against the "nothing written" contract.
   Fixed: `out[0]` is written only on the returning paths after the quiet check.

Reviewed with no finding: reopen in-place patch, maybe_close settle/ended/timer/changed order, clock removal and
re-lookup, `put_str` bounds and fallback, `id_is` / `current` direct indexing, attach reuse, schedule follows /
carryHeld rules, fresh record init, the layout comment.

After the fixes: Q1 113 / 0 / 187 / 0 (Rust and Python pruvc), derived 300 exact, byte-identical (139485 bytes).
A perf rerun gave the same ratio band (the machine was loaded; TS and hand numbers rose with it), so the table
above is kept.

Post-fix check (Codex gpt-6-astra, ultra, static): all three fixes confirmed, no new defect. The early error returns
2/3 (before the session lookup succeeds) still write nothing to out, as before.
