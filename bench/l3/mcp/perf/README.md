# P05M: MCP packet core, linear rewrite

> **Superseded (RUST_SHAPE_RESET §6 step 4.1).** `plan`/`write` are gone. The product path is now one pass,
> `packet::page(inout p, ..)`, in the shape of `hand_packet.rs` `core_hand`; `core_new` in `perf_root.pruv` wraps it.
> See `bench/l3/mcp/RESHAPE.md` for the structure table, the results and the language gaps. The text below describes
> the P05M design and is kept as history; the tool (`run_perf.py check|counters|perf|all`) is unchanged in use.

Package P05M of docs/L3_RESTRUCTURE_SPEC.md §6.2. The tools/list page loop of `bench/l3/mcp` no longer copies the
whole packet once per tool and once per appended field. It computes the accepted descriptors and an output size
first, allocates once, and writes each accepted tool once via `inout`.

## Design

Driver (LB5b): the new and hand rows both own their input packet (a copy of the start packet made outside the timed call) and free it inside the call; the one-call RSS probe hands both a fresh start packet. Before, hand borrowed the start packet on page 0 while the new row freed its input, and the probe kept a dead start copy alive for the new row.

Historical (P05M). The current `packet.pruv` follows `hand_packet.rs` helper by helper (`decode`, `key_eq`,
`field`, `put_field`, `page`); see RESHAPE.md "LB5b". The names below (`plan`, `write`, `fields4`, `usable_span`,
`put_units_room`) no longer exist.

| piece | where | what it does |
|---|---|---|
| `plan` | packet.pruv | One pure pass over the tools array. It applies the same caps and order as the old loop: 1048576 elements, objects < 500, count < 500, name usability. Output is `[admitted, reserve, count_after, objects_after]` plus 10 slots per accepted tool: name, title and description spans, the readOnly/destructive values and the raw span. `reserve` is an upper bound of the payload words: the units for each string (capped) plus the raw bytes. |
| `fields4` / `key_is` | packet.pruv | One member walk answers `access::field` for up to 4 ASCII keys. It keeps `find_ascii` order and results: -4 for any malformed member, -3 for a bad id, -2 for a non-object, the last match wins, and keys compare like `jstr::ascii_equal`. A plain (no escape, ASCII) token is compared byte for byte; other tokens are decoded with `jstr::step`. |
| `usable_span` | packet.pruv | A streaming form of `usable(string_units(..))`. It has an ASCII fast path and stops after 201 units. |
| `write` | packet.pruv | `resize(p, len(p)+reserve, 0)` (clamped to 100000000; P05G): one allocation that copies the prefix once and zero-fills only the tail. Then for each descriptor calls `put_field` ×3, writes slots 6..9 and copies the raw bytes with `put_bytes`. It ends with `truncate(q, cursor)`. |
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

`run_perf.py perf` runs on Apple M-series, release build (rustc -O). It reports the median ns, a trimmed mean ns
(slowest 5% of calls dropped; the per-call median is quantized to the 41.67 ns timer tick, which hides small-cell
differences) and the instruction count (`instr` mode). Every row is checked against the Python oracle (`run_perf.py check`, 0 mismatches on 18 cases),
and the rows' checksums must match before a number counts.

- NEW: the generated Rust of this package.
- HAND: perf/hand_packet.rs, a single-pass Rust baseline with the same input boundary (source bytes + tape + current
  packet + count/objects).
- OLD: the pre-rewrite packet.pruv, kept as perf/old_packet.pruv.

### After P05N (`has_work` skips the plan when no tool can be looked at)

`packet::has_work(d, tools, objects)` is `objects < 500 && array_length(d, tools) > 0`. When it is false, `plan`'s loop
cannot run, so `plan` would return `[0, 0, count, objects]` and `write` would not be called; `root.pruv` and
`perf_root.pruv` now skip both. The 0-tool cell loses the plan header allocation (alloc_count 2 → 1, like HAND).

Generated vs HAND, step by step (cells 0 / 1 / 10):

