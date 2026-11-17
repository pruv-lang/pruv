# Perf headroom, part A: host and resident-state path of the request-budget piece

Bar (AGENTS.md): equivalent hand-written idiomatic Rust. Every item below has a same-work hand baseline in
`perf_headroom/host/` and was measured Pruv / hand Rust / TS in alternation.

## Result

ns per op. Medians of 5 alternating rounds on Apple silicon under high load (load average 3.5–4.3). The rounds rotate
the order of the three sides.

- M4: each run is 200000 reps of the op cycle: 400000 ops for sessions and clocks, 600000 for tools.
- M5: each run is the minimum of 5 reps after one warm-up, like `harness perf`.

| item | n | Pruv ns | hand Rust ns | TS ns | Pruv/hand | scope | cause | status |
|---|---:|---:|---:|---:|---:|---|---|---|
| M4 sessions (get + put) | 100 | 19.0 | 13.6 | 54.4 | 1.40 | one `m4_op` call through the driver's `World::exec`: output capacity check, decode + validate op words, keys from units, map op, result words into `out`, `black_box` on the result | word-buffer layout plus the op-decode guards left in `m34_test/root_m4.pruv` (profiled below) | within bar (was 1.60; source fast paths, see profile) |
| M4 sessions | 500 | 19.7 | 15.1 | 59.5 | 1.30 | same | same | within bar (was 1.52) |
| M4 sessions | 1000 | 19.7 | 15.1 | 56.7 | 1.30 | same | same | within bar (was 1.51) |
| M4 tools (note seen, note new, forget) | 100 | 35.8 | 51.7 | 59.0 | 0.69 | same | — | within bar |
| M4 tools | 500 | 40.1 | 53.7 | 60.8 | 0.75 | same | — | within bar |
| M4 tools | 1000 | 39.2 | 53.1 | 57.6 | 0.74 | same | — | within bar |
| M4 clocks (run_set + run_del) | 100 | 37.8 | 38.5 | 101.6 | 0.98 | same | — | within bar |
| M4 clocks | 500 | 34.1 | 39.5 | 120.1 | 0.86 | same | — | within bar |
| M4 clocks | 1000 | 38.4 | 39.3 | 94.4 | 0.98 | same | — | within bar |
| M5 store_find (lookup + JV encode as text) | 100 | 67.1 | 69.6 | 671.3 | 0.96 | `find_session_text(&str, &mut String)`: id lookup, first-encode labelling, the JV frame `{"sref","v":{"obj":[..]}}` written as text into a reused `String` | was the owned J tree (~12 allocations) | fixed (was 365 / 1.27 vs tree, 5.7 vs text) |
| M5 store_find | 500 | 78.0 | 77.3 | 1620.3 | 1.01 | same | same | fixed (was 434 / 5.8 vs text) |
| M5 store_find | 1000 | 80.4 | 77.2 | 3392.0 | 1.04 | same | same | fixed (was 394 / 5.1 vs text) |
| M5 store_find_enc (encode only) | 100 | 30.8 | 35.4 | — | 0.87 | `Heap::enc_into` of a found, labelled session into a reused `String` (hand `store_find_enc_text`). Labelling and lookup are untimed on both sides | was tree build + drop | fixed (was 1.11 vs tree) |
| M5 store_find_enc | 500 | 37.0 | 38.8 | — | 0.95 | same | same | fixed (was 1.21) |
| M5 store_find_enc | 1000 | 39.2 | 38.5 | — | 1.02 | same | same | fixed (was 1.15) |
| M5 store_find_lookup (string-id lookup helper) | 100 | 11.2 | 11.7 | — | 0.96 | `find_session(&str)` only: index probe on the id, handle from the index entry. It is not the whole non-trace `exec_item "find"` | was the UTF-16 key build and the `stored[pos]` read for the handle | fixed (was 1.18) |
| M5 store_find_lookup | 500 | 9.6 | 12.7 | — | 0.76 | same | same | fixed (was 1.21) |
| M5 store_find_lookup | 1000 | 10.2 | 13.0 | — | 0.78 | same | same | fixed (was 1.29) |
| M5 store_save | 100 | 130.4 | 136.2 | n/a | 0.96 | `scan(program JV)`: label counter walk, sref resolve, budget pair, budget decode, set/append `budget` | was `u16s` growing its vector several times, and text labels | fixed (was 1.30) |
| M5 store_save | 500 | 128.6 | 128.9 | n/a | 1.00 | same | same | fixed (was 1.35) |
| M5 store_save | 1000 | 128.2 | 129.8 | n/a | 0.99 | same | same | fixed (was 1.32) |

