# P05M: MCP packet core, linear rewrite

Package P05M of docs/L3_RESTRUCTURE_SPEC.md §6.2. The tools/list page loop of `bench/l3/mcp` no longer copies the
whole packet once per tool and once per appended field. It computes the accepted descriptors and an output size
first, allocates once, and writes each accepted tool once via `inout`.

## Design

| piece | where | what it does |
|---|---|---|
| `plan` | packet.pruv | One pure pass over the tools array. It applies the same caps and order as the old loop: 1048576 elements, objects < 500, count < 500, name usability. Output is `[admitted, reserve, count_after, objects_after]` plus 10 slots per accepted tool: name, title and description spans, the readOnly/destructive values and the raw span. `reserve` is an upper bound of the payload words: the units for each string (capped) plus the raw bytes. |
| `fields4` / `key_is` | packet.pruv | One member walk answers `access::field` for up to 4 ASCII keys. It keeps `find_ascii` order and results: -4 for any malformed member, -3 for a bad id, -2 for a non-object, the last match wins, and keys compare like `jstr::ascii_equal`. A plain (no escape, ASCII) token is compared byte for byte; other tokens are decoded with `jstr::step`. |
| `usable_span` | packet.pruv | A streaming form of `usable(string_units(..))`. It has an ASCII fast path and stops after 201 units. |
| `write` | packet.pruv | `alloc(len(p)+reserve)` (clamped to 100000000), copies the prefix once, then for each descriptor calls `put_field` ×3, writes slots 6..9 and copies the raw bytes with `put_bytes`. It ends with `truncate(q, cursor)`. |
| `put_field` | packet.pruv | Applies the cap, the pair-safe cut (same rule as `cut`), the nonempty title rule and the `len(p) <= 100000000 - n` guard. When the room is proven it decodes with `put_units_room`; otherwise it uses the checked `put_units`. |
| root.pruv | root.pruv | `plan`, then `write` only when `admitted > 0`. The page storage (`d`) lives through `write`. |

The host still does only I/O. No host query was added. Nothing depends on G2/G3 or on new ownership features. The only
change in `bench/l3/pkgs/jstr` is visibility: `Step` and `step` are now `pub`.

Representation is unchanged on purpose. The packet stays a flat `[i64]` with UTF-16 units widened to i64, because
the host render reads that format. Principle 4 (no UTF-16-as-i64 records) is out of scope for P05M and stays a
separate package.

## Linearity counters

These counts come from an instrumented copy of the generated Rust (`gen.rs` with counters added; the source files
were not touched). The output checksums equal the uninstrumented run.

- Loop visits are the member walks, the plan loop, usable_span, ascii_equal, find_ascii and is_ascii. They exclude
  the store loops, which are counted as words written.
- Units written are the decoded string units. Raw bytes are the bytes copied by `put_bytes`.
- Prefix words are the words of the incoming packet copied by `write`.

| accepted tools | loop visits | units written | raw bytes copied | prefix words | out words |
|---:|---:|---:|---:|---:|---:|
| 0 | 28 | 0 | 0 | 0 | 5010 |
| 1 | 188 | 238 | 517 | 5010 | 5765 |
| 10 | 1628 | 2380 | 5181 | 5010 | 12571 |
| 50 | 8068 | 12100 | 26148 | 5010 | 43258 |
| 100 | 16118 | 24250 | 52356 | 5010 | 81616 |
| 250 | 40418 | 61450 | 131881 | 5010 | 198341 |
| 500 | 80918 | 123450 | 264423 | 5010 | 392883 |

- Each tool adds about 161.8 visits, 247 units and 529 raw bytes, so the cost is linear in records plus payload.
- At 500 tools, prefix + units + raw = 5010 + 123450 + 264423 = 392883, which equals the output size. Every output word
  is written exactly once.
- With 0 tools there is no copy, because `write` is skipped.
- The old code copied the whole packet for each tool and for each field: 3.97 GB allocated at 500 tools.

## Performance

