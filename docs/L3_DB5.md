# L3 DB5: perf debt bundle part 5 (opencode D-R07-1..4, codex D-R09-1..2)

Target: every listed cell at or below hand Rust 1.0x on time and RSS.

## Status

- Status: DB5: D-R07-1 partial: begin time 0.31x/0.43x rs/rust, 0.40x rs/llvm M, 1.24x rs/llvm L (keys_unique O(E); D-DB5-2).
- Status: DB5: D-R07-2 partial: step_checked 0.80x/0.92x rs/rust, 1.02x/1.07x rs/llvm (was 1316-1633x); O(E) rows on both sides (D-DB5-1).
- Status: DB5: D-R07-3 partial: step_admitted 0.95x/0.95x rs/rust, 1.11x/1.07x rs/llvm (was 356-760x); hand lane now does the same checks.
- Status: DB5: D-R07-4 closed: step RSS 0.991x rs/rust, 1.000x rs/llvm on every step cell (was 1.01-1.06x).
- Status: DB5: D-R09-1 partial: time 1.23x/1.11x rs/rust, 1.28x M / 0.93x L rs/llvm (was 1.67-1.88x); D-DB5-4.
- Status: DB5: D-R09-2 partial: RSS 7.0x-9.6x (was 10.9-12.9x); the rest is pkgs/turns::grow (D-DB5-3, G-DB5-3).

## Measurement

Old values (L3_RESTRUCTURE_SPEC 7.5):
- opencode: begin 25-218x, step_checked 1316-1633x, step_admitted 356-760x, step RSS 1.01-1.06x.
- codex: time 1.67-1.88x, RSS 10.9-12.9x.

Final run (one run, `measure_r07.py --rounds 15` then `measure_r09.py` at 7 rounds, Pruv/hand). Load averages: 127.36 before r07, 96.47 between, 84.63 after r09. Load stayed far above 16, so the cells near 1.0x on rs/llvm are within the noise of this host.

| Cell | rs/rust M | rs/rust L | rs/llvm M | rs/llvm L |
|---|---|---|---|---|
| opencode begin time | 0.307x | 0.426x | 0.397x | 1.241x |
| opencode begin RSS | 0.972x | 0.959x | 1.000x | 0.984x |
| opencode step_checked time | 0.799x | 0.917x | 1.017x | 1.071x |
| opencode step_checked RSS | 0.991x | 0.991x | 1.000x | 1.000x |
| opencode step_admitted time | 0.949x | 0.951x | 1.105x | 1.065x |
| opencode step_admitted RSS | 0.991x | 0.991x | 1.000x | 1.000x |
| codex read time | 1.231x | 1.106x | 1.283x | 0.928x |
| codex read RSS | 7.082x | 9.586x | 7.011x | 7.214x |

The quick runs below are kept for reference.

Quick runs during the work (one round each, load 40-150, not final):

| Cell | Time | RSS |
|---|---|---|
| opencode begin rs/rust | 0.262x | 0.97x |
| opencode begin rs/llvm | 1.033x | 1.01x |
| opencode step_admitted | 1.08x | 0.945x |
| codex rs/rust M / L | 1.62x / 2.05x | 7.1x / 9.4x |
| codex rs/llvm M / L | 2.33x / 1.87x | 7.0x / 8.3x |



## What landed

- `bench/l3/opencode/root.pruv`:
  - D-R07-1: `keys_unique` replaces the O(E^2) `while j < i` / `key_same` pass. It is one loop per row: refs, a value NUL scan, and a single key scan that checks NUL and '=' and computes an FNV-1a hash (`bxor`/`wrapping_mul`). The row number goes into an open-addressing table of a power-of-two size above 2E, with slot `band(hash, size-1)` and linear probing. A hit compares the bytes (`key_same`), so a hash collision never rejects. `env_ok(uniq=true)` is now this pass. `env_ok`/`cfg_ok` are `! alloc`.
  - D-R07-2: the checked `opencode_step` still runs `cfg_ok(cfg, t, false)`, which is the O(1) `cfg_head_ok` plus the refs-only `env_ok(uniq=false)` row loop (no `str_clean`, no key pass). A first draft checked the header only. The review found that this let a row damaged after begin (for example, a value ref of (-1,0)) through, so it was reverted. The hand checked step now runs the same row loop (see D-DB5-1).
  - D-R07-3: `step_body` no longer builds the blank `ctl_init(cfg[1], 999)` on every call. The failure branches build it inline.
