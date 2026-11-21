# L3 LG3: LLVM backend parity (third language bundle)

Scope: G-R02-1, G-R09-4, G-R08-1, G-R09-5, G-R07-1, G-R00-1 and G-R09-3 from `docs/L3_RESTRUCTURE_SPEC.md` §7.5 /
§7.5.1. These are the items where the LLVM backend rejected a program that the Rust backend accepts. Both compilers
(Python `pruvc/`, Rust `pruvc-rs/`) change in one commit with the same semantics and diagnostics. No `bench/l3/`
fragment is changed here.

## New LLVM conventions (internal functions only)

- `take` array parameter: the IR signature is `(ptr data, i64 len, i64 cap)` with no `readonly` / `nocapture` on
  `data`. The callee owns the buffer in a slot. It frees the buffer at scope end unless the buffer was moved (into a
  field, a return value or another `take`). The caller passes the array's capacity and gives up the value, as on Rust.
  A function with a take array parameter gets no host ABI wrapper and no `--rust-bindings` item.
- `inout` struct parameter: the IR signature is `ptr noalias nocapture` to the caller's record. Field reads and writes
  go through that pointer, and a resize of an array field writes the new `{ptr, len, cap}` in place. At the call site
  the place (`s`, `s.sub`, `d.sub.inner`) is resolved to a field pointer. Contract evaluation loads the record from
  the pointer.
- `vec_reserve::<T>(inout v, n);` on a local `Vec<i64>` or `Vec<bool>` lowers to a shim adapter that calls
  `Vec::reserve(n as usize)`. When the build keeps catalog checks (`--no-verify`, `--foreign-checks all`, or a program
  with foreign functions), the adapter checks `0 <= n && n <= 2^40` first and exits 101 like the other catalog
  guards. In proof mode the verifier has already proved that range.

| Row | Status | What exists now |
|---|---|---|
| G-R02-1 | closed | `fn wrap(take a: [i64]) -> D { return D { a: move a }; }` builds on LLVM for `[i64]`, `[u8]` and `[bool]`, with no copy. `tests/runtime/lg3_rt_take_field` (both backends, proof and `--no-verify`, allocation balance checked). On a host entry (`pub` root fn) LLVM still rejects with `crate::f has a take array parameter, which needs the Rust backend` (`tests/must_reject/lg3_host_take_array`). |
| G-R09-4 | partial | Moving an array into a field works on LLVM (G-R02-1). Moving an array field out, `move d.a`, is rejected by the common frontend on both backends: `move` takes a local only. That is a language change, see G-LG3-1. Workaround: `copy(d.a)`, or pass `inout d.a` to a helper. |
| G-R08-1 | closed | Internal `inout` struct parameters work on LLVM: read and write scalar fields, nested struct fields (`inout d.sub`), resize of an array field in place (`st.a = resize(st.a, n, 0)` through the pointer), forwarding the parameter to another `inout` call. `take [u8]` works (G-R02-1). `tests/runtime/lg3_rt_inout_struct` (both backends, both flag sets). |
| G-R09-5 | partial | Inout struct with an owned array field works on LLVM for internal functions (G-R08-1). An element write through a field, `s.a[i] = v`, is not accepted by the parser on either backend; pass `inout s.a` to a helper that writes `a[i] = v` (tested in `lg3_rt_inout_struct`). See G-LG3-2. |
| G-R07-1 | open | A host entry with an `inout` struct parameter stays rejected on LLVM (`crate::f has an inout struct parameter, which needs the Rust backend`, `tests/must_reject/lg3_host_inout_struct`) and in `--rust-bindings`. The internal convention exists now, but the host side needs a `#[repr(C)]` record layout and a binding wrapper that keeps owned array fields valid across the boundary. Not cheap inside this bundle, see G-LG3-3. |
| G-R00-1 | closed (local Vec) | `vec_reserve` on a local `Vec<i64>` / `Vec<bool>` works on LLVM (`tests/runtime/lg3_rt_vec_reserve`, both backends, both flag sets). At LG3 an `inout Vec<T>` parameter was still rejected on LLVM, see G-LG3-4. LG8 closed that (G-LG7-1): internal `inout Vec<T>` works on LLVM, `tests/runtime/lg8_rt_inout_vec`, and the old rejection test `lg3_inout_vec_reserve` was removed. |
| G-R09-3 | open | A reserve-only tail for `[T]` arrays stays blocked by §6g (a `[T]` slot always has a value in the verifier model). §6g is not changed. The LLVM `Vec` path above is the available route. |

## New gaps

