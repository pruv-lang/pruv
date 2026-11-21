# L3 LG2: language growth for the §7.5.1 gaps (second bundle)

Scope: G-R05-4, G-R05-3, G-R06-1, G-R05-2, G-R00-1 (rest), G-R07-1, G-R05-6 and the debt D-R10-1 from
`docs/L3_RESTRUCTURE_SPEC.md` §7.5 / §7.5.1. G-R05-1, G-R06-2 and G-R00-4 / G-R06-3 are out of scope.
Both compilers (Python `pruvc/`, Rust `pruvc-rs/`) change in one commit. No `bench/l3/` fragment is changed here; the
last column says which fragment can now drop its workaround.

| Row | Status | What exists now | Fragment that can drop its workaround |
|---|---|---|---|
| G-R05-4 | done | Shifts were already there (`wrapping_shl`, `ushr`, `band`, SPEC §5 bit builtins). New: `load_le(a, i, n)` (expression, code and contracts) and `store_le(inout a, i, w, n);` (statement, not in contracts), the little-endian twins of `load_be` / `store_be`: same types (`[u8]`), literal `n` in 1..=8 (else `type_mismatch`), same `index_oob` obligation `0 <= i && i + n <= len(a)`. Byte `a[i+k]` has weight `256^k`; `n < 8` gives `0 .. 2^(8n)`, `n = 8` is two's complement. Store writes `band(ushr(w, 8k), 255)` to `a[i+k]`, other bytes unchanged. Rust: `from_le_bytes` / `to_le_bytes`; LLVM: byte loop like BE in reverse order. | `bench/l3/qwen/boundary.pruv:224` `put_word` (the `% 256` / `/ 256` loop) becomes `store_le(inout w, at, v + 1, 8);` after the range check, and `get_word` (`:242`, the unrolled read) becomes `load_le(w, at, 8) - 1` with the `w[at + 7] != 0` check kept. Both backends. |
| G-R05-3 | done | New `utf8_valid(b: [u8]) -> bool`: pure, code and contracts; an uninterpreted predicate of the array value in the verifier (one z3 function over the logical length, tail fixed to 0, so equal arrays give equal results). Run time: `core::str::from_utf8(..).is_ok()` on Rust, an internal IR validator (`pruvc/utf8_valid.ll`, emitted once when used) on LLVM. New `str_from_utf8(b: [u8]) -> str ! alloc`, code only, same positions as `substr`. The verifier proves `utf8_valid(b)` (new code `verify/utf8_invalid`), then knows `len(result) == len(b)` and `result[k] == b[k]` for `0 <= k < len(b)`. Proof mode: no run-time check; `--no-verify`: check, exit 101 with `pruv: invalid UTF-8 (len n)`. Guard with `if utf8_valid(b) { let s: str = str_from_utf8(b); ... }` or `requires utf8_valid(b)` (`tests/spec/r6b_1_utf8_requires.pruv`). | The "text bytes go into the wire as spans" workaround (qwen turn output) can build `str` values with `str_from_utf8` of each span once the span is checked; the fragment decides whether a `Vec<String>`-like result is still wanted. |
| G-R06-1 | covered-already | An `inout` array keeps `len(a) == len(old(a))` (SPEC §6g), so in-place growth stays impossible by design. A helper can grow the caller's buffer by value with move semantics: `fn grow(b: [u8], n: i64) -> [u8] ! alloc requires len(b) <= n ensures len(result) == n { return resize(b, n, 0); }` and at the call site `b = grow(b, n);` for a local `let mut b: [u8]`. The verifier knows `len(result) == n`, the prefix is equal and the tail is 0 (`resize` facts). Both backends. | `bench/l3/grok/read.pruv` `window`'s inlined grow-and-retry can move into a helper called as `buf = grow(buf, want);` (both backends). |
| G-R05-2 | covered-already | R-6e-12: `move x` of an owned `[u8]` local into a constructor field moves the buffer without a copy on both backends. New test `tests/runtime/lg2_rt_u8_field_move` (`return W { tag: t, bytes: move b };`, both backends, both flag sets). | `bench/l3/qwen/chain.pruv:182` `select(data, cut, inout marks)` can build `marks` as its own local and return it as `Chain { ..., marks: move marks }`, so the caller no longer owns the bitmap (both backends). |
| G-R00-1 (rest) | not done | Blocker: every R-6e vector op, `vec_reserve` included, is Rust-backend only; the LLVM backend rejects `Vec<T>` (`pruvc/llvmgen.py:5394`, `:5431`, `:5465`; `pruvc-rs/crates/pruv-core/src/llvmgen.rs:6269`; `bindings.rs:269`). `vec_reserve` alone cannot be added because LLVM has no `Vec` representation (capacity field, grow shim, exclusive borrow of the place) to reserve into; it needs the whole R-6e vector lowering on LLVM. | None; `packet.pruv:331` keeps `resize(p, want, 0)` on the LLVM lane. |
| G-R07-1 | not done | Blocker unchanged from LG1: LLVM rejects every non-array `inout` parameter (`pruvc/llvmgen.py:5465`, "needs the Rust backend") and `--rust-bindings` has no C layout or write-back ABI for an `inout` struct (`pruvc/pruvc.py:170`, `pruvc/codegen.py:3777`). Needs an LLVM pointer-to-record convention plus a `#[repr(C)]` layout and binding wrapper. | None; `opencode_step_admitted` keeps the host-owned admission. |
| G-R05-6 | covered-already | `inout` array parameters (`[i64]`, `[u8]`, `[bool]`) on a root `pub` non-generic entry already work on both backends: Rust `&mut [T]`, LLVM ABI symbol takes pointer and length, `--rust-bindings` forwards `b: &mut [T]` (`tests/runtime/g5_r19_resident_entry/root.pruv`: `pub fn poke(inout b: [u8], i: i64, v: i64) -> i64`, binding `pub fn poke(b: &mut [u8], i: i64, v: i64) -> i64`). The host supplies the buffer and keeps its length. SPEC §6g "Host limits" said root entries cannot take `inout`; LG2 corrects that text. | `bench/l3/qwen/import.pruv:436` `window_wire` can take `inout wire: [u8]` sized by the host instead of allocating and returning it, as long as the host knows the length up front (no resize through an `inout` entry). |
| D-R10-1 | not done | Not measured in this bundle (2h cap). LG1 state holds: `put_cp` inclusive size 353 IR lines, 6 call sites, `353 * 5 = 1765 > INLINE_BUDGET 1500`, so P33 does not promote it. Smallest change: honour a source-level inline hint on LLVM bodies, or change the P33 budget, then measure with `bench/l3/supervisor/measure_r10.py`. | No fragment change. |

