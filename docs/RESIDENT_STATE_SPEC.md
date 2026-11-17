# Resident state: mutable host entries

Status: **accepted, 2026-11-17** (opposing review: modify; modifications applied, §9).

This document lets a host keep a large Pruv state between calls without encoding, decoding, copying or digesting
it on every call. It does that by implementing the item that `docs/G5_SPEC.md` §15 deferred: *mutable host
entries*. A host entry may take `inout` array parameters; the host allocates the state buffers once, passes the
same buffers to every call and never reads or edits them. Pruv updates them in place, so one call costs what it
touches, not what the state holds.

Python `pruvc` and `pruvc-rs` implement it in the same batch, for both the Rust source backend and the LLVM
backend.

## 1. Problem

L3 slices (`bench/l3/*`) are driven by a Rust host that calls one Pruv entry per step. Before this batch a host
entry could take arrays only as read-only views and return new arrays (`type/entry_signature` for inout,
collections and extended types, `docs/G4_SPEC.md`, `docs/G5_SPEC.md` §6.1). A state that must survive a call
therefore went in as a read-only `[i64]` and came back as a fresh copy. Every step paid `O(W)` in the world
size `W` for the copy alone, and the slices added digests and packet re-emission on top. The request-budget
consumer (`docs/REQUEST_BUDGET_SPEC.md` §8–§9) needs the opposite: the state stays resident, the host treats it
as opaque, and each call is bounded by keyed-map work, not by the number of sessions.

The three-way measurement (`bench/l3/perf/REQ_THREEWAY.md`, main f29f9cb) sharpens the cost picture for `req list`:

- Per-call cost is flat at about 20 µs from S=2000 to S=50000, because the world packet does not grow with S. The
  20x gap to Node is the number of host round trips (two per session per item: `sessionList.next`, `isTemporary`;
  420 k at S=20000) times that fixed cost. Node pays about 0.8 µs per round trip.
- The generated code is not the cause: the same search as one Pruv call is 0.64x hand-written Rust.
- Inside a step, the self time is allocation (27%), whole-packet digest (19%), seal and bundle (19%) and whole-array
  copies (11%).

So this batch has two goals, and §6 bounds both:

1. **Residency removes round trips.** Data that Pruv decides on (the session list, per-session flags) is ingested
   into resident state once and walked inside one Pruv call. The host is asked only for side effects and for facts
   it alone owns, never once per item. The language feature makes that possible; the slice rewrites that use it
   belong to the later L3 structure batch.
2. **The remaining boundary call has a small fixed cost.** Crossing the boundary with resident buffers allocates,
   copies, digests and encodes nothing. Its fixed cost must be in the Node range (a few µs or less); §8 measures it.

Inside Pruv the needed machinery already exists: `let mut` arrays and `inout` parameters are stored in place
(SPEC.md §6g), and their verification encoding is in place (old snapshot plus a fresh same-length final array).
Only the boundary was closed.

## 2. Decision

**Chosen: mutable host entries over flat arrays (this document).**

Considered and not chosen in this batch (debate record in §9):

- (a) A Pruv-run event loop on the G10 `Loop` that owns the state as a local. It needs stdin or socket framing,
  a termination story for a server loop (verified loops need `decreases`), and a rewrite of every host. It
  solves residency only for programs that also move their I/O into Pruv.
