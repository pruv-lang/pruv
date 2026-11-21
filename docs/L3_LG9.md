# L3 LG9: take record ABI on LLVM, consuming field moves, str binder return, ASCII utf8 fact

Scope: G-LG8-5, G-LG8-3, G-LG8-1, G-DB15-1, G-DB15-2 and G-LG8-4 from `docs/L3_RESTRUCTURE_SPEC.md` §7.5.1. Both
compilers (Python `pruvc/`, Rust `pruvc-rs/`) change with the same semantics and diagnostics, on both backends (Rust
source and LLVM). No `bench/` fragment is changed; G-R03D-2, G-LG3-3 and G-DB6-1 are out of scope.

## What landed

- `take` record ABI on LLVM (G-LG8-5, SPEC.md R-6e-12). A `take` parameter of an R-6e-6 record (buffer fields
  `{ptr, i64, i64}`) is now a pointer to storage the callee owns. The caller spills a fresh copy of the moved value
  and passes its address, and the moved source slot is zeroed whole (before, the caller passed its own slot after
  nulling only the first word, so the callee read a half-cleared record). The callee binds the parameter as an owned
  slot and drops it at exit unless it moved it out; the parameter is no longer marked `readonly`. A borrowed record
  parameter is unchanged (a read-only view). Rust source is unchanged apart from the `mut` below.
  Code: `pruvc/llvmgen.py` `sig_text`, parameter binding, `Move` of an extended record, call arguments;
  `llvmgen.rs` the same places.
- Consuming field move from a `take` record parameter (G-LG8-3, SPEC.md R-6e-16 consuming paragraph). With the
  owned ABI above, `let t = move p.f;` of a `take` record parameter consumes `p` like a `let mut` local record. The
  Rust backend declares such a parameter `mut p: P` so `::std::mem::take(&mut p.f)` compiles.
  `tests/must_reject/lg8_move_field_take_param` now rejects at the later use of `p` (line 4) instead of at the move.
- Consuming field move extended (G-LG8-1, SPEC.md R-6e-16). The consuming form now covers every field type R-6e-16
  moves (`String`, `Vec<i64>`, `Vec<bool>`, `[i64]`, `[u8]`, `[bool]`), nested struct field paths, and
  `f(move d.f)` as a `let` initializer, a call statement or a `return` value when the field move is the direct
  argument of a `take` parameter of `f`, the only field move in the call, and no other argument names `d`
  (otherwise the old errors stay). The move tracker (`foreign.py` / `foreign.rs`) marks `d` moved after such a
  call. `p.g = move p.f;`, plain / `inout` roots and match binders keep their errors. LLVM has no records with
  `String` / `Vec` fields (pre-existing), so those cases are Rust-backend only; buffer fields at nested paths work on
  both backends.
- `return x` of a `str` match binder (G-DB15-2, SPEC.md R-6e-12a LG9 paragraph). Under the R-6e-12a conditions
  (owned local scrutinee of a non-generic enum, no arm names it), `return x` of a `str` binder is allowed and means
  exactly `return copy(x)`: it needs `alloc`. The checker skips the borrowed-source check for it and rewrites the
  return value into the `copy(x)` call, so both backends and the cost proof see the copy (a function with
  `cost 6` over it accepts, `cost 5` rejects with `verify/cost_bound`).
- ASCII `utf8_valid` fact (G-DB15-1, SPEC.md §6b `utf8_valid`). The verifier model of `utf8_valid(b)` is now
  `ascii(b) || utf8_valid!(b)` with `ascii(b) = forall k in 0..len(b): b[k] < 128`. This is sound (ASCII bytes are
  valid UTF-8, so reading `utf8_valid!` as the real check makes the model equal the real predicate) and lets a
  byte-level proof establish `utf8_valid`, so `str_from_utf8` of a span proved ASCII has no runtime check (Rust
  `from_utf8_unchecked`, no `pruv_utf8_valid` call on LLVM). Multi-byte sequences are not modelled (G-LG9-1).
- Whole-record move on LLVM. Moving an extended record local out (`let b = move a;`, a `take` argument) now zeroes
  all three words of the source slot instead of storing `null` in the first word only. This is an LLVM IR change
  outside `take` parameters; it is needed so the drop at scope exit of the moved slot sees an empty record.
- `docs/L3_LG3.md:30` (G-LG8-4) now says the `inout Vec<T>` rejection was closed by LG8 (G-LG7-1) and the test
  `lg3_inout_vec_reserve` was removed, instead of citing that removed test as current.

