# L3-10 M8: activity accounting

Scope: `charge`, `noteFailure`, `noteTool`, `forgetTurn` of `RequestBudgets` (reference B:244–282), spec §4.3, §8, §9.

Files:
- `bud_charge.pruv`: charge decision and counter arithmetic on a charge view frame `cv`; `note_failure` and `add_tool` (the counting half of noteTool).
- `bud_tools.pruv`: the per-turn activity sets in the bud_map world (root 2: turnId → activityIds, both in insertion order). `note_tool`, `forget_turn`.
- `m8_test/root_m8.pruv`: test root (not product). It holds a PLACEHOLDER budget record in the session blob until M7 defines the record, runs CURRENT (session exists, has a budget, budgetId equals ref.budgetId) and dispatches the halt sub-operation frame.
- `m8_test/driver_m8.rs`, `run_m8.py`, `hand_m8.rs`, `perf_ts.mjs`: Q runner, derived set, perf.

All decisions are in Pruv. The host owns the buffers, grows on `-N`, and prints what Pruv returned. It never reads state words and never fills a missing path.

## Wire (P05: packed-u16 keys)

- Key = `n, packed words(n)`: n UTF-16 units (0..2^30) as m = ceil(n/4) i64 words; word i = `u[4i]<<48 | u[4i+1]<<32 | u[4i+2]<<16 | u[4i+3]` (two's complement, so `u[4i] >= 0x8000` gives a negative word); lanes past n are zero. Any word is 4 valid u16 lanes, so there is no per-unit range check; the only key check is O(1): n in 0..=2^30, the m words inside the op, tail lanes of the last word zero (canonical).
- put: `[sessionId key, len, record...]`, record = `[state, unlimited, haltReason, limits x4, used x4, tools, failures, retries, bn, packed budgetId words(bn)]`, so `len == 15 + ceil(bn/4)`.
- calls: `[1, sk, bk, what x10]` charge; `[2, sk, bk]` noteFailure; `[3, sk, bk, turnId key, activityId key]` noteTool; `[4, turnId key]` forgetTurn. Halt frame `H 20 k <op[1..e2) raw>` carries both keys in packed form.
- observe (`O ...`): map keys are printed unpacked (`n, units...`); a session blob is its raw record words, so its budgetId part is `bn, packed words`.
- Hand baselines (`hand_m8_u16`, `hand_m8_fx`, `hand_m8_oa`) decode the packed words at ingress (canonical check, then lane unpack into the reused scratch buffer) with algorithms unchanged and re-emit the id packed for observe/`chk`. `hand_m8.rs` (String keys) and `perf_ts.mjs` never touch the wire and are unchanged.

## Sub-operation interface

- `charge(inout cv, what, ws)` statuses: 0 charged `{ok:true}`; 1 `{ok:true}` with nothing charged (no current budget, or closed); 16+r `{ok:false, reason:r}` for a halted budget; 32+k over limit on key k (1..4 = delegations, commands, reviews, checks), nothing charged; 5 ResourceBound; 2 invalid frame.
- On 32+k the root returns status 6 with the halt frame `[20, k, sessionId key, budgetId key]` = `halt(ref, k)`. The halt body is M10, so the root stops there (not-implemented). After M10 runs the halt, the answer is `{ok:false, reason:k}`.
- `note_tool` statuses: 0 already seen (returns before CURRENT), 1 inserted (caller runs CURRENT, then `add_tool`), −N Grow, 2 invalid. The turn entry is created with its first activity, so a turn set is never left empty in the store; TS creates the set before the `has` check, which is the same observable state.
- `forget_turn`: 1 removed, 0 absent.

## Behaviour classification (spec §0, §15)

| behaviour | class | M8 |
|---|---|---|
| over-limit charge charges nothing and halts with the first key in BUDGET_COUNT_KEYS order | intent | kept, oracle-compared |
| limit is inclusive (`used+amount > limit` refuses) | intent | kept |
| unlimited budget is charged but never halts | intent | kept |
| `what[key] ?? 0`; null and undefined both count 0; negative amounts are added and never halt | intent (as written in B) | kept |
| §15 #4–#5 (closed / missing budget answers `{ok:true}`) | keep | kept |
| §15 #6 a missing or stale budget still consumes the noteTool dedup key | keep (chosen accident) | kept, compared via finalToolSeen |
| §15 #7 noteFailure / noteTool count on halted and closed budgets | keep (chosen accident) | kept |
| §15 #27 halted budget without `halt` answers reason `'delegations'` | excluded row | code 17 kept; Q2 never generates it (halted budgets always have `halt.reason`) |
| §15 #26 | excluded row | not reachable from M8 calls |

Known differences (not compared, recorded):
- **KD-M8-1** ResourceBound. TS adds IEEE doubles; once a counter leaves ±(2^53−1) it silently loses precision. Pruv returns status 5 and writes nothing (charge is atomic: all sums are checked before any write). Q2 values never get there.
- **KD-M8-2** Non-integer amounts (e.g. `0.5`, `NaN`, `Infinity`) are not representable in the i64 frame. Q2 generates integers only. A future boundary must reject or map them; nothing in M8 fills them.
- **KD-M8-3** One halt nested in charge: TS calls `deps.now()` inside halt. That belongs to the halt body (M10); the frame M8 emits carries no time.

## Not-implemented paths

- the halt body after an over-limit charge (M10): the frame is checked (see below);
- `reopen`, `halt` (M10), `current`, `isHalted`, `hasCard` (M7 / M11): the case is cut at the first such call.

## Q results

`python3 bench/l3/budget/m8_test/run_m8.py q` (Rust pruvc root, driver_m8):

| set | cases | exact | not-implemented | known-difference | fail |
|---|---|---|---|---|---|
| Q2 seed 20261117 (`work/m8/cases.jsonl`) | 300 | 63 | 237 | 0 | 0 |
| accounting-only derived (`run_m8.py derive`, `work/m8/cases_acct.jsonl`) | 300 | 222 | 78 | 0 | 0 |

Not-implemented reasons, Q2: reopen call 83, halt call 53, halt dispatch (M10) 46, hasCard call 21, isHalted call 18, current call 16.
Derived: halt dispatch (M10) 78; every other call is an M8 method.

Compared calls: Q2 charge 608, noteTool 285, forgetTurn 136, noteFailure 117; derived charge 1038, noteTool 498, noteFailure 213, forgetTurn 207.

Per exact case: every return value, plus every final session's budget presence, state, halt reason, the seven counters, and toolSeen (turn order and activity order).
Per halt dispatch (46 + 78, all checked): the request's oracle value is `{ok:false, reason:k}` with the frame's k; the oracle's halt ran for the frame's ref (`runningDelegationIds(ref)`); the counters at the dispatch equal the oracle's at the halt's `findSession` (nothing charged). The prefix before every cut matched.

The derived set is the same world and script without reopen / halt / current / isHalted / hasCard, re-run through the TS oracle (ids end in `-acct`).

## Compiler parity

Python `pruvc/pruvc.py … --backend rust` and the Rust `pruvc` emit byte-identical Rust for `m8_test/root_m8.pruv` (`cmp`; sha1 `13202593…`, 107183 bytes, after the perf pass).

## Perf

`run_m8.py perf --rounds 5` (median ns per call, interleaved rounds). Items:
- `charge`: `charge(ref_i, {commands:1, retries:1})` round-robin over N sessions;
- `failure`: `noteFailure(ref_i)` round-robin;
- `tooldup`: `noteTool` on a turn holding N activities, dedup hit;
- `toolgrow`: N new activities on one turn, then `forgetTurn`; ns per noteTool, forget included.

Perf pass (L3-10 M8 pass 2). Before = the M8 commit, load average 17.98→17.59; after = this pass, load average
5.40→6.45. Host load differs between the two runs, so compare the Pruv/hand ratio and the instruction counts below,
not raw ns across runs.

| item | size | before Pruv / hand / TS ns | before ratio | after Pruv / hand / TS ns | after ratio |
|---|---|---|---|---|---|
| charge | 100 | 91.7 / 30.8 / 221.8 | 2.98 | 37.2 / 15.8 / 133.5 | 2.35 |
| charge | 500 | 92.4 / 42.7 / 302.5 | 2.16 | 44.0 / 18.5 / 135.6 | 2.38 |
| charge | 1000 | 97.4 / 41.6 / 259.2 | 2.34 | 43.6 / 17.0 / 141.8 | 2.56 |
| failure | 100 | 87.4 / 31.2 / 51.9 | 2.80 | 26.4 / 13.9 / 29.0 | 1.90 |
| failure | 500 | 77.2 / 18.5 / 46.0 | 4.17 | 34.3 / 15.0 / 33.7 | 2.29 |
| failure | 1000 | 69.1 / 19.8 / 43.5 | 3.49 | 33.4 / 15.3 / 33.3 | 2.18 |
| tooldup | 100 | 64.1 / 36.2 / 22.1 | 1.77 | 34.8 / 26.7 / 12.3 | 1.30 |
| tooldup | 500 | 73.6 / 35.5 / 35.5 | 2.07 | 47.9 / 25.0 / 13.5 | 1.92 |
| tooldup | 1000 | 45.6 / 27.7 / 39.4 | 1.65 | 46.6 / 25.9 / 14.4 | 1.80 |
| toolgrow | 100 | 173.2 / 125.7 / 113.1 | 1.38 | 95.1 / 79.3 / 58.9 | 1.20 |
| toolgrow | 500 | 178.8 / 116.8 / 102.8 | 1.53 | 113.1 / 86.2 / 58.2 | 1.31 |
| toolgrow | 1000 | 188.6 / 124.5 / 123.0 | 1.51 | 116.3 / 86.0 / 59.5 | 1.35 |

Load-robust measure: instructions retired per call (`/usr/bin/time -l`, difference of two rep counts, item-gated
driver builds, N = 1000):

| item | Pruv before | Pruv after | hand Rust | after/hand |
|---|---|---|---|---|
| charge | 1358 | 863 | 351 | 2.46 |
| failure | 1181 | 736 | 287 | 2.56 |
| tooldup | 836 | 835 | 462 | 1.81 |
| toolgrow | 2650 | 2206 | 1739 | 1.27 |

**No row reaches 1.1x.** Charge and failure lost ~37% of their instructions; tooldup is unchanged (its path is
almost all `bud_map`); toolgrow lost the `load_cv` share. Pruv beats TS on charge (~3x) and ties it on failure;
TS (V8 caches string hashes) still beats Pruv on tooldup and toolgrow.

### Fairness

- Lookup count matches hand: instrumented `find` = 1 per charge / failure, 2 per tooldup (turn, activity), with
  1.07–1.2 probes per find. Hand does 1 `HashMap` lookup per charge / failure and 2 per tooldup.
- Both sides use `black_box` on inputs, same warm-up (1000 charge calls), same sizes and round-robin order.
- The Pruv side also pays the test-root boundary rule (`bud_store::valid` on every `t_call`) and the PLACEHOLDER
  blob record (copy into `cv`, write counters back); hand has neither (a struct field). Both are kept: the first is
  the store's safety contract, the second is M8's frame until M7 defines the record.

### What this pass changed (M8 files only)

- `load_cv`: one range check (`off + n <= len(st)`, `bs + bn <= len(bk)`, `len(cv) >= 15`), then plain proven
  indexing; the budget-id compare is an OR-accumulate (`d |= st[..] ^ bk[..]`, no early exit) that LLVM vectorizes;
  the 14-word copy is plain indexing. The old `rd`/`wr` path stays as the fallback (same tests, same words).
- `load_cv` returns the blob offset; `store_cv` writes counters at that offset directly instead of re-deriving it
  from the session entry (`field_addr` + header reads again; the state does not change between the two). Fast path
  for the 7 writes, total fallback.
- `charge`: amounts read once (`a0..a4`) with direct indexing (`amount` gets a `requires` for the 10-word range);
  the three loops (check / validate / write) are unrolled; still atomic (ResourceBound: nothing written unless
  every sum passes, KD-M8-1).
- `bump`, `t_call`, `key_end`: direct indexing where the range is already checked.

### Remaining gap, line by line (failure, N = 1000: 736 vs 287 instructions)

| step | hand Rust (`hand_m8.rs`) | generated Rust | approx. cost | owner |
|---|---|---|---|---|
| boundary check | none | `bud_store::valid` header check per `t_call` | ~80 instr (failure 736 → 651 with `valid` stubbed to `true`) | M4 store / boundary rule |
| session lookup | SipHash 13 bytes + SwissTable probe + `memcmp` | `bud_map::find`: `hash_key` one dependent multiply per u16 unit, probe re-reading header words, `entry_matches` per unit | the bulk of the rest (~40% of failure samples are `load_cv` with `find` inlined, ~20% `valid`, then `hash_key`) | M4 `bud_map` (not editable here) |
| CURRENT id compare | `memcmp` 36 bytes | vectorized OR-accumulate over 36 words | small now | M8 (done) |
| record access | struct field | `field_addr` + copy 14 words, write 7 back | ~20–30 instr | PLACEHOLDER record until M7 |
| call overhead | everything inlined | separate `fn`s for `load_cv` / `store_cv` / `charge` / `key_end` / `find` | 25–60 instr (all generated fns `#[inline]` in a scratch copy) | compiler (no inline hints) |

charge = failure + ~130 instructions of limit work (5 amount reads, 4 limit compares, 5 safe sums, 5 writes) vs ~65
for hand's `Option` loop; the extra is the safe-integer checks of `sum` (ResourceBound), which hand skips.
tooldup: 2 × `find` (+ `hash_key` each) + `valid` vs 2 SipHash lookups; nothing M8-owned is left on that path
(`note_tool` is ~30 instructions). toolgrow: the dedup `find` misses, then `bud_map::add` hashes the key again
(no "insert at the slot `find` stopped at" API in `bud_map`), plus `make_room` / free on forget, vs `String` alloc
+ set insert.

Next steps (outside M8's files): the M9 `bud_map::hash_key` / `entry_matches` change (4 units per mix, slice
compare); a `find`-returns-slot / `add_at` API so `note_tool` hashes once; a cheaper per-call `valid` (or one check per
batch); inline hints from the compiler; after M7, read the counters in place instead of copying `cv`.

### Perf pass 3 (L3-10 M8, after the compiler `#[inline]` / `eq_range` pass)

Measurement first (`40f83aa`): `run_m8.py perf` now runs each item in its own process (`driver_m8` / `hand_m8` take an
item name; setup always runs), rotates the order per round, takes 7 rounds and adds an instruction column
(`/usr/bin/time -l`, (instr(1M reps) - instr(200k reps)) / 800k, so setup cancels). Instructions are the load-robust
column; ns of the "before" run were taken at load average 18-22 and are not comparable.

Fix (`159a1be`): `note_tool` hashed the activity key twice on a new activity (`find`, then `add`). `bud_map` gets
`find_h` / `put_key_h` (the old bodies, taking the hash) and `add_absent(st, owner, key, ..)`: one `hash_key`, then
probe, `make_room`, insert with the same hash (rehash rewrites only the index, so the hash stays valid). Codes:
0 present, 1 added, -N Grow, 2 invalid. `find` / `add` / `insert` keep their API and behaviour (M7 / M10 untouched).

After, load average ~4.3, 7 rounds, median:

| item | size | Pruv ns | hand ns | TS ns | Pruv/hand | Pruv/TS | Pruv instr | hand instr | instr ratio | instr before |
|---|---|---|---|---|---|---|---|---|---|---|
| charge | 100 | 33.7 | 16.0 | 127.7 | 2.11 | 0.26 | 817 | 351 | 2.33 | 821-824 |
| charge | 500 | 40.4 | 17.3 | 132.2 | 2.34 | 0.31 | 825 | 351 | 2.35 | |
| charge | 1000 | 40.9 | 16.8 | 137.7 | 2.43 | 0.30 | 825 | 350 | 2.36 | |
| failure | 100 | 25.4 | 13.5 | 28.5 | 1.88 | 0.89 | 688 | 286 | 2.40 | 693-697 |
| failure | 500 | 31.2 | 14.5 | 31.4 | 2.15 | 0.99 | 696 | 284 | 2.45 | |
| failure | 1000 | 31.7 | 15.4 | 32.8 | 2.06 | 0.97 | 697 | 284 | 2.45 | |
| tooldup | 100 | 27.4 | 26.0 | 12.0 | 1.05 | 2.28 | 674 | 461 | 1.46 | 751-767 |
| tooldup | 500 | 39.7 | 24.3 | 12.8 | 1.63 | 3.10 | 706 | 460 | 1.53 | |
| tooldup | 1000 | 39.3 | 24.4 | 13.9 | 1.61 | 2.83 | 699 | 460 | 1.52 | |
| toolgrow | 100 | 83.6 | 76.9 | 53.7 | 1.09 | 1.56 | 1853 | 1643 | 1.13 | 2069-2085 |
| toolgrow | 500 | 99.8 | 83.0 | 56.8 | 1.20 | 1.76 | 1875 | 1752 | 1.07 | |
| toolgrow | 1000 | 101.9 | 83.1 | 57.8 | 1.23 | 1.76 | 1873 | 1737 | 1.08 | |

The hash-once fix took toolgrow from ~2075 to ~1870 instructions (-10%) and tooldup from ~760 to ~700 (the dedup
hit now hashes and probes through one frame). Charge and failure do not go through `note_tool` and did not move.
Tests: m7 q / derive / same, m8 q / acct, m9, m34 m3 / m4, m11 pairs, all green; Python and Rust `pruvc` output
identical.

Attribution, failure N = 1000 (694 instructions; scratch builds in which one piece is stubbed, plus counted code):

| step | hand Rust | generated Rust | instr | why Pruv cannot emit the hand code today |
|---|---|---|---|---|
| driver | loop, `black_box` | loop, `black_box`, op build | ~14 | same on both sides |
| boundary check | none | `bud_store::valid(st)` per `t_call` (header words, index geometry) | ~70 | the root takes an untrusted `[i64]`; only the typed-handle ABI in `perf_headroom/HOST.md` can drop it (language / ABI change) |
| op parsing | typed `&Ref` | `key_end` x2, length checks on the flat op | ~68 | the test root's wire format is a flat `[i64]`; hand gets a struct |
| lookup frame | SipHash + SwissTable probe | `find` frame, owner gen, slot probe | ~80 | open-addressing index over the i64 store |
| hash | SipHash over 13 bytes | `hash_key` over 13 i64 units (pack4 + mix + fin) | ~100 | keys are one i64 per UTF-16 unit (8x the bytes of hand's UTF-8) |
| key compare | `memcmp` 13 bytes | `entry_matches` + `eq_range` (`memcmp` 104 bytes) | ~90 | same: 8x wider key words |
| CURRENT budget id | `memcmp` 36 bytes (`String ==`) | OR-xor over 36 i64 words | ~110 | 36 words vs 36 bytes; `eq_range` (`memcmp` 288 bytes) measured worse (749 vs 694) |
| record + rest | struct field `+= 1` | `field_addr`, blob checks, 14-word copy, `note_failure`, 7-word store | ~130 | PLACEHOLDER record until M7; stubbing the copy saves ~0 (it is not the cost), the checks are |

Experiments that did not help (scratch, not committed): `eq_range` for the bid (749, worse); xor-accumulate or
early-exit loop in `entry_matches` (no change); `#[inline(always)]` on loop-free fns only (no change).
`#[inline(always)]` on every generated fn (compiler scratch) gives failure 587 (-107), charge 697 (-128), tooldup
622 (-139): real, but a compiler-wide change that needs its own M11 compile-time / binary-size evaluation, and it
still leaves failure at ~2.1x. Not committed in this pass.

Rows still above 1.1x, and why:
- charge / failure (2.3-2.45x instr): the table above. The gap is the i64-per-unit key and id width (hash, compare,
  bid ~300 instr), the `valid` boundary check (~70) and call frames (~110 with `inline(always)`). None is in an M8 file
  except the PLACEHOLDER record, which costs ~0 measured.
- tooldup (1.5x instr, and slower than TS): two `find`s over i64-unit keys plus `valid`, vs two SipHash lookups;
  V8 keeps a cached hash on each string, so TS hashes nothing per call.
- toolgrow (1.07-1.13x instr, 1.09-1.23x ns): within 1.1x on instructions at n = 500 / 1000; the ns gap at larger n
  is the 8x wider key words in the store (cache footprint), same cause as above.

Next steps (outside this pass): compiler `#[inline(always)]` with an M11 evaluation; the typed-handle ABI to drop
`valid`; a byte-packed key layout in `bud_map` (the M5 store already moved to byte keys and reached parity).

### Inline policy (compiler, after perf pass 3)

The compiler now chooses `#[inline(always)]` per fn (SPEC section 7, "Inline policy"). A fn gets it when it is
non-recursive and one of these holds: `incl <= 8`, at most one call site, or `incl * (call sites - 1) <= 1500`. The
same text pass runs in both compilers. Parity holds byte for byte on all budget roots: m7, m8, m9, m11, m34 m3 and m4.

How the policy was chosen: each candidate rewrote the hints in the generated Rust and was measured on M8, M7, M9,
M4 (m34) and M11. Instruction counts per op are `/usr/bin/time -l`, (instr(hi reps) - instr(lo reps)) / delta, which
cancels setup. During the runs the load average was 14-51, so ns are noisy and instructions decide. Candidates:
- `all`: `#[inline(always)]` on every non-recursive fn.
- `size:N`: the same, limited to fns with at most N body lines.
- `single`: fns with one call site.
- `or:40`: size at most 40 or one call site.
- `incl:G`: the chosen rule, with budget G.

Instructions per op (Pruv; hand Rust in the last row):

| candidate | M8 charge 500 | M8 failure 500 | M8 tooldup 500 | M8 toolgrow 500 | M7 500 | M9 10 | M4 sessions 100 / 1000 | M4 tools 100 / 1000 | M11 B 1000 |
|---|---|---|---|---|---|---|---|---|---|
| before (`#[inline]`) | 825 | 697 | 710 | 1876 | 8340 | 20728 | 6712 / 6830 | 10097 / 11064 | 16.29M |
| all | 698 | 583 | 613 | 1661 | 8250 | 20942 | 6394 / 6396 | 9796 / 10676 | 15.16M |
| size:20 | 814 | 683 | 708 | 1830 | 8539 | 20540 | | | |
| size:40 | | | | | 8252 | 22320 | | | |
| size:60 | 783 | 650 | 623 | 1685 | 8219 | 20912 | | | |
| single | 812 | 696 | 705 | 1873 | 8500 | | | | |
| or:40 | 824 | 703 | 705 | 1791 | 8625 | 22309 | | | |
| incl:200 | 812 | 697 | 734 | 1888 | 7996 | 20371 | | | |
| incl:600 | 752 | 637 | 661 | 1768 | 7895 | 20573 | 6507 / 6602 | 9704 / 10700 | 15.59M |
| **incl:1500 (chosen)** | 698 | 585 | 611 | 1680 | 7629 | 20431 | 6323 / 6547 | 9784 / 10724 | 15.29M |
| incl:5000 | | | | | 8301 | | | | |
| hand Rust | 350 | 285 | 460 | 1752 | ~7805 | ~19049 | | | |

The M7 column is the instruction cost of one `perf` op mix at S = 500. The other M7 sizes moved the same way: 7634
and 7633 with the chosen policy. M9 is `perf` at size 10. Empty cells were not measured, because the candidate had
already failed on M7, M8 or M9.

rustc cost of the chosen policy against before. Each build was compiled 5 times, base and policy alternating; the
table shows min child CPU in seconds and the binary size:

| driver | before cpu | policy cpu | before bytes | policy bytes |
|---|---|---|---|---|
| M8 | 0.77 | 1.14 | 577344 | 592656 (+2.7%) |
| M7 | 0.97 | 2.28 | 594864 | 658320 (+10.7%) |
| M9 | 1.14 | 1.35 | 599280 | 614816 (+2.6%) |
| M4 | 0.85 | 1.53 | 575024 | 590368 (+2.7%) |
| M11 | 1.56 | 2.71 | 629152 | 693456 (+10.2%) |

For comparison, `all` took 7.3 s for M7 and 6.8 s for M11, and its binaries were 823 KB and 875 KB. `incl:5000`
took 6.7 s for M7, its binary was 807 KB, and its M7 instructions went back to 8301.

Why `incl:1500`:
- The M8 gain needs whole call trees inlined into the root's entry and the driver loop. Size limits stop at the
  first large frame, and `single` misses the shared helpers. The inclusive-size budget takes the tree as long as the
  copies it adds stay bounded.
- It improves every measured piece: M7 -8.5%, M9 -1.4%, M4 -3 to -6%, M11 B -6% and C -4%.
- Its compile time and binary growth are a fraction of `all`'s.
- Size and `or` rules were non-monotonic. size:40 and or:40 made M9 7.7% worse. The cause is rustc's own choices
  shifting when some frames are forced and others are not.

M8 after the policy (load average ~7-50, 7 rounds, median; instructions are the stable column):

| item | size | Pruv ns | hand ns | Pruv/hand | Pruv instr | hand instr | instr ratio | instr before |
|---|---|---|---|---|---|---|---|---|
| charge | 100 / 500 / 1000 | 61.3 / 53.1 / 59.2 | 37.9 / 40.4 / 45.0 | 1.62 / 1.31 / 1.32 | 695 / 698 / 697 | 350-352 | 1.97-1.99 | 817-825 |
| failure | 100 / 500 / 1000 | 23.6 / 29.7 / 32.8 | 14.0 / 14.6 / 15.9 | 1.69 / 2.03 / 2.06 | 581 / 585 / 584 | 284-286 | 2.03-2.05 | 691-697 |
| tooldup | 100 / 500 / 1000 | 25.3 / 36.1 / 38.8 | 27.5 / 24.3 / 25.0 | 0.92 / 1.49 / 1.55 | 596 / 611 / 607 | 460-461 | 1.29-1.33 | 676-710 |
| toolgrow | 100 / 500 / 1000 | 106.1 / 101.7 / 116.0 | 107.6 / 97.6 / 107.5 | 0.99 / 1.04 / 1.08 | 1666 / 1680 / 1679 | 1643-1752 | 0.96-1.01 | 1859-1876 |

Toolgrow is now within 1.1x on both ns and instructions. Charge, failure and tooldup are still above 1.1x. The
policy removed the call-frame share, about 110-130 instructions. What is left is the key and id width and the
`valid` boundary, as listed in the table above.

## Complexity bound check (§9)

Expected normal path; Grow (host copy, rare) and rehash (amortized) are storage upkeep.
- charge: O(sessionId + budgetId length): one `find`, the bid compare, constant field work. Measured flat in N: 63.6 / 76.1 / 72.0 ns at N = 100 / 500 / 1000.
- noteFailure: O(key length); flat: 53.4 / 65.1 / 65.9 ns.
- noteTool: O(turnId + activityId + ref key lengths), no scan of the turn's activities: dedup hit on a turn of N activities is flat: 50.9 / 65.8 / 55.4 ns.
- forgetTurn: O(turnId + A): `bud_map::remove` frees the A children. Measured inside toolgrow as a per-noteTool cost that stays flat (135.9 / 155.6 / 155.9 ns), so forget is linear in A, not worse.