M5 rows are medians of 7 alternating rounds (`host/run.py --only m5 --rounds 7`). Before the 1.1× pass (same runner,
n=1000): store_find 414, store_find_lookup 16, store_find_enc 299 (tree), store_save 174. Before the first fixes:
store_find 405–466, store_find_lookup 33, store_save 310–330.

How to read the comparison rows:

- The TS `store_find` is `deps.findSession` on the oracle world. It is a linear `sessions.find` with no encode, so
  it is the TS counterpart of a lookup, not of the encode.
- TS has no `store_save` measure (a TS save is a dep record on a shared object).
- The `store_find` hand column is `store_find_text`: hand code that writes the frame as JSON text into a reused
  `String`, including the lookup and first-encode labelling, like the Pruv side. The tree measures are gone: the host
  has no tree encoder any more.
- `store_find_lookup` against the hand `&str`-key core (`store_find_core`) is 1.17 / 1.22 / 1.24. That core is not the
  same work (see the breakdown); the paired hand core is `store_find_core_u16`. That pair is close, not identical:
  the hand core converts the id to UTF-16 units for its `HashMap<Vec<u16>,_>` and returns `&Session`, while the host
  probes a byte key and returns a handle.

## M4 sessions: profile (was the one item over 1.5×)

Before: 1.51–1.60× hand Rust. Cause: the `ent_u16::rd`/`wr` total accessors, which check `0 <= i < len(a)` on every
state-word access (~23% of the time; 1.17–1.24× with the guards removed in a scratch copy). The sessions op is almost
only state-word reads and writes (map probe, key compare, blob copy in and out). The other axes spend more of their
time in key decoding and map work.

Fix (source only; compiler, lib and host unchanged). The state-word indices come from buffer contents, so no single
access can be proven in range. Instead each hot run of accesses gets one range check up front. Inside it, plain
`a[i]` is proven and compiles to `get_unchecked`. Outside it, the original total-accessor code runs, so results are
the same for every buffer, including corrupt or short ones:

- `bud_map`: `hash_key`, `entry_matches` (entry fields and key compare), `write_at` (blob copy in), `slot_pos`,
  `fits_region`; new `blob_off`.
- `bud_world`: new `blob_out` (blob copy out for `session_get` / `clock_get`); `ret_one` and the `session_get`
  header words use plain writes after the existing `len(out)` checks.
- `bud_store`: `ea` and `need` (one `len(st)` check, then plain header reads); `valid` reads the header plainly after
  its `len(st) >= 232` check (LLVM had already folded these, so no measurable change there).

Instructions per op (retired-instruction difference between 100k and 500k reps, median of 5, scratch drivers that
encode one cycle):

| n | before | after | after, remaining guards removed | hand |
|---:|---:|---:|---:|---:|
| 100 | 739 | 645 | 587 | 401 |
| 1000 | 768 | 668 | 608 | 418 |

Wall time (`host/run.py --only m4 --rounds 7`, two runs, same both times): 1.40 / 1.30 / 1.30× at n = 100 / 500 /
1000.

What is left:

- About 58 instructions per op of guards remain. Most are op decoding in `m34_test/root_m4.pruv` (`key_end` reads
  each key unit through `rd`, about 7.5 guarded `op` reads per op), plus single reads in `find` (`gf(owner, 3)`,
  header word 4, the slot word) and the `gf`/`sf` field accesses in `set_blob` / `put_units`.
  - `root_m4.pruv` was out of scope for this change.
  - The same pattern applies there: `key_end` already checks `kn <= n - p - 1`, so its loop can read `op[p + 1 + j]`
    directly.
  - Expected: about −20 instructions per op, which should bring n = 100 to about 1.3×.
- The rest is the word-buffer layout (open addressing over i64 words with a per-unit key compare) against
  `HashMap<Box<[u16]>, Vec<i64>>`.