| Row | Gap | Reason |
|---|---|---|
| G-LG3-1 | `move d.a`: moving an owned array field out of a record. | The frontend allows `move` on locals only. Allowing it on a field needs a partial-move rule (the record is then unusable or must get the field back) in the checker and the verifier on both compilers: a language change, outside a parity bundle. |
| G-LG3-2 | `s.a[i] = v`: an element write through a field place. | The parser accepts only `x[i] = v` with a local `x`. Needs an assignment-place grammar change plus the verifier write rule for nested places. Workaround: `inout s.a` helper. |
| G-LG3-3 | `inout` struct on a host entry (LLVM ABI symbol and `--rust-bindings`). | Needs a C record layout for structs with owned array fields and a binding wrapper that hands the host a stable pointer, then writes back. Over the 2 h cap; the internal convention is the base for it. |
| G-LG3-4 | `inout Vec<T>` parameter on LLVM. | `vec_reserve` lands for local `Vec`s only. A `Vec` passed by reference needs a pointer-to-`Vec` convention for every R-6e op on LLVM, not only `vec_reserve`. |
| G-LG3-6 | An internal `take` array with a refined element type (`take a: [i64 \| 0..=255]`) keeps the 8-byte storage width. | `storage.py` / `storage.rs` treat a take array parameter like a host array, so the narrow-width pass skips it. Correct but larger; not measured. Found by the bundle audit (advisory). |
| G-LG3-5 | A cost expression that reads a plain struct field of an `inout` struct parameter was not run-tested on LLVM. | Excluded from `lg3_rt_inout_struct` because the Rust-source backend has an existing reference issue with that form. The LLVM path loads the record for contracts, but no test covers it. |

Status: LG3: G-R02-1 closed (internal take array params on LLVM for `[i64]`/`[u8]`/`[bool]`; host entries reject).
Status: LG3: G-R09-4 partial (move into a field works; `move d.a` out is G-LG3-1).
Status: LG3: G-R08-1 closed (internal inout struct params on LLVM incl. array-field resize; take `[u8]`).
Status: LG3: G-R09-5 partial (internal only; `s.a[i] = v` is G-LG3-2).
Status: LG3: G-R07-1 open (host entry inout struct and `--rust-bindings` still reject; G-LG3-3).
Status: LG3: G-R00-1 closed (local `Vec<i64>`/`Vec<bool>` `vec_reserve` on LLVM; `inout Vec` param is G-LG3-4).
Status: LG3: G-R09-3 open (blocked by §6g, unchanged).
Status: LG3: G-LG3-1 open (move of an array field out of a record).
Status: LG3: G-LG3-2 open (element write through a field place).
Status: LG3: G-LG3-3 open (host entry inout struct ABI and bindings).
Status: LG3: G-LG3-4 open (inout `Vec<T>` params on LLVM).
Status: LG3: G-LG3-5 open (cost expression on an inout struct field not run-tested on LLVM).
Status: LG3: G-LG3-6 open (internal take arrays keep the wide storage width).

## Verification

Audit (one pass) found four required issues, all fixed in this commit: Python emitted invalid IR for `size(c)` of an
`inout` struct in a contract (it now uses the caller's pointer, as Rust); Rust reported an unsupported `vec_reserve`
element type at the `let` instead of the call; the two compilers reported a different first host-entry ABI error when a
program had both a take array and an inout struct entry (Rust now checks in declaration order,
`bindings::llvm_param_abi_error`); the `vec_reserve` shim adapters came out in a different order (Python now sorts by
helper name, as Rust).

New tests: `tests/runtime/lg3_rt_{take_field,inout_struct,vec_reserve}`,
`tests/must_reject/lg3_{host_take_array,host_inout_struct,inout_vec_reserve}` (LLVM backend only; `run_all.py` runs
`lg3_*` reject projects with `--backend llvm`).

| Check | Result |
|---|---|
| `run_all.py --only runtime --sample-prefix tests/runtime/lg3 --backend both` (py, rs) | 12/12 each |
| `run_all.py --only must_reject --sample-prefix tests/must_reject/lg3` (py, rs, LLVM) | 3/3 each |
| `run_all.py --only g8_frontend` | py 27/27, rs 39/39 |
| lg2 runtime and g6 on LLVM (py, rs) | 7/7 and 181/181 each |
| `cargo test --release` | 150 passed, 0 failed |
| `tests/diff_rust.py --fuzz-count 20` | all same |
| `tests/diff_compilers.py --backend both` on the lg3 runtime inputs plus the audit repros (default, `--no-verify`) | 10/10 each, no diff |
| `run_all.py --compiler py` (full suite, before the audit fixes) | 3216/3216 |
| `run_all.py --backend both --list-rows` vs `tests/run_all_rows.txt` | 15 new rs rows (lg3), none removed |