`run_perf.py perf` runs on Apple M-series, release build (rustc -O). It reports the median ns and the instruction
count (`instr` mode). Every row is checked against the Python oracle (`run_perf.py check`, 0 mismatches on 18 cases),
and the rows' checksums must match before a number counts.

- NEW: the generated Rust of this package.
- HAND: perf/hand_packet.rs, a single-pass Rust baseline with the same input boundary (source bytes + tape + current
  packet + count/objects).
- OLD: the pre-rewrite packet.pruv, kept as perf/old_packet.pruv.

| cell (accepted tools) | NEW ns | HAND ns | ns ratio | NEW instr | HAND instr | instr ratio |
|---|---:|---:|---:|---:|---:|---:|
| 0 | 667 | 667 | 1.00x | 10625 | 10094 | 1.05x |
| 1 | 1291 | 834 | 1.55x | 22359 | 19704 | 1.13x |
| 10 | 4917 | 4459 | 1.10x | 88683 | 106208 | 0.83x |
| 50 | 21083 | 21584 | 0.98x | 385497 | 495610 | 0.78x |
| 100 | 38667 | 42792 | 0.90x | — | — | — |
| 250 | 95208 | 107083 | 0.89x | — | — | — |
| 500 | 199084 | 214417 | 0.93x | 3884592 | 4915026 | 0.79x |
| chain 5 pages × 100 | 384334 | 329958 | 1.16x | 5384713 | 6116428 | 0.88x |

- OLD at 500 tools: 190.9 ms, which is about 960x slower than NEW.
- In the steady state each tool costs about 7748 instructions in NEW and 9809 in HAND.
- The packet core target (≤0.95x) is met from 100 tools up. The 1.0x floor fails in these cells:
  - ns at 1 tool, 10 tools and the 5-page chain
  - instr at 0 tools and 1 tool

### Why the >1.0x cells are slower, line by line

Every remaining gap is in building the output buffer. The per-tool work is faster than hand (0.78–0.83x instr).

Generated Rust (`write`, from perf/work/gen.rs):

```rust
let mut q: Vec<i64> = { let pruv_n: i64 = want; let pruv_v: i64 = 0i64; vec![pruv_v; pruv_n as usize] };
let mut i: i64 = 0i64;
while (i < (p.len() as i64)) {
    { let pruv_i: i64 = i; let pruv_x: i64 = (unsafe { *p.get_unchecked((i) as usize) });
      unsafe { *q.get_unchecked_mut(pruv_i as usize) = pruv_x; } }
    i = (i + 1i64);
}
```

Hand Rust (`core_hand`):

```rust
let mut out: Vec<i64> = Vec::with_capacity(p0.len() + reserve);
out.extend_from_slice(p0);
```

| | generated | hand |
|---|---|---|
| allocation | `vec![0; want]`: calloc. A small block is zero-filled eagerly, and that includes the 5010-word prefix region. | `with_capacity`: malloc, no fill. |
| prefix | element-wise loop over the 5010 words, written over the zeros | one memcpy |
| payload | written by `put_*` over the zeros | `push` / `extend` |

At 1 and 10 tools the buffer is mostly prefix, so the zero fill plus the copy loop dominates. In the 5-page chain this
is paid once per page, and the packet grows each time.

With 0 accepted tools, NEW skips `write` but HAND still runs `extend_from_slice`, which is why that cell is 1.00x in
ns. Its 1.05x instr (531 instructions) is fixed setup: the plan header allocation and the walk over the root and
result. I did not break that cell down further.

**The experiment that proves the cause** (in /tmp, not committed). I replaced only those lines of the generated Rust
with `Vec::with_capacity(want); q.extend_from_slice(p); q.resize(want, 0)`:

| case | generated ns | experiment ns | hand ns |
|---|---:|---:|---:|
| 1 tool | 1250 | 833 | 875 |
| 10 tools | 4791 | 4208 | 4459 |
| 500 tools | 194125 | 185875 | 209666 |
| chain 5 × 100 | 371583 | 298167 | 315542 |

With that change every cell is at or below hand.

**Why Pruv source cannot emit it.** SPEC.md gives arrays only `alloc(n, v)` (`vec![v; n]`), `copy(a)` (an exact
copy, no extra room) and `truncate`.

