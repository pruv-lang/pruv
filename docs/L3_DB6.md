# L3 DB6 qwen perf debt (D-R05-1..4)

Status: DB6: D-R05-1 closed. The bucket walk is gone: collide time went from 17.6x/35.0x to 1.18x/1.19x, the same as dense. What remains is D-R05-3 cost, and q_grid's collide keys no longer target the new hash (D-DB6-1, G-DB6-1).
Status: DB6: D-R05-2 partial. Linear RSS went from 2.81x/2.36x to 2.19x/1.63x. The bitmap is now one byte per Entry ordinal. heads and slots are sized to the entry count. Remaining: the turns buffer, wire and runner copies (D-DB6-2).
Status: DB6: D-R05-3 partial. There is one `type` lookup per line. Each line is still parsed twice and copied by the lib (D-DB6-3, G-DB6-2). Dense time is 1.21x/1.24x.
Status: DB6: D-R05-4 partial. The duplicate-group walk is cheaper with the new hash. The rest shares D-R05-3's cause. Dupbranch time is 1.19x/1.23x.
The main conversation folds this file into docs/L3_RESTRUCTURE_SPEC.md; this file does not edit the spec.

## 1. What landed (bench/l3/qwen only)

| Part | Change | Files |
|---|---|---|
| D-R05-1 hash | `bucket` mixes the key 8 bytes at a time with `load_le`, `wrapping_mul` and `bxor`/`ushr`, then a splitmix-style finalizer and a `band` mask (see note 1). The table is a power of two of at least 2x the entry capacity. `lookup` semantics are unchanged: newest-first, every duplicate marked, parent taken from the first match. | `chain.pruv` `bucket`, `select` |
| D-R05-2 sizes | New `capacity(data)` = newlines + 1, an upper bound on Entry frames. The marks/`selected` bitmap now holds one byte per Entry frame ordinal, where it used to be `len(data)`. heads and slots are sized from the capacity. The arena stays `len(data)`, but it is zero-allocated (lazy pages) and only uuid bytes are written. | `chain.pruv` `capacity`, `scan`, `select`; `import.pruv` `window_wire` |
| Ordinal marks | `scan` stores the Entry ordinal `ord` in slot 0 instead of the byte start. `window_wire` counts Entry frames the same way: Dropped frames do not count, and Skip and Object lines do. It reads `selected[ordinal]`. | `chain.pruv` `scan`; `import.pruv` `window_wire` |
| D-R05-3 type | `entry::string_id(d, 0, "type")` is read once and compared with `access::string_is` against 3 literals. It used to be 3 `field_is` calls, each looking up the key again. | `chain.pruv` `scan` |

Note 1: the mixing in `bucket` is proved bounded by `ensures 0 <= result && result <= mask`. It is not keyed (G-DB6-1).

No proof invariant replaces a run-time check: the mask bound replaces `% size`, and the index stays inside `heads`. So per the brief, no audit was run.

## 2. Tests (qwen only)

| Script | Result |
|---|---|
| `test_impl.py` | verifier PASS, 26/26 on py_rust, py_llvm, rs_rust, rs_llvm |
| `q.py` | 0 unexplained on all 4 pairs (same 19756, P1 64, P4 201, P5 1) |
| `q_mutants.py` | 12/12 killed: 10 differential; last-title and turn-cap rejected by the verifier |
| `q_grid.py --check` (part of the perf run) | 33/33 Pruv == hand: 21 fixtures and 12 grid files. Collide passed at the full rate, denominator 1, with no timeout. |

## 3. Perf (`python3 q_grid.py --runs 15`, rs_llvm vs hand Rust)

- Load: `uptime` showed averages of 19.04 / 21.13 / 32.19 just before the run. That is over 16, so 15 runs were used.
- Medians of 15 alternating fresh-process runs. Time and RSS are defined as in docs/L3_R05.md §3.
- Cells: `bench/l3/qwen/work/r05/perf_cells.json`.
- The before columns are copied from docs/L3_R05.md §3 (7 runs, load not recorded). Compare the ratios, not the absolute ms: both sides ran interleaved under the same load.

