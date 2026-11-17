# L3-2 req `list` three-way split: protocol vs constant factor

Question: why does the Pruv req fragment take 20.09 s on `list` (S=20000, M=50, no match) where the Node original takes
1.17 s? The candidates were (1) the step protocol, (2) no cross-call state (out of scope here, handled elsewhere) and
(3) the code generator's constant factor.

**Answer: it is the step protocol. The constant factor is not a cause.**
- The same search written as one pure Pruv call takes 0.67 ms at S=20000. That is 0.64x the hand-written Rust and
  0.31x the same flat-array loop in JS.
- The full Pruv run takes 8.19 s, so the search itself is under 0.01% of the time.
- The other 99.99% is the ~20 µs that every host round trip costs. The fragment makes two round trips per session per
  item (`sessionList.next`, `isTemporary`), which is 420 k round trips at S=20000, M=10.

Machine: Apple Silicon, macOS 26.5, node v26, rustc -O. All times are best of 3 (A–C) or best of 5 after a warm-up
(D). M=10 throughout, because S=20000 with M=50 (the original cell) and S=50000 with M=10 both reach the 1 M host-call
bound.

## Legs

| leg | what runs | timed |
|---|---|---|
| A | Node original: `reference.mjs` oracle on the `perf_req.py` scenario | whole process |
| B | the Pruv req fragment as committed (`bench/l3/req/*.pruv`), rebuilt with `-g` | whole process |
| C | B with `req_world.dig` → `1` and `world_ok` → `true` (no digest, no seal or world check) | whole process |
| D | the bound search alone (each item walks every session's 8 candidates), one in-process call | the call only |

D variants:
- `int`: threads interned to ids.
- `str`: threads as UTF-16 spans in a pool, compared unit by unit.
- `obj` (JS only): the real handler shape. It uses `perf_req.py` session objects, the pinned `sessionThreads`
  generator, `excludedFromBound`, and string `===`.

## Results

Process times in seconds. The ratio in parentheses is against A. Empty-scenario process time is A 0.081 s, B and C
0.004 s.

| S | A Node | B Pruv | C Pruv, no digest/check | B−C | host calls | B per call | C per call | A per call |
|---|---|---|---|---|---|---|---|---|
| 2000 | 0.132 | 0.831 (6.3x) | 0.503 (3.8x) | 0.328 | 42 k | 19.8 µs | 12.0 µs | 1.2 µs |
| 20000 | 0.443 | 8.189 (18.5x) | 4.993 (11.3x) | 3.196 | 420 k | 19.5 µs | 11.9 µs | 0.86 µs |
| 50000 | 0.847 | 21.05 (24.9x)\* | 13.46 (15.9x)\* | 7.60 | 1.05 M | 20.0 µs | 12.8 µs | 0.73 µs |

\* At S=50000, B and C stop at the 1 M call bound with `InputTooLarge`, which is the same behaviour as the original
M=50 cell. A finishes `ok`, so `same_value` is false only in this row. Both are timed to the same point (about 1.05 M
host calls). At S=2000 and S=20000, all three legs return the same request value.

Leg D, one call, milliseconds. The checksums of all implementations agree.

| S | Pruv int | hand Rust int | JS int | Pruv str | hand Rust str | JS str | JS obj (real shape) |
|---|---|---|---|---|---|---|---|
| 2000 | 0.066 | 0.104 | 0.214 | 0.069 | 0.103 | 0.232 | 9.75 |
| 20000 | 0.669 | 1.045 | 2.14 | 0.713 | 1.046 | 2.28 | 101.2 |
| 50000 | 1.687 | 2.66 | 5.54 | 1.779 | 2.62 | 5.86 | 243.8 |

Constant factor:
- Pruv / hand Rust is 0.63–0.68x. The Pruv kernel is faster because its indexed `while` loops beat the hand version's
  `chunks_exact` and iterator code.
- Pruv / JS on the same flat loop is about 0.31x.
- Against the real JS object and generator shape, Pruv is 140x faster.

There is no codegen constant factor above 2x to explain.

## Breakdown at S=20000, M=10 (B = 8.19 s)

| share | seconds | source |
|---|---|---|
| core search | 0.0007 | D Pruv int |
| process start | 0.004 | empty scenario |
| integrity: per-step digest, seal and world check | 3.20 (39%) | B − C |
| rest of the step protocol: packet build, world re-encode, bundle, host decode/serve/trace | 4.99 (61%) | C |

So the protocol share is about 100% and the constant-factor share is about 0%. Nothing else remains.

The per-call cost stays flat at about 20 µs from S=2000 to S=50000. The cost is therefore O(host calls × fixed packet
size), not O(calls × S): the world packet does not grow with the session count. It is simply a large constant per step
against about 0.8 µs per call in Node. The original 20.09 s cell is the same thing: about 1 M calls × 20 µs.

### B profile (macOS `sample`, 1 ms, S=20000, 4480 samples; `profile_b.sh`)

Inclusive:
- Pruv `req_step`: 2582 (58%).
- Host `accept` / `Leaf::serve_call`, which serves the call and builds the trace JSON: 1532 (34%).
- Host `rejected` check: 165 (4%).

Self time, top items:

| frame | samples | share | what |
|---|---|---|---|
| `req_world::dig` | 837 | 19% | the two-modulus polynomial hash, run over the whole packet every step |
| `emit_raw` + `emit_await` (sol.rs 184/218) | 819 | 19% | inlined `out(act_set(w2, …, dig(pkt)), pkt)`, which re-seals the world and bundles the packet |
| `memmove`, `memset` | 489 | 11% | whole-array copies (`w1.to_vec()`, `apply`, `act_set`, `seal`, `bundle`) |
| `malloc` / `free` family | ~1200 | ~27% | one fresh `Vec` per world or packet stage per step |
| `mach_absolute_time` | 192 | 4% | host per-step clock reads |
| host JSON (`BTreeMap` clone, `serde_json` drop, UTF-16 encode, `enc_session`) | ~180 | 4% | trace and session encoding |

## Reading

- Fix the round-trip count, not the code generator. The 20x gap is fully explained by 420 k round trips at about 20 µs
  each.
  - Removing integrity work alone (C) still leaves 11x.
  - Even a 10x cheaper step leaves the fragment slower than Node, unless the walk over sessions stops costing one or two
    host round trips per session per item.
  - The search itself is negligible when it runs inside one Pruv call.
- Within a step, about half the time is the digest and seal. The other half is allocation and copying of whole world
  and packet arrays. Making steps incremental (digest only what changed, reuse buffers) would cut the per-step cost.
  The protocol shape would stay the same.

## Reproduce

```sh
cd bench/l3/perf
sh build_bc.sh bc                  # legs B and C -> work/b/harness, work/c/harness (needs bench/l3/req/dev_build.sh host rlib)
python3 run_ab.py --reps 3         # legs A, B, C at S=2000,20000,50000, M=10 -> work/ab.json (~4 min)
sh d/build_d.sh                    # leg D: verifies d/kernel.pruv with pruvc-rs, links d/hand.rs -> work/d/main
python3 d/run_d.py --reps 5        # leg D table -> work/d/d.json
sh profile_b.sh 20000 10 6         # B profile -> work/sample_b_20000_10.txt, prints top self-time frames
```

Files:
- `run_ab.py`, `build_bc.sh`, `patch_c.py`: legs A to C. The C patch touches only a copy under `work/c/src`.
- `d/`: `kernel.pruv` (Pruv), `hand.rs`, `ts_kernel.mjs`, `gen.py` (data), `main.rs` (driver), `run_d.py`,
  `build_d.sh`.
- `profile_b.sh`: profile of leg B.