- There is no grow, extend, with_capacity or uninitialized alloc.
- `Vec<T>` has push but no conversion to `[i64]`.
- `inout` cannot resize.

So "a copy of `p` with room for `n` more words" can only be written as alloc + an element loop. The audit agreed
with this.

**Proposed fix (compiler package, outside P05M scope).** Choose one:

1. Add an array builtin such as `grow(a, n, v)`: a copy of `a` with `len(a)+n` words, the tail set to `v`. Emit it as
   `with_capacity + extend_from_slice + resize`.
2. Recognise `alloc(n, v)` followed by a full-prefix copy loop `q[i] = p[i]` for `i < len(p)`, and fuse them into
   the same code.

Either change must go into both compilers (py, rs) and both backends (rust, llvm).

### Hand baseline caveats (from the audit)

- The hand walks the members separately for each key (6 walks per tool), while Pruv's `fields4` uses 2 combined
  walks. A combined hand walk would be a stricter floor and could raise the ratios in the per-tool cells.
- The hand does not validate the tape (no -4 path), so it does less work than Pruv.
- `key_eq` allocates for escaped keys. The perf cases do not exercise this path.
- perf_root.pruv rows include a `copy(p0)` that the real root.pruv does not do: `core_old` always and `core_new` when
  0 tools are admitted.

## Checks

| check | status | proof |
|---|---|---|
| per-unit `at + k < len(q)` before each store in the string decode | removed from `put_units_room` only | `requires at + cap <= len(q) \|\| at + (end - start) <= len(q)`, the loop invariants `k <= p - p0` and `k < cap`, and `jstr::step`'s `value > 65535 ==> next >= p + 2` (two units need two bytes). `put_field` tests the requires at run time once per field; when it fails, the checked `put_units` runs. |
| per-byte bound in the raw copy | `put_bytes` writes unchecked | `requires at + (e - s) <= len(q)`; the caller checks `rl <= len(q) - cursor` |
| `n <= len(q) - cursor` before a field is committed | added (defensive) | — |
| `rl <= len(q) - cursor` before the raw copy | added (defensive) | — |
| `len(p) <= 100000000 - n` and `rl <= 16000000` guards, title nonempty, description cap 65536, pair-safe cut | kept | — |
| `cnt - count < sz` in the plan admission | kept | redundant at run time but needed by z3 to prove the `res` writes |

No `trusted`, `assume` or `assert` was added.

Absent fields: the new code writes 0 to the len slot (and 0/0 to slots 8/9), while the old code left them untouched.
The record slots 10..5009 start as `alloc(5010, 0)`, and only the writer of record `count` touches that record, so the
output is identical. The Q differential confirms it.

## Results

| gate | result |
|---|---|
| `test_impl.py` (py/rust, py/llvm, rs/rust, rs/llvm) | 22/22 each |
| Q differential, 2040 cases × 4 pairs | identical to the pre-rewrite baseline: 2027 exact, 3 exact-decimal-id, 3 invalid-utf8, 3 byte-line-cap, 3 numeric-signal, 1 surrogate-safe-cut |
| `q_mutants.py` | 12/12 killed. page-contract and tool-contract are rejected by the verifier; the other 10 are differential-killed. |
| jstr consumers (codex, grok, opencode) | pruvc verify ok |

q_mutants anchors moved with the code:

| mutant | new anchor |
|---|---|
| drop-title | `"title"` → `"absentTitle"` (one occurrence, in the `fields4` call) |
| description-cap | `{ return 65536; }` in `desc_cap` |
| readonly | `q[base+6]=1;` |
| destructive | `q[base+7]=dv;` |

## How to run

```sh
python3 bench/l3/mcp/perf/run_perf.py check   # oracle check, all cases
python3 bench/l3/mcp/perf/run_perf.py perf    # ns + instr table (NEW / HAND / OLD)
python3 bench/l3/mcp/perf/run_perf.py all
```

The harness compiles perf_root.pruv with pruvc. It includes hand_packet.rs into perf_driver.rs and writes its build
output under `perf/work/`, which is not committed.