| step | NEW (perf/work/gen.rs) | HAND (hand_packet.rs) |
|---|---|---|
| `result`, `tools` lookup | `access::field` ×2: one member walk each, every member validated (kind 7, parent, next, span), plain keys compared by length then bytes | `field` ×2: one member walk each, span checked, plain keys compared as slices |
| array size | `array_length`: O(1) tape read | kind + stop check, span → `reserve = 2 * array bytes` |
| 0 tools, before | `plan`: `alloc(4, 0)` (malloc + free), checks, empty loop; then `copy(p0)` | `with_capacity(5010 + 4)` + `extend_from_slice(p0)` |
| 0 tools, after | `has_work` false → `copy(p0)` | same as before |
| member lookups per tool | `fields4`: 1 walk for name/title/description/annotations + 1 walk for the two hints | 6 walks (one per key) |
| descriptors | `alloc(4 + 10 * sz, 0)`: a second, small allocation | none (decodes straight into the output) |
| output buffer | `resize(p0, len + reserve, 0)`: malloc, memcpy of the prefix, **memset of the reserved tail** | `with_capacity(len + 2 * array bytes)`, memcpy of the prefix, tail left uninitialized |
| strings | `put_units_room`: stores in bounds by proof, plain-byte fast path | `decode` with `Vec::push` (capacity test per unit), bounds-checked `src[p]` |
| raw JSON | `put_bytes`: proven indices | `extend(raw.iter().map(..))` |
| end | `truncate` (no copy) | nothing |

Causes, in the order data representation → language gap → codegen:

1. **Representation (fixed).** The 0-tool cell paid the plan header allocation for a plan that could not admit
   anything (the +524-instruction gap). `has_work` removes it by structure; no check was weakened on either side.
   A plan returned as a struct (scalar header + descriptor buffer) was tried and rejected: a buffer field must be a
   fresh `alloc`/`copy`/call result (`type/array_alias`), so moving the filled descriptor local into the struct would
   cost a copy.
2. **Language gap (not fixable under bench/l3/mcp, proposal only).** `resize` must give a fully defined array, so the
   reserved tail is zero-filled and then overwritten; HAND only reserves capacity and pushes. Measured by patching the
   generated `resize` in a copy of gen.rs to `with_capacity + extend_from_slice + set_len` (unsound, measurement only),
   7 alternating runs, trimmed mean ns:

   | cell | NEW | NEW without tail fill | HAND |
   |---|---:|---:|---:|
   | 1 | 801 | 815 | 810 |
   | 10 | 4064 | 3586 | 4275 |

   Proposal for the compiler/SPEC owners: an append form for owned arrays (e.g. a move conversion `Vec<i64>` →
   `[i64]`, both are `Vec<i64>` in Rust, so `vec_push` with a capacity hint could build the packet), or lowering
   `resize` without the fill when every tail index is proven written before the first read. The descriptor buffer
   (second allocation in the 1+ tool cells) would also go away with a move of a fresh local into a constructor field.
3. **Codegen.** No difference found beyond the two above.

Why the 1-tool cell stays at ~0.99x ns (same work): the 0-tool time (≈670 ns: the 5010-word prefix memcpy plus the two
lookups, identical on both sides) is ≈80% of the 1-tool time. The per-tool part is NEW 8131 vs HAND 9596
instructions (0.85x) and about equal in ns; the tail-fill experiment above shows no measurable 1-tool gain, so the
remaining difference is inside timer noise. The HAND checks that proof can remove (per-unit capacity tests, `src[p]`
bounds checks) are already gone from NEW. The prefix copy itself is required at this boundary (`&[i64]` in, new
array out) for both sides; removing it needs in-place growth of an owned array (`p = write(p, ...)` in root.pruv), the
same language gap as above.

Results (`run_perf.py all --rounds 5 --time-ms 200`, oracle check 0 mismatches on 18 cases, checksums agree; load
average ≈3.5 from other work, so cells were measured alternating rows). "Before" for 0/1/10 is this package's baseline
run on 5fa5a38 (7 rounds, same machine state); the larger cells' "before" is the P05G table below.

