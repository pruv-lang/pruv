# R10 hand Rust baselines + measure script (worker brief)

Repo: Pruv. Work only in `bench/l3/supervisor/hand/` (new files) and a new script `bench/l3/supervisor/measure_r10.py`.
Do not edit any other file. Do not run full test suites. Only kill pids you started; no `pkill -f`.

## Context
`bench/l3/supervisor/` holds a Pruv command supervisor. `test_a3.py` has `HARNESS` (a Rust main that reads fixture
lines, runs a control TCP socket thread, calls `sol::supervise(shell, argv, cwd, out, file_limit, tail_chars, ctl)`
and prints `packet | started | summary(out file: len + FNV-1a 64)`). `common.py` `configure()`/`build()` compile a
Pruv root on pairs py/rs x rust/llvm and build a harness with `@SOL@` replaced by the generated `.rs` path.
`measure.py` is the old measurement (read it; reuse `line()`/`timed()` ideas). Packet layout and semantics: read
`supervise.pruv`, `capfile.pruv`, `packet.pruv`, `args.pruv` and `docs/COMMAND_SUPERVISOR_SPEC.md`.

The tail is being changed (by the main session, file `bench/l3/supervisor/btail.pruv`, not yours; since step 4.2 the
Pruv side uses `bench/l3/pkgs/tail` instead, see `../RESHAPE.md`) to this byte
algorithm; your hand Rust must implement exactly the same algorithm in natural safe Rust:
- Two WHATWG UTF-8 decoders (stream 1 = stdout, stream 2 = stderr), maximal-subpart U+FFFD replacement, exactly as
  `bench/l3/pkgs/tail/tail.pruv` `dec_byte` (needed/seen/cp/lower/upper; on a bad continuation emit U+FFFD and
  re-process the byte as a lead). Flush at end: pending stream 1 then stream 2 each emit one U+FFFD.
- Each decoded code point is appended as UTF-8 bytes to one shared `Vec<u8>` buffer (capacity 6T+4, T = tail_chars),
  keeping `units` = UTF-16 length (cp >= 0x10000 counts 2, else 1).
- After an append, if `units > 2T || bytes > 6T`: drop whole code points from the front (length from the lead byte:
  <0x80 1, <0xE0 2, <0xF0 3, else 4; units 2 for a 4-byte lead else 1) while `units > T || remaining bytes > 3T`,
  then move the remainder to the front (`copy_within`).
- Output (`tail_bytes`): the same front-drop with `units > T || bytes > 3T` on the final buffer, return the rest.
  This equals: the longest suffix of whole code points of the decoded text with at most T UTF-16 units.

## Deliverables
1. `hand/core_hand.rs`: `pub fn core(t: i64, data: &[u8], chunk: i64) -> Vec<u8>` — split `data` into consecutive
   `chunk`-byte pieces, piece k goes to stream 1 if k even else 2, push each piece (as a sub-slice, no copy), flush,
   return tail bytes. Natural safe Rust (indexing, Vec), no unsafe, no get_unchecked.
2. `hand/supervise_hand.rs`: `pub fn supervise(shell: &str, argv: &str, cwd: &str, out: &str, file_limit: i64,
   tail_chars: i64, ctl: i64) -> Vec<i64>` with the same packet as the Pruv `supervise` for the normal paths used in
   measurement (spawn shell with NUL-separated argv, cwd; read stdout/stderr in 64 KiB reads; capture file prefix up
   to file_limit; tail as above; control socket connect to 127.0.0.1:ctl, a byte/EOF on it means stop). Natural
   std Rust (threads or poll), safe code. It must give packets identical to the Pruv supervisor on the measurement
   cases (exit 0, wait mode) — the script checks this.
3. `hand/core_main.rs`: harness template for the core lane, `#[path="@SOL@"] mod sol;` calling `sol::core`. Args:
   `T CHUNK SIZE`. Build input deterministically (repeat the 64-byte UTF-8 pattern
   `"ab\u{e9}\u{d55c}\u{1f600}\n"` plus bytes `\x80 \xe0\x80 \xf0\x9f` mixed in, same on both sides, built before
   timing), time only the `core` call with `Instant`, print `ns len fnv64hex` of the result.
   The Pruv side is `bench/l3/supervisor/core_root.pruv` (main session writes it) with
   `pub fn core(t: i64, data: [u8], chunk: i64) -> [u8] ! alloc`; check the generated signature (`&[u8]` or Vec) and
   adapt the harness call so the same template builds against both (a tiny shim is fine).
4. `measure_r10.py`: builds (a) Pruv core lane for pairs rs/rust and rs/llvm (py pairs optional) from
   `core_root.pruv` with `core_main.rs`, (b) hand core with `core_main.rs` + `core_hand.rs`, (c) Pruv whole-process
   runner from `supervise.pruv` with `test_a3.HARNESS` (flags `common.manifest_flags`), (d) hand whole-process runner
   with `test_a3.HARNESS` + `supervise_hand.rs`. Use `rustc -O` for hand (same flags as `common.build`).
   Child for the whole-process lane: a small Rust generator built by the script (`gen SIZE CHUNK` writes SIZE bytes
   of the same pattern to stdout in CHUNK-byte `write` calls), used identically by both sides.
   Grid: sizes M = 16 MiB, L = 64 MiB; core lane chunks 4 and 65536, T 4096 and 65536; whole lane chunk 65536,
   T 4096, file_limit 1<<30. Alternate runs Pruv/hand, >= 7 rounds each cell, median. Time: core = printed ns;
   whole = process wall time. RSS: max RSS of the runner process (`/usr/bin/time -l`, "maximum resident set size",
   bytes on macOS). Check outputs equal (core: len+fnv; whole: packet and file summary) and fail loudly if not.
   Write `work/r10/perf_cells.json` (under `bench/l3/supervisor/work/`) in the format used by `tools/perf_gate.py`
   (read that file at the repo root for the cell format; one cell per lane x size x chunk x T x pair with time and
   RSS ratio pruv/hand) and print a table. Option `--quick` runs 1 round on M only for a smoke test.

## Done when
- `python3 measure_r10.py --quick` runs end to end once the main session's `core_root.pruv`/`btail.pruv` exist; if
  they do not exist in your copy, make the hand parts compile standalone (`rustc -O` a test main) and verify
  `core_hand.rs` against a tiny Python model of the algorithm above on random inputs (including T=1 and 4-byte chars).
- Return: file list, how you verified, any interface assumption the main session must match.