Status: LG2: G-R05-4 done (`load_le` / `store_le`; shifts covered-already).
Status: LG2: G-R05-3 done (`utf8_valid`, `str_from_utf8`, `verify/utf8_invalid`).
Status: LG2: G-R06-1 covered-already (`b = grow(b, n)` with `resize`; in-place growth blocked by the §6g length invariant).
Status: LG2: G-R05-2 covered-already (`[u8]` field move, `tests/runtime/lg2_rt_u8_field_move`).
Status: LG2: G-R00-1 not done (LLVM has no `Vec` lowering; `vec_reserve` stays Rust-backend only).
Status: LG2: G-R07-1 not done (no LLVM inout-struct convention, no C layout for `--rust-bindings`).
Status: LG2: G-R05-6 covered-already (`inout` array root params on both backends, `g5_r19_resident_entry`; SPEC §6g text fixed).
Status: LG2: D-R10-1 not done (not measured; needs an inline hint or a P33 budget change).

## Verification

New tests: `tests/runtime/lg2_rt_{words_le,utf8,utf8_panic,u8_field_move}`,
`tests/must_reject/lg2_{load_le_index,load_le_width,store_le_contract,utf8_contract,utf8_no_alloc,utf8_type,utf8_unguarded}`,
`tests/spec/r6b_1_utf8_after_store` (reject, `utf8_invalid` after a store), `tests/spec/r6b_1_utf8_requires` (accept).

| Check | Result |
|---|---|
| `run_all.py --only runtime --sample-prefix tests/runtime/lg2` (py, rs, both backends) | 14/14 each |
| `run_all.py --only must_reject --sample-prefix tests/must_reject/lg2` (py, rs) | 7/7 each |
| `tests/spec_trace.py` (py, rs) | 1282/1282, false_reject 0, false_accept 0 |
| `cargo test --release` | 150 passed, 0 failed |
| `tests/diff_rust.py --fuzz-count 20` | all same |
| `run_all.py --only g8_frontend --backend both` | py 27/27, rs 39/39 |
| `tests/run_all_rows.txt` | 21 rows added for the new runtime and must_reject samples on the rs compiler (4605 rows) |
| Python full suite (`run_all.py --backend both --compiler py`) | 4593/4593 |

The new verification encoding (`load_le` / `store_le` byte order, `utf8_valid` predicate, `str_from_utf8` obligation and
facts, both compilers) was audited once with no findings.