| cell | NEW ns before → after | HAND ns | ns ratio before → after (tmean) | NEW instr before → after | HAND instr | instr ratio before → after |
|---|---:|---:|---:|---:|---:|---:|
| 0 | 708 → 667 (671) | 667 (684) | 1.06x → 1.00x (0.98x) | 10611 → 10015 | 10094 | 1.05x → 0.99x |
| 1 | 792 → 833 (828) | 833 (833) | 0.95x → 1.00x (0.99x) | 18095 → 18146 | 19690 | 0.92x → 0.92x |
| 10 | 4250 → 4291 (4224) | 4416 (4372) | 0.96x → 0.97x (0.97x) | 83961 → 84597 | 106157 | 0.79x → 0.80x |
| 50 | 19750 → 19750 | 21250 | 0.92x → 0.93x | 381498 → 384703 | 495341 | 0.77x → 0.78x |
| 100 | 37375 → 37416 | 42250 | 0.89x → 0.89x | 753158 → 760148 | 982126 | 0.77x → 0.77x |
| 250 | 92750 → 92958 | 105333 | 0.88x → 0.88x | 1881915 → 1898886 | 2456215 | 0.77x → 0.77x |
| 500 | 187250 → 187375 | 211042 | 0.89x → 0.89x | 3765450 → 3799096 | 4917756 | 0.77x → 0.77x |
| chain 5 × 100 | 303708 → 305875 | 324875 | 0.94x → 0.94x | 4965993 → 4999450 | 6115676 | 0.81x → 0.82x |

- Every cell is now ≤1.0x in both ns and instructions (the 0-tool cell was the last instr cell above 1.0x).
- The 1-tool median moves between 792 and 833 ns run to run (one timer tick); the trimmed mean is the stable figure.
- 1 and 10 tools stay above the 0.95x packet-core target for the reasons above (shared prefix copy; tail fill).
- HAND is unchanged.
- NEW instructions in the 10+ tool cells are up to 0.9% higher than before (the per-tool code is unchanged; most
  likely inlining/layout around the new wrapper, not investigated further). Every ratio stays ≤0.82x.
- Gates: test_impl.py 22/22 in all four pairs, Q differential identical to the baseline below, q_mutants 12/12.

### After P05G (`write` uses `resize`)

Generated Rust for the new `write` allocation (perf/work/gen.rs):

```rust
let mut q: Vec<i64> = { let pruv_a = p; let pruv_n: i64 = want; let pruv_v: i64 = 0i64;
    let mut pruv_t = Vec::with_capacity(pruv_n as usize); pruv_t.extend_from_slice(pruv_a);
    pruv_t.resize(pruv_n as usize, pruv_v); pruv_t };
```

This is the hand shape: malloc without a fill, one memcpy of the prefix, and a fill of the tail only. The oracle check
gives 0 mismatches on 18 cases.

| cell (accepted tools) | NEW ns | HAND ns | ns ratio | NEW instr | HAND instr | instr ratio |
|---|---:|---:|---:|---:|---:|---:|
| 0 | 667 | 666 | 1.00x | 10617 | 10093 | 1.05x |
| 1 | 833 | 833 | 1.00x | 18095 | 19702 | 0.92x |
| 10 | 4292 | 4417 | 0.97x | 83925 | 106151 | 0.79x |
| 50 | 19750 | 21542 | 0.92x | 381498 | 495396 | 0.77x |
| 100 | 37375 | 42166 | 0.89x | 753158 | 981854 | 0.77x |
| 250 | 92750 | 105166 | 0.88x | 1881915 | 2457324 | 0.77x |
| 500 | 187250 | 210542 | 0.89x | 3765450 | 4917593 | 0.77x |
| chain 5 pages × 100 | 303708 | 321875 | 0.94x | 4965993 | 6115772 | 0.81x |