- (b') Opaque collection handles at the boundary (`take`/borrow/return `Vec`, `HashMap`). A host entry returns
  one value, so a state made of several collections plus an outcome does not fit one call, and the LLVM path
  needs the G6 shim linked into the host. Kept as a possible later batch on top of this one.
- (c) Owned handles inside structs and collections (`type/foreign_owned_type`). Needs field-level ownership,
  partial moves and nested drop duties in the must-consume analyzer. Separate, larger batch.

## 3. Surface

In explicit visibility mode (G4) a host entry may declare `inout` parameters:

```pruv
pub fn step(inout st: [i64], inout out: [i64], call: [i64]) -> i64 ! alloc
    requires len(st) >= 16 && len(out) >= 4
{ ... }
```

- The allowed types are exactly those of internal `inout` (SPEC.md §6g) after the host-entry rule that entries
  are nongeneric: `[i64]`, `[u8]`, `[bool]`. Any other type stays `type/inout_type`.
- Any number of `inout` parameters may be mixed with ordinary parameters, in any order.
- The length of an `inout` array never changes in a call. Growth is a program convention (§5).
- All other host-entry restrictions are unchanged: collections, extended (G9) types and owned catalog types in a
  host-entry signature remain `type/entry_signature`.
- Legacy (non-explicit) mode does not change: a function with an `inout` parameter stays internal and gets no
  public Rust API, raw ABI wrapper or binding.
- `main` still cannot have `inout` parameters.

## 4. Meaning at each backend

### 4.1 Verification

A host entry with `inout` parameters is verified exactly like an internal `inout` function (G5 encoding):
`old(a)` names the entry contents, the body stores in place, the ensures relate the final contents to `old`.
Nothing new is encoded.

### 4.2 Requires checks at the entry

Host entries check their requires unless `--trust-callers` is set (`docs/G4_SPEC.md`). That rule applies to a
mutable host entry unchanged. The check reads the buffers before the body runs and never copies them.

A program separates two kinds of precondition (opposing review, modification 3):

- **The state invariant** (the resident buffers are a valid state). It is established by Pruv code only: by the
  init entry, by every normal return of a step entry (an ensures), and by the grow entry (§5). The host never
  writes the buffers, so the invariant carried by one call's ensures is what the next call's requires assumes.
  Checking it at the boundary is `O(W)`; a program that needs per-call cost independent of `W` keeps it out of the
  entry requires (or keeps only fixed-cost facts there: lengths, header words) or builds with `--trust-callers`.
- **New input** (the call, the facts, anything the host produced). It is untrusted on every call. A program
  validates it in executable Pruv code (an `if` returning a rejection status), not only in requires, so the
  validation runs under `--trust-callers` too. Its cost is the input size, not `W`.

A call that fails (requires failure, overlap failure, panic) ends the process (`pruv_panic_requires`, exit 101). A
host that catches such a failure by other means must not reuse the buffers: a partly modified state is not a valid
state. An entry built with `--trust-callers` is not a safe public API on its own; it is safe only behind the
adapter in §4.5.

The G5 rule that an internal `inout` function's checked wrapper runs even under `--trust-callers` (because Pruv
callers that skip proof can reach it) is not changed for internal functions. It is a separate legacy branch and does
not apply to the explicit host entry (which uses the existing explicit `checked_entry` choice).

### 4.3 Rust source backend

The entry is emitted `pub fn step(st: &mut [i64], out: &mut [i64], call: &[i64]) -> i64`. The host passes
`&mut v[..]` for a `Vec<i64>` it owns. rustc's borrow rules already forbid passing one buffer as two writable
arguments or as a writable and a read-only argument, so the backend adds no overlap check.

### 4.4 LLVM backend

- The raw wrapper `pruv_abi_step` takes each array parameter as pointer plus length, as before; an `inout`
  parameter's pointer is writable and is passed straight to the internal function (no copy, no allocation). The
  wrapper's own pointer parameters carry no `noalias`; the internal function keeps its `noalias` parameters, which
  the overlap check below makes true.
- **Overlap check.** Before anything else in the wrapper (representation checks, requires, body), and regardless of
  `--trust-callers`, the wrapper rejects overlap between each `inout` argument and every other pointer-carrying
  argument:
  - byte ranges: `[i64]` is `len·8` bytes, `[u8]`, `[bool]` and `str` are `len` bytes, an aggregate parameter
    passed by pointer is `words·8` bytes, and the result storage `%out` (array result `{ptr, i64}` is 16 bytes,
    aggregate result `words·8` bytes) is compared too, because the wrapper writes it after the body;
  - on Windows x64 the hidden `Option`/`Result` `%sret` storage (16 bytes) is part of the list too, like `%out`;
  - read-only/read-only pairs are not compared; each writable pair is compared once (pairs `i<j` where either is
    `inout`);
  - a lone pointer-carrying `inout` region (the only pointer-carrying argument, scalar result) is checked against an
    empty sentinel, so a negative length or a `len·elem` / `ptr+bytes` overflow is still rejected;
  - a range of length zero overlaps nothing; any partial overlap is rejected, not only equal start addresses;
  - a negative length is rejected, and the byte count and end address are computed so that an overflow is
    rejected instead of wrapping.
  An overlap takes the requires failure path (`pruv_panic_requires` with the entry name and the message
  `inout argument overlaps another argument`, exit 101). Cost: `O(k²)` in the number of pointer arguments `k`,
  independent of their lengths.
