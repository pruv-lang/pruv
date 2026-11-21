# L3 LG1: language growth for the §7.5.1 gaps

Scope: the gaps G-R10-1, G-R00-1, G-R00-2, G-R00-3, the debt D-R10-1 and G-R07-1 from
`docs/L3_RESTRUCTURE_SPEC.md` §7.5 / §7.5.1. Both compilers (Python `pruvc/`, Rust `pruvc-rs/`) change in one commit.
No `bench/l3/` fragment is changed here; the last column says which fragment can now drop its workaround.

| Row | Status | What exists now | Fragment that can drop its workaround |
|---|---|---|---|
| G-R10-1 | done | `copy_within(inout a, dst, src, n);` (SPEC R-5-2): overlap-safe move inside one `[i64]`/`[u8]` array. Obligations, in order: `0 <= dst`, `0 <= src`, `0 <= n`, `dst + n <= len(a)`, `src + n <= len(a)` (bounds only). Fact: one array-theory update `a' = fun p. if dst <= p < dst + n then a[src + p - dst] else a[p]` over the old `a`. Rust: `::core::ptr::copy` (verified) / `copy_within` after range checks (`--no-verify`); LLVM: `llvm.memmove`. An index argument naming `a` (also `len(a)`) is `type/inout_alias`, so bind `let l: i64 = len(a);` first. | `bench/l3/supervisor/btail.pruv:77` `trim`: the byte loop that moves the kept rest to the front becomes `copy_within(inout buf, 0, ts[0], l - ts[0])` with `l = len(buf)` (both backends). |
| G-R00-1 | done (Rust backend) | `vec_reserve::<T>(inout P, n);` (SPEC R-6e-19), needs `! alloc`. The verifier requires `0 <= n <= 2^40` and keeps `len(P)`, every element and sibling fields; capacity is not modelled, so what the proof knows is the length invariant only (`len(P)` unchanged, then each `vec_push` / `vec_extend_from_slice` adds exactly what it writes). Rust: `P.reserve(n as usize)`. LLVM: `codegen/unsupported_type`, like the other R-6e vector ops. There is no reserve-only tail for `[i64]`/`[u8]` arrays: a `[T]` slot always has a value in the verifier model. | `bench/l3/mcp/packet.pruv:331` `resize(p,want,0)` can drop its zero-fill only by building the packet as `Vec<i64>` (reserve `want - len(p)`, then push / extend), i.e. on the Rust backend. The `[i64]` form and the LLVM lane keep the workaround. |
| G-R00-2 | covered-already (Rust backend, `Vec`); not done for `[T]` | §6g forbids resizing an `inout` array and every inout array has `len(a) == len(old(a))` (SPEC §6g, "never ... resize"), so a caller's `[i64]` packet cannot grow in place. An `inout v: Vec<i64>` parameter can: `vec_reserve::<i64>(inout v, k); vec_extend_from_slice::<i64>(inout v, s);` grows the caller's vector without copying the prefix (R-6e-10 places, R-6e-18, R-6e-19). Rust backend only (LLVM rejects inout collections). | `packet.pruv` `write` could take `inout p: Vec<i64>` instead of `p: [i64]`, Rust lane only. The 5010-word prefix copy stays on LLVM and for the `[i64]` signature. Blocker for `[T]`: §6g length invariant; lifting it needs a new rule for resizable inout arrays (out of LG1 scope). |
| G-R00-3 | covered-already | R-6e-12: `move x` of an owned array local into a constructor field moves the buffer without a copy on both backends (`tests/spec/r6e_12_llvm_move.pruv`: `let a: [i64] = alloc(4, 0); return D { a: move a };`). A `take` array parameter moved into a field also works on the Rust backend (`tests/spec/r6e_12_take_param.pruv`); on LLVM a `take` array parameter is `codegen/unsupported_type`. Usage: build the descriptors in an owned local `let mut d: [i64] = ...;` in the function that builds the result record and end with `R { ..., desc: move d }`. | The plan's descriptor buffer in `packet.pruv` can move into the result record when it is an owned local of the function that returns the record (both backends), or a `take` parameter (Rust backend). |
| D-R10-1 | partly done | SPEC R-7-7: the LLVM backend runs the P33 rule (R-7-3, same `INLINE_SMALL` 8 / `INLINE_BUDGET` 1500 from `pruvc/data/inline_policy.json`) over its IR text and marks promoted `define internal ... @F__pruv_body` headers `alwaysinline`. `put_cp` (`btail.pruv:105`) is NOT promoted: inclusive size 353 IR lines, 6 call sites, 353 * 5 = 1765 > 1500. Making `put_cp` inline needs a policy change (budget or a size measure other than IR lines), which is a P33 decision, not done here. | No fragment change. The perf effect on the rs/llvm `core` cells is for the stage-end measurement (not measured in LG1). |
| G-R07-1 | not done | Blocker: the LLVM backend rejects every non-array `inout` parameter, not only on entries (`pruvc/llvmgen.py` "has an inout struct parameter, which needs the Rust backend"; Rust `pruv-core/src/bindings.rs` `struct_inout_abi_error`), because it has no calling convention for an inout struct (a pointer to the struct record with in-place field writes). `--rust-bindings` rejects it on host entries ("host entry ... takes an inout struct, which has no C ABI") because there is no C layout for a struct passed by reference. Extending it needs both: an LLVM inout-struct convention and a `#[repr(C)]` struct layout plus a binding wrapper. | None; `opencode_step_admitted` keeps the host-owned admission. |

## Verification

New tests: `tests/spec/r5_2_copy_within_{accept,src_end,idx_alias}`, `tests/spec/r6e_19_{vec_reserve,reserve_unbounded}`,
`tests/spec/r7_7_llvm_inline{,_recursive}`, `tests/runtime/l3_rt_copy_within{,_nv_oob}`, `pruvc-rs/crates/pruv-proof/tests/copy_within.rs`.

| Check | Result |
|---|---|
| `tests/spec_trace.py` (py, rs) | 928/928 each, false_reject 0, false_accept 0 |
| `run_all.py --only runtime --sample-prefix tests/runtime/l3_rt_copy_within` (py, rs, both backends) | 6/6 each |
| `cargo test --release` | pass (the `catalog_shapes` count is now 104 for `vec_reserve`) |
| `tests/diff_rust.py --fuzz-count 20` | all same |
| `run_all.py --only g8_frontend --backend both` | py 27/27, rs 39/39 |
| `tests/run_all_rows.txt` | 6 rows added (the two new runtime samples on the rs compiler) |
| Python full suite (`run_all.py --backend both --compiler py`) | 4572/4572 |

`tests/g8/compatibility.json`: the four `llvm` output hashes change only by the ` alwaysinline` attribute on promoted
`__pruv_body` headers (removing it gives the old hashes); bindings and stdout hashes are unchanged.
The new verification encoding (copy_within obligations and array fact, vec_reserve and its loop-havoc exclusion) was audited once
with no findings.