| Row | Status | What exists now |
|---|---|---|
| G-LG8-5 | closed | Owned `take` record ABI on LLVM in both compilers; `tests/runtime/lg9_rt_take_record`, `lg9_rt_record_move` (both backends). |
| G-LG8-3 | closed | Consuming field move from a `take` record parameter; `tests/runtime/lg9_rt_take_field_move` (both backends), `tests/must_reject/lg8_move_field_take_param` (use after). |
| G-LG8-1 | partial | String/Vec fields, nested paths, `f(move d.a)`; `tests/runtime/lg9_rt_field_move_kinds` (Rust only), `lg9_rt_field_move_nested`, `lg9_rt_field_move_arg` (both), `tests/must_reject/lg9_move_field_arg_{use_after,other_mention,borrowed}`. String/Vec record fields stay Rust-only (LLVM has no such records). |
| G-DB15-1 | partial | ASCII fact: a proof that every byte is below 128 gives `utf8_valid`; `tests/runtime/lg9_rt_utf8_ascii`, `tests/must_reject/lg9_utf8_ascii_partial`. Multi-byte UTF-8 rules are G-LG9-1. |
| G-DB15-2 | closed | `return x` of a str binder of a consuming match; `tests/runtime/lg9_rt_match_str_return` (both; includes a fall-through first match and a cost-bounded function), `tests/must_reject/lg9_match_str_return_{param,named}`. |
| G-LG8-4 | closed | `docs/L3_LG3.md:30` citation fixed. |

Status: LG9: G-LG8-5 closed (LLVM take record parameter is callee-owned storage: caller spills a copy and zeroes the moved slot, callee drops it).
Status: LG9: G-LG8-3 closed (`let t = move p.f` of a take record parameter consumes p; later use is type/foreign_move).
Status: LG9: G-LG8-1 partial (consuming field move for String/Vec fields, nested paths and `f(move d.a)` to a take parameter; String/Vec record fields remain Rust-backend only).
Status: LG9: G-DB15-1 partial (verifier knows all-bytes-below-128 implies utf8_valid; multi-byte UTF-8 rules not modelled, see G-LG9-1).
Status: LG9: G-DB15-2 closed (`return x` of a str binder of a match on an owned local enum means `return copy(x)`; no source copy needed).
Status: LG9: G-LG8-4 closed (docs/L3_LG3.md:30 no longer cites the removed test as current).

## New gaps

| Gap | What | Why not here |
|---|---|---|
| G-LG9-1 | A full byte-level UTF-8 definition for `utf8_valid` (lead byte ranges, continuation counts, the E0/ED/F0/F4 second-byte limits), so a scanner proof of a multi-byte span gives the fact. Only the ASCII case is modelled. | A quantified definition over sequence starts is a larger verifier change in both proof engines and needs a scanner whose invariants are in that shape; the ASCII case is sound and small. |
| G-LG9-2 | LLVM records with `String` / `Vec` fields, so the String/Vec consuming field moves run on LLVM. | Pre-existing LLVM limit (`codegen/unsupported_type`), not part of the LG9 rows. |
| G-LG9-3 | A field move inside a nested call argument (`print(f(move d.a))`) or with two field moves in one call stays `type/array_alias` / `type/foreign_move`. | Kept to the direct-argument form the rows asked for. |

## Tests run

- `tests/run_all.py --backend both`: Python compiler 4760/4761, Rust compiler 4772/4773. The one failure in each was
  `tests/spec/r6e_2_move_out.pruv`: its `let mut` root is now the accepted consuming form, so the reject case was
  changed to a plain `let` root (both compilers reject it with `type/foreign_move` at 7:21) and
  `--only spec_trace` passes on both compilers (1/1).
- New LG9 rows on both compilers: `tests/runtime/lg9_*` 30/30, `tests/must_reject/lg9_*` 6/6,
  `tests/must_reject/lg8_move_field*` 5/5.
- `--only g8_frontend`: Python 27/27, Rust 39/39. `cargo test --release`: all ok.
- `tests/diff_rust.py --fuzz-count 20`, `tests/diff_llvm.py`: all same. `tests/diff_compilers.py`: 4746 cases, 0
  differ (54 differ only in counterexample values).
- `tests/run_all.py --backend both --list-rows` equals the committed `tests/run_all_rows.txt` (new rows are the lg9
  tests only).
- IR of existing programs: every `examples/`, `bench/` and `tests/runtime/` program that is not an lg9 test
  (1013 files; 697 compile to Rust, 690 to LLVM) gives byte-identical Rust and LLVM output before and after LG9.