- Hand numbers moved from 16–18 to 13.6–15.1 ns after the audit aligned the hand loop with the driver loop: one
  measured run, no separate warm-up runs, the same per-op capacity check, and `black_box` on the result. Before that,
  the sessions ratio was 1.30–1.37×.

### M4 sessions under the 1.1x bar (2026-11-17)

More source fast paths, same pattern (one range check, plain proven indexing, the old total code as the fallback;
public signatures unchanged):

- `root_m4.pruv`: `key_end` reads `op[p]` and `op[p + 1 + j]`; `dec_key` / `dec_put` read `op[1]` / `op[e1]`.
- `bud_map::find`: reads slot base and mask once and the slot word directly. It no longer calls `slot_pos` and
  `rd` per probe.
- `bud_map::set_blob`: one `field_addr`. When the old region takes the blob and lies apart from the header and the
  fields, it does the in-place copy and the field 13 / word 14 updates with plain indexing.
- `bud_map::blob_fits`: one `field_addr`, then fields 12 and 19 read directly.
- `bud_world::session_get`: one `field_addr` for fields 12 and 13, then the region copy.

Wall time (`host/run.py --only m4 --rounds 7`), ns per op:

| n | Pruv | hand | TS | Pruv/hand (was) |
|---:|---:|---:|---:|---:|
| 100 | 17.0 | 13.9 | 55.4 | 1.22 (1.40) |
| 500 | 17.8 | 15.2 | 59.6 | 1.17 (1.30) |
| 1000 | 17.6 | 15.2 | 58.0 | 1.16 (1.30) |

**Still above 1.1x.** The remaining gap has two causes, separated with scratch builds of the generated
`work/m34/gen_m4.rs` (`/tmp`, not in the repo; same driver; 7 alternating runs, then 5):

| build | n=100 | n=500 | n=1000 |
|---|---:|---:|---:|
| generated as is | 1.25 | 1.19 | 1.17 |
| every generated `fn` given `#[inline]` | 1.15 | 1.06 | 1.04 |
| plus `valid()` short-circuited to `true` | about 0.98 | - | about 0.94 |

1. **Inlining (compiler).**
   - The generated code is a chain of plain `fn`s with no inline hint. One put goes
     `m4_op -> dec_put -> session_put -> put_blob -> fd -> find -> {gf -> ef -> ea, hash_key, entry_matches ->
     field_addr -> ea} -> blob_fits -> make_room -> need -> set_blob -> field_addr -> ea -> fits_region`.
   - rustc keeps most of these as real calls. In the hand version, `HashMap::get_mut` and `decode_key` are inlined
     into `exec`.
   - Generated:
     ```rust
     fn pruv_m_f_2_9_bud_store_2_ea(st: &[i64], e: i64, f: i64) -> i64 {
         if ((st.len() as i64) < 7i64) { return (-1i64); }
         let cap: i64 = pruv_m_f_2_9_bud_store_3_c32((unsafe { *st.get_unchecked((6i64) as usize) }));
         ...
     ```
     This is called 3-4 times per op, each time as a real call.
   - Hand: `self.sessions.get_mut(&self.k1[..])`, all inlined.
   - Pruv source has no way to ask for inlining. The fix belongs in the compiler: `#[inline]` on small
     non-recursive functions in `pruv-core/src/codegen.rs`.
   - Manual flattening in source would mean copying `ea` / `find` bodies into every caller in `bud_world`. That
     would duplicate the validated layout arithmetic, so it was not done.
2. **The per-op header check (contract).**
   - `m4_op` starts with `if (!pruv_m_f_2_9_bud_store_5_valid(&*st)) { return 2i64; }`. That is about 25 header
     compares on every op, about 2 ns of the 2-3 ns that remain after inlining.
   - The hand `exec` has no header: `HashMap` is typed state that cannot be corrupt.
   - Pruv receives `st` as a raw word buffer from the host on every call. The contract (status 2 on an invalid
     header) and "judgments in Pruv, host does side effects" mean the check cannot move to the host or be cached:
     the host could hand over any buffer.
   - `valid` already reads the header with plain indexing after one `len >= 232` check, so there is nothing left to
     cut in source.
   - Removing it needs a typed resident-state handle in the language or lib, a lib/compiler change.