- The raw `[bool]` representation check (`pruv_check_bool`) stays on the raw wrapper, also for `inout` `[bool]`. It
  is `O(len)`; a host that needs `O(1)` boundary cost for a `[bool]` state uses the `__typed` variant (Rust
  bindings), which drops only that check and keeps the overlap check.
- `--rust-bindings` exposes an `inout` parameter as `&mut [i64]` (`&mut [u8]`, `&mut [bool]`), with an extern
  declaration `*mut i64` / `*mut u8` plus `i64` length and a call `x.as_mut_ptr(), x.len() as i64`, so safe Rust
  callers also get the overlap guarantee from rustc.
- Pointer validity (each range is one live, writable or readable allocation) and exclusive access during a call
  remain the raw caller's duty; the C ABI cannot check them.

### 4.5 Host contract (residency) and the adapter

The host owns the buffers' memory. Between two calls it must not read, write, move without copying, or share them
with another thread while a call is running; calls on one state are serialized. With `--trust-callers` a host
that breaks the contract voids the entry's verified guarantees for that call, exactly as for any other unchecked
requires.

A consumer (e.g. `docs/REQUEST_BUDGET_SPEC.md` §8) does not see the raw entries. It sees an opaque adapter that the
host program writes once, for example in Rust:

```rust
pub struct ResidentState { st: Vec<i64> }          // private buffers; no Clone, no accessors
impl ResidentState {
    pub fn new(cfg: &[i64]) -> Self;                 // calls the Pruv init entry
    pub fn step(&mut self, call: &[i64], facts: &[i64], out: &mut [i64]) -> Status; // Done / Need / Grow handled inside
    pub fn observe(&self, out: &mut Vec<i64>);       // whole-state encoding, only at final observation
}
```

The adapter never swaps, clones or edits the buffers, and its `&mut self` gives the per-instance serialization.
All decisions are made in Pruv; the adapter only allocates, calls and performs side effects the program asks for.

## 5. Growth (program level, not language)

The length of an `inout` buffer is fixed in a call, so growth is a protocol between the program and the adapter:

1. When a step finds its internal capacity short, it returns a status *Grow(N)* **before changing anything**: no
   word of any `inout` buffer is written and no side effect is requested. *Grow* is distinct from the consumer's
   *Need* (a request for an external fact); the adapter never shows *Grow* to the consumer.
2. The adapter allocates a new zeroed buffer of `max(N, 2·len)` words and calls a Pruv entry
   `grow(old: [i64], inout new: [i64])`. Relocation and rehashing happen in Pruv; the host never copies or
   interprets words. `grow` establishes the state invariant in `new`.
3. The adapter drops the old buffer and repeats the same step with the same input.

Cost: a step without growth pays only its own work. A grow is `O(W)`. With doubling, the total grow work over a
run is `O(final W)`, so growth is **amortized** `O(1)` per stored word; it is not a worst-case per-call bound. A
program that needs a strict per-call bound needs pre-sizing, split storage or incremental migration, which this
document does not provide.

## 6. Complexity contract

Let `W` be the total length of the resident buffers, `k` the number of pointer arguments, `T` the words a call
reads or writes in its body, `I` the size of the new input it validates, and `Q` the cost of evaluating the entry
requires. `R` is the representation-check cost of the raw wrapper: `O(len)` for each raw `[bool]` argument (every
byte must be 0 or 1) plus `O(len)` UTF-8 validation for each raw `str` argument. `R = 0` on the `__typed` path or
when the entry has no `bool`/`str` parameters.

| Flow | Bound |
|---|---|
| Boundary crossing (Rust source backend) | `O(1)` |
| Boundary crossing (LLVM raw or typed wrapper, overlap check) | `O(k²)` |
| Representation check `R` (raw `[bool]`, raw `str`) | `O(len)` per bool array, `O(len)` UTF-8 per `str`; `0` on the `__typed` path or without bool/str params |
| Requires check (checked mode) | `O(Q)`; `O(1)` for length and header facts |
| Requires check (`--trust-callers`) | `0` |
| Input validation (executable Pruv) | `O(I)` |
| Body | `O(T)` |
| One call, no growth | `O(k² + R + Q + I + T)`; no term in `W` unless `R` scans the state or the program's own code touches `W` words |
| Grow (§5) | `O(W)` for that call; amortized `O(1)` per stored word over a run |

For `N` calls the total is the sum of the per-call bounds plus growth. No call copies, digests or re-encodes a
resident buffer, and nothing is proportional to `N·W` unless the program reads its whole state on every call.
The per-call bound is independent of `W` only for calls where `R` does not scan the state: a resident `[bool]` or
`str` state passed through the raw wrapper pays `R = O(W)` on every call, so such a host uses the `__typed` variant.

