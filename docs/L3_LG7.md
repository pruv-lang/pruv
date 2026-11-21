# L3 LG7: bitwise operators, LLVM struct field GEP access, `ev_` read-into API

Scope: G-DB8-1, G-DB9-1, G-DB9-2 and (time permitting) G-LG6-1 from `docs/L3_RESTRUCTURE_SPEC.md` §7.5.1.
Both compilers (Python `pruvc/`, Rust `pruvc-rs/`) change with the same semantics and diagnostics, on both backends
(Rust source and LLVM). No `bench/` or `pkgs/` fragment is changed; G-LG6-2, G-R03D-2 and G-LG3-3 are out of scope.

## What landed

- Bitwise operators (G-DB8-1, SPEC.md §5 "Bitwise operators (LG7)"). `a & b`, `a | b`, `a ^ b`, `a << k`, `a >> k`
  and `~a` on `i64` (any other operand type is `type/type_mismatch`). Precedence, loosest first: comparisons, `|`,
  `^`, `&`, `<< >>`, `+ -`, `* / %`; comparisons still cannot be chained and `==>` stays right-associative. `>>` is
  lexed as two adjacent `>` tokens so that nested generics keep parsing.
  - Verifier: the operands are encoded as 64-bit bit-vectors of the integer value (`int2bv`), the result is read back
    as signed (`signed(w) = if bv2int(w) < 2^63 then u else u - 2^64`). `a >> k` is arithmetic (`bvashr`); with a
    literal `k` it is the cheaper `floor(a / 2^k)`, which equals the arithmetic shift. Facts kept for cheap proofs:
    `a & m` with a literal non-negative mask `m` is `a mod (m + 1)` when `m + 1` is a power of two, else in `0..m`.
  - Obligations: a shift count outside `0..64` is `verify/shift_range` ("shift count in 0..64 in `<<`"), never
    undefined behaviour; `a << k` whose result leaves `i64` is `verify/overflow` (checked as `(r >>a k) == a`).
  - Rust backend: plain `& | ^ ! << >>` on `i64` when verified; under `--no-verify` `pruv_shl_chk` / `pruv_shr_chk`
    (runtime "shift count outside 0..64" / "overflow in <<"). In runtime contracts (i128 values) the helpers
    `pruv_c{and,or,xor,shl,shr}_op` are undefined ("argument outside i64 in op") outside `i64`.
  - LLVM backend: `and/or/xor/shl/ashr`, `xor -1` for `~`; under `--no-verify` an `icmp` range check and the same
    panic messages. Python and Rust LLVM IR are byte-identical; Rust source output is identical.
- LLVM struct field access by GEP (G-DB9-1). A field read is `getelementptr inbounds %pruv_t_S, ptr, 0, i` plus a
  scalar `load`; a field write is GEP plus `store`, nested fields chain GEPs. No whole-struct `load` /
  `insertvalue` / `store` remains on field paths (only the struct literal still builds an aggregate). The byte loop of
  `work/lg7perf/field.pruv` shows 11 GEPs and no struct load in the loop body.
- `ev_` read-into API (G-DB9-2, SPEC.md async section). `ev_read_into(take l, slot, inout dst: [u8], inout n: i64)`
  and `ev_proc_read_into(take l, slot, stream, inout dst, inout n)` copy queued bytes straight into `dst[0..n)` of the
  caller's buffer. The verifier guarantees `0 <= n <= len(dst)`, keeps `len(dst)` and the public `ev_bytes` buffer,
  and `eof ==> n == 0` (but `n == 0` alone does not mean end of file). EOF and error rules are those of
  `ev_read` / `ev_proc_read`, which keep working unchanged; no fragment changes.
  - Runtime (both preludes byte-identical): one work buffer per reader thread, reused (64 KiB socket, 8 KiB pipe);
    read-ahead per stream is bounded at 1 MiB, the reader stops when the queue is full and resumes when it drains;
    one wake-up per data batch, and an empty read wakes only when it posts EOF or an error.
  - LLVM: the `inout i64` count is passed as a pointer to its place; `dst` is the caller's array box.