- The ns floor (≤1.0x) holds in every cell. The instr floor holds everywhere except the 0-tool cell.
- The 0-tool cell skips `write`, so `resize` cannot reach it. Its 1.05x instr (524 instructions) is the fixed setup
  described below: the plan header allocation (alloc_count 2 vs 1) and the root/result walk.
- The packet core target (≤0.95x ns) is met from 50 tools up and in the chain. 10 tools is 0.97x and 1 tool is 1.00x:
  at those sizes the buffer is mostly the 5010-word prefix, and both sides now copy it with one memcpy.

### Before P05G (P05M numbers)

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
- The packet core target (≤0.95x) was met from 100 tools up. The 1.0x floor failed in these cells:
  - ns at 1 tool, 10 tools and the 5-page chain
  - instr at 0 tools and 1 tool

### Why the >1.0x cells were slower before P05G, line by line

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

**Fix (done in P05G).** Option 1 of the original proposal was taken, with no pattern fusion. SPEC.md now has the array
builtin `resize(a, n, v)`:

- result length `n`; elements below `len(a)` are copied from `a`, and the tail is `v`;
- it requires `len(a) <= n`, carries `! alloc`, and costs `1 + n`;
- the contracts give z3 the length, the prefix values and the tail values.

It is in both compilers (py, rs) and both backends. The Rust backend emits `with_capacity + extend_from_slice +
resize`. The LLVM backend calls the runtime helpers, which copy the prefix and fill only the tail.

Other bench code with the same pattern (listed only, not changed in P05G; `bench/l3/budget` is excluded):

| file:line | pattern | fit |
|---|---|---|
| `bench/perf/brainfuck/brainfuck.pruv:15` | `alloc(len(tape)*2)` + prefix copy loop | direct |
| `bench/l3/claude/a1_root.pruv:33` | `lead + s` into a fresh buffer | partial (only the first part is a prefix) |
| `bench/l3/mcp/root.pruv:23`, `bench/l3/codex/util.pruv:20` | `join`: one loop with an a/b branch | partial (needs the loop split first) |

Full same-length copies and str→[u8] conversions do not fit, for example claude/a0_root:109, json/a4_root:40,
qwen/root:47, qwen/q_root:47, supervisor/a1_root:39, the codex/grok/opencode util copies and mcp/root `to_bytes`.

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
| `test_impl.py` (py/rust, py/llvm, rs/rust, rs/llvm) | 22/22 each (also after P05G) |
| Q differential, 2040 cases × 4 pairs | identical to the pre-rewrite baseline: 2027 exact, 3 exact-decimal-id, 3 invalid-utf8, 3 byte-line-cap, 3 numeric-signal, 1 surrogate-safe-cut |
| Q differential after P05G | identical to the same baseline in all 4 pairs |
| `q_mutants.py` | 12/12 killed (also after P05G). page-contract and tool-contract are rejected by the verifier; the other 10 are differential-killed. |
| jstr consumers (codex, grok, opencode) | pruvc verify ok |

q_mutants anchors moved with the code:

| mutant | new anchor |
|---|---|
| drop-title | `"title"` → `"absentTitle"` (first occurrence: the `field(d,cur,"title")` call; before LB5b it was in the `fields4` call) |
| description-cap | `{ return 65536; }` in `desc_cap` |
| readonly | `p[base+6]=1;` |
| destructive | `== 2 { p[base+7]=1; }` → `p[base+7]=0` (LB5b) |

## How to run

```sh
python3 bench/l3/mcp/perf/run_perf.py check   # oracle check, all cases
python3 bench/l3/mcp/perf/run_perf.py perf    # ns + instr table (NEW / HAND / OLD)
python3 bench/l3/mcp/perf/run_perf.py all
```

By default the harness uses the main checkout's pruvc. When testing a branch (for example a worktree with `resize`),
pass `--pruvc <worktree>/pruvc-rs/target/release/pruvc`, or the build fails with "unknown function resize".

The harness compiles perf_root.pruv with pruvc. It includes hand_packet.rs into perf_driver.rs and writes its build
output under `perf/work/`, which is not committed.