So the generated code matches the hand code once (1) is done in the compiler and (2) is accepted as the cost of
the raw-buffer ABI. Neither can be fixed in the owned files.

### Pass 3: compiler inline hints and `eq_range` (2026-11-17)

Cause (1) is fixed in the compiler: both compilers emit `#[inline]` on every generated function (the two outputs stay
byte-identical; M11 compile CPU +~7%, binary -0.4%). `bud_map::entry_matches` compares the key with the new
`eq_range` builtin once both ranges are in bounds (verified code: one slice `==` through `get_unchecked`, the M4 binary now
imports `memcmp`, which the earlier asm did not call; the total per-unit loop stays as the fallback). The build profile (`opt-level = 3`) was checked
and left as is.

`host/run.py --only m4`, 5 rounds, load average 5.5-5.8, ns per op:

| n | Pruv | hand | TS | Pruv/hand (pass 2) |
|---:|---:|---:|---:|---:|
| 100 | 18.0 | 14.7 | 59.3 | 1.22 (1.22) |
| 500 | 18.6 | 16.4 | 65.6 | 1.13 (1.17) |
| 1000 | 18.5 | 16.2 | 65.3 | 1.14 (1.16) |

Tools 0.69-0.74, clocks 0.92-1.04. The earlier inline-only run of the same pass gave 1.23 / 1.13 / 1.12. The scratch
"all `#[inline]`" build (1.15 / 1.06 / 1.04) was a separate, lighter-load run; the n = 100 row is the noisiest.

**Still above 1.1x: cause (2), the per-op `valid()`.** It already runs once per root call (`m4_op`, `t_call`): no
sub-operation re-validates, so "once per root call" is the current state and saves nothing more. What is left is the
check itself, about 2 ns. The hand side has none because its state is a typed `HashMap` that the host cannot corrupt.

### Design: a typed resident-state handle (removes the per-call header check)

Not implemented in this pass (a language/ABI change across both compilers, the ABI generator and every host). The
goal is to keep the guarantee of `valid()` without re-checking a buffer the host cannot have changed.

1. **Opaque state type.** A lib type, e.g. `Store`, that wraps the word buffer. Its words are not exported. The
   generated ABI hands the host a `Store` value (a Rust newtype over `Vec<i64>` with a private field, an opaque
   pointer on the C ABI). The host can hold, move and drop it, but cannot build one or write its words.
2. **Only proven constructors.** `Store` is built only by Pruv functions that check or establish the layout:
   `init_store` (writes a fresh header) and `from_words(words: [i64]) -> Option<Store>` (runs today's `valid()`
   once, for a buffer that comes from outside, e.g. a snapshot). `grow_copy` returns a new `Store`.
3. **Mutators keep the invariant, by proof.** Every function that takes `inout st: Store` gets
   `requires valid(st)` and `ensures valid(st)` checked by the verifier. Today's code needs no such proof because
   it re-checks; the proof is the cost of the design. `valid()` reads only header words 0-14, and the mutators write
   the header only in `entry_alloc` / `entry_free` / arena bump / `rehash`, so the obligations are local to those
   functions.
4. **Root calls take the type.** `m4_op(inout st: Store, ...)` and `t_call` drop the `valid()` line. The status 2
   "invalid header" stays only on `from_words`.
5. **No trust in unchecked input.** The host never supplies words that skipped `valid()`: a buffer from outside
   goes through `from_words`, and anything else came out of a proven mutator. A host that mutates the words through
   unsafe code is outside the ABI contract, the same as corrupting a `HashMap`.

Expected effect from the scratch build above: M4 sessions about 0.94-0.98x, M8 noteFailure -80 of 736
instructions (`M8_NOTES.md`).

## Breakdown of store_find (n=1000, ns per op, medians of 7 rounds)

| step | Pruv | hand |
|---|---:|---:|
| lookup only (`store_find_lookup` / hand `store_find_core_u16`, UTF-16 keys) | 10.2 | 13.0 |
| lookup only, hand with Rust `&str` keys (`store_find_core`) | — | 8.2 |
| encode of a found, labelled session as text into a reused `String` (`store_find_enc` / hand `store_find_enc_text`) | 39.2 | 38.5 |
| lookup + labelling + text encode (`store_find` / hand `store_find_text`) | 80.4 | 77.2 |