| Row | Status | What exists now |
|---|---|---|
| G-DB8-1 | closed | Operators in both compilers and backends; `tests/runtime/lg7_rt_bitops`, `tests/must_reject/lg7_{bitop_bool,shift_out_of_range,shl_overflow,shr_negative_count}`. |
| G-DB9-1 | closed | GEP field read/write on LLVM in both compilers; `tests/runtime/lg7_rt_field_gep` (same output on both backends). |
| G-DB9-2 | closed | `ev_read_into` / `ev_proc_read_into` in both compilers and backends; `tests/runtime/lg7_rt_read_into` (process and socket), `tests/must_reject/lg7_read_into_bad_n`. |
| G-LG6-1 | open | Not attempted (see G-LG7 rows). |

Status: LG7: G-DB8-1 closed (`& | ^ << >> ~` on i64 in both compilers and backends; out-of-range shift count is an obligation; `<<` overflow is an obligation).
Status: LG7: G-DB9-1 closed (LLVM field read = GEP + scalar load, field write = GEP + store; no whole-struct load/insertvalue/store on field paths).
Status: LG7: G-DB9-2 closed (`ev_read_into` / `ev_proc_read_into` read into a caller buffer, read-ahead bounded at 1 MiB per stream, one wake per batch; `ev_read` / `ev_proc_read` unchanged).
Status: LG7: G-LG6-1 open (inout `Vec<T>` on LLVM not attempted; time cap, and the llvmgen files were held by the G-DB9-2 change).

## New gaps

| Gap | What | Why not here |
|---|---|---|
| G-LG7-1 | `inout Vec<T>` parameters on the LLVM backend (carried from G-LG6-1). | Not attempted within the 2 h cap; the llvmgen files were being changed by the G-DB9-2 work. |

## Performance

`work/lg7perf/field.pruv` (200M iterations over a nested struct field, LLVM backend, clang -O2), 15 interleaved runs,
user time: before median 0.86 s (0.85-0.87), after median 0.86 s (0.85-0.87), same output (`534942`, `200000000`).
At -O2 SROA already removed the whole-struct round trip in this loop, so the change is an IR cleanup with no measured
speedup (an earlier 7-run set gave 0.78 s / 0.80 s, inside run-to-run noise).

`work/lg7perf/rd/{old,new}.pruv` (drain 512 MiB of child stdout through the event loop, LLVM backend, clang -O2),
7 interleaved runs on the after build, wall time: `ev_proc_read` + `ev_size` (old API) median 0.80 s (0.59-0.95),
`ev_proc_read_into` with one reused 64 KiB buffer median 0.23 s (0.19-0.27); user time 0.51-0.79 s vs 0.07-0.12 s;
same byte count (536870912). The old API also runs on the new bounded runtime here, so this compares the two APIs,
not the old build. Cells: `work/lg7perf/perf_cells.json` (`tools/perf_gate.py`: PASS, 2 cells; not committed).

## Tests run

- Full suite, `tests/run_all.py --backend both`: Rust compiler 4712/4712, Python compiler 4700/4700 (after the last
  shared-code change; includes `g8_frontend`, `runtime` with the socket and process paths of `lg7_rt_read_into`, and
  `must_reject` with the five new LG7 rejections).
- `cargo test --release`: 150 passed, 0 failed.
- `tests/diff_rust.py --fuzz-count 20`, `tests/diff_llvm.py`, `tests/diff_compilers.py`: all same (diff_compilers 4666
  cases, 52 differing only in counterexample values).
- Audit (one pass, read-only): no must-fix. Note: the Python and Rust verifiers add a slightly different set of extra
  true facts for `literal & mask` (Python both sides, Rust the right side) and for `x >> 64` (Rust also adds range
  facts after the failing `shift_range` obligation); obligations and diagnostics are the same.