The gate in §8 measures this bound.

## 7. Diagnostics

| Code | Meaning |
|---|---|
| `type/entry_signature` | Host entry with a collection, extended or owned type (unchanged); no longer raised for `inout` arrays in explicit mode |
| `type/inout_type` | `inout` of a type other than `[i64]`, `[u8]`, `[bool]` (unchanged) |
| runtime `requires violated: <entry>: inout argument overlaps another argument` | LLVM raw/typed wrapper overlap check (§4.4), exit 101 |

## 8. Tests and gates

- Accept: an explicit-mode host entry with two `inout` parameters and an ordinary parameter, verified, with an
  ensures over `old`.
- Reject: an explicit-mode host entry with `inout` of an aggregate type (`type/inout_type`) and a collection in a
  host entry (`type/entry_signature`, unchanged). Legacy mode: an `inout` function still gets no public API.
- Runtime (both backends): a Rust driver keeps a state buffer and calls the entry repeatedly; the results match an
  expected trace.
- LLVM raw ABI: a C caller calls the wrapper on disjoint buffers (succeeds, buffer changed in place), on two
  overlapping `inout` buffers, on an `inout` buffer overlapping a read-only array, and with `%out` inside an
  `inout` buffer (each rejected with exit 101 before the buffer is touched), and with zero-length adjacent buffers
  (accepted). `tests/abi/check_inout_overlap.py` also covers (both compilers, with and without `--trust-callers`):
  - lone inout `[i64]` (`size_*`): normal and empty accepted; negative length, `len·8` multiply overflow and
    `ptr+bytes` add overflow rejected (empty-sentinel check);
  - `[u8]` byte granularity (`bytes_*`): exactly adjacent accepted, a 1-byte overlap rejected;
  - `str` inside an inout `[u8]` (`tag_overlap_str`) rejected;
  - `[bool]` (`flags_*`): overlap rejected on `pruv_abi_flags__typed`; on `pruv_abi_flags` with an invalid byte (7) in
    the read-only array the overlap message is still the one reported (overlap check runs before the bool check);
  - three pointers (`tri_*`): disjoint accepted; only the last pair (`y`, inout `z`) overlapping rejected.
- Generated output agrees between the two compilers (`tests/diff_rust.py`, run_all compiler comparison).
- Performance gate (`tests/resident/perf/`): a resident state of `S = 2000` and `S = 20000` records; 10,000 calls
  that each touch a constant number of words; the time per call differs by less than 2x between the two sizes.
  The same harness measures the fixed cost of an empty call through the entry, which must be at most 2 µs (the
  Node round trip in REQ_THREEWAY is about 0.8 µs). As a contrast it times the pre-batch shape (read-only state in,
  fresh copy out). Results go to `RESULTS.md`.

## 9. Debate record

An opposing review (Codex, 2026-11-17) attacked the choice. Verdict: **modify**, not reject. The applied
modifications:

1. Only explicit-mode host entries get `inout`; the legacy policy (inout functions are internal) stays (§3).
2. The consumer sees an opaque adapter, not raw buffers; no swapping, cloning or concurrent calls (§4.5).
3. The trusted state invariant is separated from untrusted new input; input validation is executable Pruv code that
   runs under `--trust-callers`; init, normal return and grow each establish the invariant; a failed state is not
   reused (§4.2).
4. Growth is a no-change, no-side-effect return; the host only allocates; relocation is a Pruv `grow` entry;
   internal capacity shortage is distinct from an external *Need* (§5).
5. The overlap check covers read-only arrays, `str`, aggregate pointers and `%out`, by byte range with overflow
   checks; pointer validity and exclusive access stay raw-caller duties (§4.4).
6. Costs are stated per path: amortized growth, not a worst-case bound; the raw `[bool]` check is `O(len)` and
   avoided on the typed path (§6).

Rejected alternatives, briefly: a Pruv-run event loop (a) moves all I/O into Pruv and needs nonterminating loops;
collection handles (b') do not fit a multi-collection state into one return value and need the G6 shim in every
host; owned handles in structs (c) need field-level ownership in the must-consume analyzer. The review found no
conflict with the existing proof encoding (the old snapshot is a proof object, not a runtime copy) and confirmed
that the always-checked rule for internal `inout` wrappers is a legacy branch separate from the explicit entry.
