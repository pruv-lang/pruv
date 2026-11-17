# L3-10 M7 — request budget lifecycle (bud_lifecycle.pruv)

Scope: `current`, `isHalted`, OPEN, `attach`, `replaceForUserMessage`, `reopen`, CLOSE / `maybeClose`.
All decisions are in Pruv (`bud_lifecycle.pruv`); the host (`m7_test/driver_m7.rs`) only runs side effects
(fx frame: 13 timer clear, 20 changed, 21 carryHeld, 22 uuid, 23 isQuiet consulted) and supplies the `isQuiet`
answer up front in the op (status 5 only when it is not known). Protocol rows: `ABI.md` "M7 test host protocol".
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

Byte identity: `run_m7.py same` → Python and Rust pruvc emit identical Rust (149080 bytes each, after the perf pass).

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

Before = this milestone's first commit (main at 41a6d0b); after = the perf pass. Machine load ~16 during both
after-runs, so hand numbers moved too; ratios are per run.

| item | S | before Pruv/hand | after Pruv | after hand | after Pruv/hand |
|---|---|---|---|---|---|
| attach | 100 | 0.38 | 35.3 | 57.7 | 0.61 |
| attach | 500 | 0.39 | 24.1 | 55.7 | 0.43 |
| attach | 1000 | 0.40 | 24.2 | 58.4 | 0.41 |
| current | 100 | 4.08 | 32.8 | 11.8 | 2.78 |
| current | 500 | 4.01 | 30.4 | 10.3 | 2.95 |
| current | 1000 | 4.14 | 29.5 | 10.0 | 2.95 |
| ishalted | 100 | 2.26 | 35.0 | 11.9 | 2.94 |
| ishalted | 500 | 2.74 | 29.4 | 10.7 | 2.75 |
| ishalted | 1000 | 2.80 | 30.1 | 10.2 | 2.95 |
| closereopen | 100 | 3.96 | 36.1 | 19.2 | 1.88 |
| closereopen | 500 | 4.03 | 32.2 | 18.9 | 1.70 |
| closereopen | 1000 | 4.05 | 32.5 | 18.4 | 1.77 |
| replace | 100 | 1.49 | 170.7 | 216.8 | 0.79 |
| replace | 500 | 1.49 | 137.5 | 214.5 | 0.64 |
| replace | 1000 | 1.52 | 135.0 | 218.0 | 0.62 |
| dispatch (probe) | 100/500/1000 | – | 9.5 / 8.2 / 8.0 | – | – |
| rawcall (probe) | 100/500/1000 | – | 5.8 / 4.9 / 4.9 | – | – |

attach and replace meet the 1.1x bar. current, ishalted and closereopen do not; the reason is the same for all
three (below) and is outside the files M7 owns.

Probes: `dispatch` is the driver's request dispatch with an empty request (code 0); `rawcall` is one `sol::t_call`
with a prepared op. Together they are the fixed per-request host cost that every Pruv item pays and the hand
baseline does not. In silent (perf) mode the driver caches built ops per (body ptr, len, quiet, next_uuid) and only
rewrites `op[1] = now`, so op building is not in the numbers.

### Done in this milestone

- `current`: returns a view `[4, n, off]` into `st` instead of copying the ~130-word record (protocol change,
  `ABI.md`). Was ~41 ns with the copy, ~130 ns before the first pass.
- `isQuiet` is supplied up front (op word 2) and consumed only when the effects hold `[23]`; `maybeClose` is one
  `t_call` instead of two. Status 5 stays for quiet = -1.
- Session entry field address is read once per call (`entry_addr` / `at_off` / `at_n` / `current_at`) instead of
  `blob_at` + `blob_n` each calling `field_addr`.
- `clock_in`: the clock map lookup is skipped when the clock map (owner 1) is empty (`bud_map::count`).
- `replace`: the halted record is closed and replaced without building the closed record (`blob_copy` →
  `close_rec` → store); `fresh` copies the old id from the stored record into
  follows, and carryHeld is written from that follows string. 1.5x → 0.6–0.8x.
- `reopen`: in-place patch when `ended.at` is a number; `maybeClose`: in-place patch when activeSince and ended.at
  strings are empty; `attach` ref path reads in place (first pass).