### What closed the store_find gap

- There is one encoder. `Heap::enc_into` writes the JV text directly (frames `{"sref":"sN","v":..}`,
  `{"bref":"bN","v":..}`, `{"undef":1}`, `{"u16":[..]}` for lone surrogates, `{"num":"<hex>"}` for non-safe-int
  numbers). `Heap::enc` returns it as `J::Raw(text)`, an output-only J variant: `write_json` copies it as is and
  `decode` rejects it. The tree encoder was removed, so there is no second encoder to drift.
- Labels are `Label::N(u64)` for the canonical `<p><n>` form and `Label::T(String)` otherwise, in two maps
  (`by_num`, `by_text`). Labelling on first encode no longer formats and hashes a `String`.
- The id index is keyed by bytes (`idx_key`: an ASCII id is its own bytes, any other id is 0xFF followed by the LE
  bytes of each UTF-16 unit; 0xFF never occurs in ASCII, so the key is injective). It stores the handle next to the
  position, so a hit does not read `stored[pos]`. An ASCII `&str` probes with `id.as_bytes()` directly.
- Lookup with JS string semantics still holds: non-ASCII ids, including lone surrogates, go through their UTF-16
  units.
- The trace-only argument of the earlier won't-fix no longer applies under the tightened bar, and is not needed: the
  encode is now within the bar on its own.

## Fixes (host side only, no decision moved into the host)

All in `leaf.rs` / `host.rs`. They remove allocations and repeated work only. Every budget decision is still made by
the Pruv core.

Items 1–7 are the first pass, kept as history. Items 8–11 (the 1.1× pass) replaced parts of them: the lookup now
probes a byte-keyed index (no UTF-16 stack buffer for ASCII ids), labels are `Label` values in `by_num` / `by_text`
(no 32-byte label buffer or `by_label`), and the encoders write text (no pairs vector).

1. `find_session` / `find_session_enc` / `find_h`:
   - A string id probes the index through UTF-16 units in a 64-unit stack buffer. A longer id continues into a `Vec`
     from where the buffer ends, with no re-encode.
   - The index probe is factored into `find_key`.
   - The lookup helper went from 33 to 14–16 ns.
2. `label_num`: parses `p` + `u64` straight from the UTF-16 units, with the same acceptance as `str::parse::<u64>`
   (optional `+`, digits, overflow means none). Before, it allocated a `String` per `sref`/`bref` label seen.
3. `apply_sref`:
   - A label of up to 32 ASCII units is held in a stack buffer, with one `by_label` lookup through the new
     `set_budget_h(h, ..)`.
   - A miss allocates the label once and goes on to create the object, with no second lookup.
   - Non-ASCII labels take the old `from_utf16_lossy` path.
   - Before, every sref did an allocation plus a `contains_key` and a second `get` in `set_budget`.
4. `decode` of `{"obj":[[k,v],..]}`: a string key is cloned once. Before, `decode` produced `V::Str` and
   `js_to_string` cloned it again.
5. `build_obj`: objects with ≤ 8 entries dedup in place, with the same semantics (first position, last value). This
   saves the second vector.
6. `k_eq`: an ASCII literal is compared unit by unit after a length check, instead of through an `encode_utf16`
   iterator. Every key compare on the save path goes through it. All callers pass short literals.
7. `enc_session` / `enc_budget`: the label is encoded from `&str` with no `String` clone first, and the pairs vector
   is sized up front. This gives a small gain on the trace path.
8. One text encoder (`Heap::enc_into`, `J::Raw`) replaces the J tree encoder; see "What closed the store_find gap".
9. `Label` enum with numeric labels, and the byte-keyed id index holding the handle (same section).
10. `u16s` allocates exactly once for an ASCII string (it grew its vector several times on the save path).
11. `num_of`: up to 15 digits are folded directly (exact in f64); longer text goes through `parse::<f64>`.

## Compiler requests (not done here; the compiler belongs to another sub)

1. **Total accessor guards** (M4_NOTES "Range-check cost"). Done at source level, no compiler change; see the M4
   sessions profile.
   - Indices come from buffer contents, so per-call proofs aren't possible. A verifier-driven guard drop at the call
     site would find nothing to drop at the hot sites.
   - The source fix (one range check per run of accesses, plain proven indexing inside, the total accessor as the
     fallback) brings M4 sessions from 1.51–1.60× to 1.30–1.40×.