- `bench/l3/opencode/packet.pruv` (D-R07-3/4): `done_simple` builds the 116-word reject packet with one `alloc` and direct writes. For a valid (stage, cls, code), the words are the same as the old `done_wrap(ctl, simple_error(...))` path. The runtime `step_args_ok` re-check is gone for these, because the words are fixed by construction. An out-of-range enum returns `invariant_step` as before. A class/code pair that breaks the `result_shape` rule takes the old `done_wrap` path unchanged; the review found the first draft skipped this.
- `bench/l3/opencode/hand/begin_hand.rs`, `hand/hand_main.rs` (D-R07-3 scope note, perf rule 4): the hand step lane now runs the same checks as `step_body` (cfg head, ctl consistency `ctl_ok`, event shape) and returns the same 116-word reject packet. The runner folds the whole packet each step, and `measure_r07.py` stops if the Pruv and hand step digests differ.
- `bench/l3/opencode/measure_r07.py`: a `--rounds` option (default 7), and the digest equality check above.
- `bench/l3/codex/` (D-R09-1/2, delegated write by Codex gpt-6.2-sol; checked and applied here). In `codex_read_wire`, the thread-read `line` buffer and the held-turn arena `hb` no longer start as `alloc(8388608)` and `alloc(8700000)`. They are `util::ByteBuffer { data: [u8] }`, starting at 4 KiB, and `util::grow_bytes` grows them by doubling up to the old caps (8388608 and 8700000). The wire limits are unchanged.
  - `read::absorb` grows `hb` before each copy, and its contracts change from `len == cap` to `len <= cap`.
  - `rpc.pruv` gets a local `receive`/`step` over the growable line. This is the same poll and event order as `transport::receive`, which requires a full-size line (G-DB5-2).
  - `codex_list` still sizes its line at 8388608 through the same type.
  - `read::replay` treats a stamp offset of -1 as "none" before its range check, so a short arena no longer flags the record as bad.

## Debt rows

| Id | Cell | Line-level cause (hand vs generated) |
|---|---|---|
| D-DB5-1 | checked step lane scope | Both sides still walk the E env rows on every checked step (root.pruv `opencode_step` → `cfg_ok(uniq=false)`, hand/begin_hand.rs `step`). The cost is O(E) per step rather than O(1). An O(1) checked entry would need a begin admission token in the packet, which changes the packet contract. |
| D-DB5-2 | begin lane scope | Hand `begin` returns a `bool` (hand/begin_hand.rs `begin`). Pruv `opencode_begin` also builds the 76-word first step packet. The hand side does not build it, so the begin cells slightly favour hand. |
| D-DB5-3 | codex RSS, M and L (D-R09-2) | The replay collector `pkgs/turns::grow` (turns.pruv:61) jumps straight to `alloc(13116800)` once the ring wraps. That is 200 slots x 65584 bytes, zeroed. The hand reader keeps a `Vec` of the turns it holds. It is outside the editable set (bench/l3/pkgs), so it is recorded and not changed. |
| D-DB5-4 | codex time, M and L (D-R09-1) | The remaining cost is the same 13 MB zero fill (D-DB5-3) plus the JSON tree walk in `read.pruv`. The hand reader streams it with serde. |

## Gap rows

| Id | Gap | Workaround |
|---|---|---|
| G-DB5-1 | `HashMap` keys are only i64, bool or String (R-6e-5). There is no hash set over a `[u8]` slice, so a byte-key uniqueness check cannot use the standard map. | A hand-written open-addressing table of row numbers in a `[i64]` (root.pruv `keys_unique`). |
| G-DB5-2 | `pkgs/transport::receive` requires `len(line) == 8388608`, so a growable line cannot be passed to it. | A local copy of its poll and event order over a `ByteBuffer` in codex `rpc.pruv`. A package `receive` over a growable buffer would remove the copy. |
| G-DB5-3 | `pkgs/turns::grow` has no growth policy below the full ring size. Its comment rejects doubling because of the copy cost on a full ring. | None in this lane (D-DB5-3). It needs a package change, for example sizing to `begun` slots, or storing the folded turns in a byte list. |

## Tests

- codex `test_impl.py`: 136/136 on py/rust, py/llvm, rs/rust and rs/llvm (rc 0). `q.py`: rc 0, 0 unexplained. `q_mutants.py`: every mutant listed so far was killed.
- opencode `test_impl.py`: reference ts/expected 199/199. Each pair passes 194/199. The 5 failures are all F08 (hang, term-exit, kill-required, close-overrides-timeout, give-up-script). These are timing fixtures, and unchanged HEAD a3a4196 also fails them at this load (rs/rust F08 FAILED 4).
- opencode `q.py`: 2199 cases, 16 unexplained = 4 cases (q01107, q01451, q01581, q01730, all `read`) x 4 pairs. In each, both the TS reference and the impl report "no exporter log exists". The reference side is missing too, so this is load and not this change.
- opencode `q_mutants.py`: every mutant listed so far (M01, M02, ...) was killed. The run was still going at commit time.
- Audit: Codex gpt-6.2-astra ultra found 2 mandatory issues (the done_simple class/code pair, and the narrowed checked step). Both are fixed, and the re-check confirmed both fixed.