- Driver: per-body op cache in silent mode (test host only, not in Pruv's numbers before either).

Hand fairness, checked both ways: hand `attach` allocates (`sid.to_string`, id clone), which favours Pruv; hand
`clocks` is empty like Pruv's clock map; hand `replace` puts the halted budget back with `halted.clone()` (allocs),
Pruv puts it back with `t_put` (alloc + copy), comparable; hand `maybeClose` gets `isQuiet` as an inline closure, Pruv
now gets it in the same call. No hand change was needed.

### Line-by-line: current / isHalted (2.8–3.0x)

hand (`hand_m7.rs` `World::current`):
```
match self.sessions.get(&r.session_id) {            // hashbrown: SipHash of 5 bytes, SIMD 7-bit tag group
    Some(Session { budget: Some(b) }) if b.id == r.budget_id => Some(b),   // memcmp 36 bytes
    _ => None }                                      // &Budget, no call boundary
```
generated (`work/m7/gen.rs`):
```
t_call: bud_store::valid(st) + 3 key_end scans + 8 rd   // per-call entry checks (~5 ns, rawcall probe)
current_at -> session_of -> bud_map::find:
  hash_key(owner, og, key)        // FNV per u16 unit + owner generation read
  loop: rd(st, slot_pos(st,h,i))  // the slot word holds only the entry index
    entry_matches(...)            // field_addr(st,e) + hash/owner/len compares + unit loop, per probed slot
  field_addr once; state_of; id_is (36 units, direct after one bound check)
wr(out,0,4); wr(out,1,n); wr(out,2,off)
```
Split: host call ~5–8 ns (dispatch/rawcall probes), `bud_map::find` hit ~15–25 ns (probe: find-hit on owner 0 ≈ 25 ns
over dispatch, find-miss on the empty owner 1 ≈ 10 ns), the rest a few ns. Hand is ~10 ns in total. Why Pruv
cannot emit the hand code from M7: the call boundary is the test ABI (hand has none), and the lookup is
`bud_map::find`, whose index slot stores only the entry index, so every probed candidate costs an
`entry_matches` call that reads entry memory, where hashbrown rejects candidates on the 7-bit tag without touching
the entry. Both are in `bud_map` / the compiler (not owned here); see "Requests to other owners".

### Line-by-line: closereopen (1.7–1.9x)

hand: `maybe_close` = one `get_mut`, one `clocks.get` (empty map), inline `is_quiet`, field writes; `reopen` = one
`get_mut`, id compare, two field writes.
generated: `maybeClose` = one `t_call` (was two), `bud_map::find` for the session, clock lookup skipped (empty clock
map), in-place patch of 7 header words, fx `[23, 20]`; `reopen` = one `t_call`, find, id compare, 4-word patch,
fx `[20]`. Per pair: 2 host calls (~2 × 6 ns) + 2 finds (~2 × 15–20 ns) ≈ 65 ns = 2 × 32.5 ns. The remainder over hand
is the same call boundary + find cost as `current`; the patches themselves are a few ns.

### Requests to other owners

For the `bud_map` / `bud_store` / compiler owners (not edited in M7):

1. `bud_map` index: store hash tag bits in the slot word (e.g. `idx + 1 | (h >> 32) << 32`) so `find` rejects
   candidates without `field_addr` + `entry_matches`. This is the main remaining gap for current / isHalted /
   closereopen.
2. `bud_store::valid`: a cheaper check once per root call (it runs on every `t_call`).
3. `#[inline]` on small helpers (`rd`, `c32`, `c40`, `clamp`, `mix`, `field_addr`); the generated crate builds with
   `rustc -C opt-level=3` and default codegen units, and a `codegen-units=1` run was inconclusive under load.
4. Slice equality (`memcmp`) for the unit compare loops in `entry_matches` and `id_is`-style compares once bounds
   are proven.

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

Perf pass (view contract, quiet up front, replace without the closed-record build): a Codex review (role review) of
the protocol change was started; its result is recorded here when it arrives. Checks after the pass: Q1 113 / 0 /
187 / 0 (Rust and Python pruvc), derived 300 exact, byte-identical (149080 bytes).