## Method

- Runner: `python3 perf_headroom/host/run.py [--rounds 5] [--pruvc P] [--only m4|m5] [--no-build]`, run from
  bench/l3/budget.
  - It builds the hand crate into `work/handbuild`, the M4 driver through `m34_test/run.py build()`, and the M5
    harness through `dev_build.sh`.
  - It runs the `m4_hand` and `perf_ts.mjs` selftests, then the rotating rounds.
- M4 hand baseline (`host/src/bin/m4_hand.rs`) is a std-only port of `perf_ts.mjs` `exec()`.
  - The timed loop matches `driver_m4 perf` in work and shape, not expression for expression: a cyclic replay of one
    op period, and per op an output capacity check, the call, and a `black_box` on the op and on the result. The
    differences are that the hand check is against a fixed 4096 words where the driver's depends on the state size,
    and the hand sink is `out[0]+out[1]` where the driver's is `out[0]+max(out[0],0)`. There is one measured run on fresh state
    after an untimed prefill, with no separate warm-up runs.
  - The driver's grow retry never fires in these runs: `grow_ns` is subtracted, and it is 0 at steady state.
  - Keys are rebuilt from units into scratch buffers on every call, as Pruv and TS do.
  - State is `HashMap`/`HashSet` with `Box<[u16]>` keys.
  - `m4_hand selftest` checks result words and rejections against the CONTRACT cases.
- M5 hand baseline (`host/src/bin/m5_hand.rs`):
  - Same n, the same id sequence `(i*7919)%n`, and the same session values `{id, n}`.
  - Labels are assigned on first encode, and store_save starts from s1..sn labelled.
  - `store_find_enc_text` labels all sessions and finds the handles untimed, like `harness` `store_find_enc`, then
    writes each frame as text into a reused `String`. `store_find_text` does lookup, labelling and the text frame.
  - store_save gets the same program JV per save as `harness.rs`, as an owned tree built untimed. It does the same
    walk: label counters, sref resolve, the budget pair only, decode with `Object.fromEntries` duplicate semantics,
    then set or append `budget` and record the budget label.
  - The untimed check after store_save only confirms that each session has a `budget` field and a budget label. The
    values are checked by `m5_hand sample`.
  - The hand input tree holds numbers as `f64`. The host J keeps number text and parses it in `decode`. That parse is
    part of the host's boundary and is inside the Pruv number.
  - `m5_hand sample` prints the find frame and a saved session to check the shape:
    `{"sref":"s6","v":{"obj":[["id","s5"],["n",5]]}}`.
- Pruv M5 breakdown rows (`store_find_lookup`, `store_find_enc`) are `harness perf` measures. `store_find` and
  `store_find_enc` write into a reused `String`, like the hand text measures.
- `run.py` pairs: store_find/store_find_text, store_find_enc/store_find_enc_text,
  store_find_lookup/store_find_core_u16 (and store_find_core for reference), store_save/store_save.
- Correctness after the final changes:
  - `m5_test/run_m5.py`: exact=300, P6=0, unexplained=0, driver violations 0, determinism identical.
  - `m34_test/run.py m4`: 300/300 pass.
- `bench/l3/lib`, `pruvc/` and `pruvc-rs/` are untouched. The M4 Pruv sources and the compiler binary (main checkout
  `pruvc-rs/target/release/pruvc`) are unchanged, so the M4 Pruv numbers differ from M4_NOTES only by load.
- 1.1× pass audit (Codex gpt-6-astra ultra), two findings, both fixed: a `Label::N` above `i64::MAX` was written
  as a negative number (now `push_uint`), and the hand `store_find_enc_text` labelled in lookup order, not store
  order like the harness, so the frames differed (now store order).
- One audit (Codex gpt-6-astra ultra) found no semantic change and no moved decision. It had six required findings,
  all applied:
  - compare encode with encode;
  - scope the lookup claim;
  - align the M4 hand loop with the driver;
  - no re-encode for long ids;
  - no second lookup on a label miss;
  - state the M4 op counts.