| Grid | Size | Before time | After Pruv ms | After hand ms | After time | Before RSS | After Pruv RSS | After hand RSS | After RSS |
|---|---|---|---|---|---|---|---|---|---|
| linear | M | 0.86x | 6.06 | 7.26 | 0.84x | 2.81x | 14.6 MB | 6.7 MB | 2.19x |
| linear | L | 0.92x | 11.64 | 14.61 | 0.80x | 2.36x | 18.8 MB | 11.5 MB | 1.63x |
| dense | M | 1.15x | 32.14 | 26.60 | 1.21x | 0.63x | 15.2 MB | 29.0 MB | 0.52x |
| dense | L | 1.33x | 64.68 | 52.21 | 1.24x | 0.54x | 21.2 MB | 56.5 MB | 0.37x |
| dupbranch | M | 1.11x | 32.11 | 27.02 | 1.19x | 0.63x | 14.9 MB | 29.0 MB | 0.51x |
| dupbranch | L | 1.11x | 64.83 | 52.69 | 1.23x | 0.51x | 20.9 MB | 56.5 MB | 0.37x |
| collide | M | 17.6x | 30.52 | 25.78 | 1.18x | 0.64x | 15.4 MB | 29.0 MB | 0.53x |
| collide | L | 35.0x | 62.69 | 52.53 | 1.19x | 0.52x | 21.6 MB | 56.5 MB | 0.38x |

8 of 16 cells are still above 1.0x: the 6 dense/dupbranch/collide time cells and the 2 linear RSS cells. Collide is no longer an outlier.

## 4. Debt rows

| ID | Cells | Cause | Fix direction |
|---|---|---|---|
| D-DB6-1 | collide (grid validity) | `q_grid.py` `bucket()`/`collision_keys` still build keys for the old `h*257 % size` hash. Against the new mixing hash they are ordinary keys, so the collide cells now measure the dense path rather than the worst case. Finding colliding keys for the new hash needs a brute-force search over its low bits, which this pass did not do. | Port the new `bucket` to `q_grid.py` and search printable suffixes for each of the 8 targets, or replace the cell with a fixed adversarial set once a keyed hash exists (G-DB6-1). |
| D-DB6-2 | linear RSS M 2.19x, L 1.63x | Not measured line by line, so this is the expected cause, unconfirmed. Selected text lives three times: in the `turns` buffer (`pkgs/turns/turns.pruv:61` grows to a 13116800-byte zeroed buffer, touched as text is written), in the result wire (`import.pruv` `hl + total`), and in the runner's `unwire` + `record()` copies. Hand Rust owns only one copy of the turn strings. | Write the turn text straight into the wire, or hand the turns buffer off as the wire (needs a `turns` pkg change, out of this lane). Measure each buffer before choosing. |
| D-DB6-3 | dense time M 1.21x, L 1.24x; dupbranch M 1.19x, L 1.23x; collide M 1.18x, L 1.19x | Every line is still framed and parsed twice: `chain.pruv` `scan` and then the title/turn pass in `import.pruv` `window_wire` (`window_next` + `parse_line`). `lines::parse_range` copies each line into an owned buffer (G-DB6-2), and `string_field` copies `uuid`/`parentUuid` into a fresh `str` before `put` writes them to the arena (G-R05-5). | Parse once: keep the parsed fields the second pass needs (title rank, role, turn content spans) per ordinal, or let the chain decision be lazy. Both need a borrowed-span parse from the lib (G-DB6-2, G-R05-5). |

## 5. Gap rows

| ID | Gap | Workaround here |
|---|---|---|
| G-DB6-1 | No seed or randomness source for a keyed hash, and the LLVM backend has no String-keyed HashMap. The uuid index cannot match hand Rust's keyed SipHash against an adversary who knows the hash. | A fixed mixing hash (`chain.pruv` `bucket`) that defeats the old structured grid but is not keyed |
| G-DB6-2 | `lines::parse_range` (bench/l3/lib) copies each line into an owned buffer before parsing, with no parse over a borrowed `data` span. | None: lib is out of this lane |
