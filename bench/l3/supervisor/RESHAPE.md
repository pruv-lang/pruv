# Command supervisor: Rust-shape reset (RUST_SHAPE_RESET §6 step 4.2)

Hand baseline: `hand/supervise_hand.rs` (`supervise`, `write`, `read_pipe`, `stopped`) and `hand/core_hand.rs`
(`Tail`, `Decoder`, `core`). Pruv product path: `supervise.pruv` + `capfile.pruv`, with the shared packages
`pkgs/tail` (hand `Tail`, reshaped in step 2.10) and `pkgs/args` (`arg_spans`, `build_args`, step 2.8) through
`supervise.pruv.json`. Behaviour: `docs/COMMAND_SUPERVISOR_SPEC.md` (P1-P8), unchanged by this step.

Hand extension in this step: the file half of the receive loop's chunk arm is now `fn write(file: &mut Option<File>,
packet: &mut [i64], file_limit, bytes: &[u8])` (same statements, moved out of the loop) so it has a per-function
counterpart. Nothing else in the hand changed.

## 1. What changed

| before (3.8) | now | hand |
| --- | --- | --- |
| private `btail.pruv` (R10 byte tail: `Dec`/`Acc`/`Tail` structs, `buf: [u8]` beside the struct, streams 1/2) | `pkgs/tail` (`new`, `push(inout tl, 0\|1, bytes)`, `finish(take tl)`); `btail.pruv` deleted | `Tail::new`, `push`, `finish` |
| private `args.pruv` (old span packet + `substr` per argument) | `pkgs/args` (`arg_spans`, `build_args` with `find_byte` + `subview`); `args.pruv` deleted | `split_terminator('\0')` |
| `cw = [written, truncated, error, limit, total]` beside the result; `report` built `header()` (alloc 11) then `packet()` (alloc 12+n, two copy loops) | the result packet `p = alloc(12, 0)` is the state from the start: p[6] truncated, p[7] file error, p[8] file open, p[9] total, p[10] written; the end does `p[11] = len(bytes)` and one widening `arr_append`; `packet.pruv` deleted | `let mut packet = vec![0; 12]`, same slots, `packet.extend(bytes.map(i64::from))` |
| `cap_take` wrote a prefix through `truncate(copy(data), n)` (a copy of the chunk) | `capfile::write`: `n = min(limit - p[10], len)`, `p[6] = 1` when cut, one write of the view `bytes[..n]` | `write_all(&bytes[..n])` |
| tail state created before the spawn | `tail::new(tail_chars)` after the started byte, as the hand (after `set_nonblocking`) | same |
| validation: `arg_spans` before the range tests | range tests first, then `arg_spans` (the size + trailing-NUL test, gap U8), so a bad number allocates nothing, as the hand's `\|\|` | ranges, `len > 1048576`, `ends_with('\0')` |
| `core_root` / `a1_root` on `btail` with streams 1/2 | on `pkgs/tail`; `core_root` uses `index % 2` like the hand `core`; `a1_root` maps the test's stream 1/2 to 0/1 | `core`: `chunks(chunk).enumerate()`, `index % 2` |
| both pipes at EOF → `ev_proc_wait` (blocking; a stop request after the pipes closed was not seen) | `ended`: poll the socket with `ev_wait(0)` and `try_wait` (`ev_proc_wait_for(.., 2)`), as the hand's `try_wait` + `sleep(2 ms)` on a disconnected channel (Codex review, BUG) | `Err(Disconnected)` arm |
| exit fields in locals `exit`/`known`/`stopped`/`reaped`, copied to `p[2..5]` at the end | written into `p` where decided (`p[4]` at the socket event, `p[2]`/`p[3]`/`p[5]` in `try_wait`, `p[5]` after `ev_remove`); `failed` also clears `p[2]`, `p[3]`, `p[5]` | `packet[2..5]` written in the loop |
| 4 KiB control-socket buffer | 1 byte, as the hand's `read(&mut [0])` | `stopped` |
| `core_root::core` returned an empty tail for `t <= 0`, `chunk <= 0` or `t > 65536` | `requires 1 <= t <= 65536 && chunk >= 1` (the hand's `assert!`; the upper bound is the `pkgs/tail` contract) | `assert!(t > 0 && chunk > 0)` |
| `a2_root` drove `cw` | drives the same packet slots (`p[9] += n` is the loop arm's count, then `capfile::write`) | — (test root) |

## 1.5 Structure table (§1.5 form)

| item | hand | Pruv now | difference |
| --- | --- | --- | --- |
| result / state | `Vec<i64>` of 12, slots written in place, then extended | `[i64]` of 12 (`alloc(12,0)`), slots written in place, then `arr_append` (widening) | = (the `[i64]` packet is the host boundary on both sides) |
| file handle | `Option<File>`; `None` after a failed write | `File` threaded as `take f -> File`; "Some" is `file_writable(f)` (a failed write leaves it not writable) | gap FO |
| chunk write | `write_all(&bytes[..n])`, no copy | `write_all(move f, subview(bytes, 0, n))` through a one-line Pruv wrapper, no copy | gap FV (wrapper) |
| file close | `drop(file)` (errors ignored) | `capfile::close`: `file_close`, failed close sets p[7] = 2 when no error yet | E2 (spec P4) |
| tail | `Tail` struct, `push(stream, &bytes)`, `finish(self)` | `pkgs/tail::Tail`, `push(inout tl, stream, bytes)`, `finish(take tl)` | = (package rows CW, BV, U8, P7-1 stay with `pkgs/tail`) |
| arguments | `command.args(argv.split_terminator('\0'))` | `build_args(argv)`: `find_byte`, `proc_args_push(move a, subview(..))` | = (pkgs/args) |
| pipe reading | two `scope.spawn` reader threads, `[0; 65536]` stack buffer each, `buf[..n].to_vec()` per chunk, `sync_channel(2)`, `recv_timeout(2 ms)` | one `Loop` with the process and the socket; `ev_wait(-1)`, then `ev_proc_read_into` into two reused 64 KiB buffers and a view per chunk (no per-chunk allocation) | gap TH |
| stop / disconnect | `stopped()` polls a nonblocking 1-byte read each turn, `child.kill()`, keeps receiving until both pipes close, `try_wait` | socket readiness event, one 1-byte `ev_read_into`; then a drain loop until a pump step reads nothing, `ev_remove` (kill + reap); after both pipes close the socket is still polled (`ended`) | gap TH (same results; ordering is the event model's) |
| spawn | `Command` + `Stdio::null()` stdin | `proc_spawn` (piped stdin) + `proc_close_stdin` | host API (no stdin mode on `proc_spawn`), E2 path to kind 3 |
| fallible setup | thread spawn cannot fail softly | `ev_new`, `ev_add_process`, `ev_add_socket` can fail: each ends in `failed` (kind 3) | gap TH |
| allocations per run | packet 12, `Tail` bytes `6t+4`, 2×64 KiB stack, one `Vec` per chunk, packet growth once | packet 12, `Tail` bytes, 2×64 KiB + 1 byte heap buffers (once), packet growth once | Pruv fewer (no per-chunk `to_vec`) |

## 2. Per-function line table (kinds as in `mcp/RESHAPE.md`: `=`, E1 proof-only check, E2 spec/host-required, P annotations only)

| hand | Pruv | difference | kind |
| --- | --- | --- | --- |
| `supervise`: `packet = vec![0; 12]` | `p = alloc(12, 0)` first | none | = |
| range tests on `tail_chars`, `file_limit`, `ctl` | same tests, same order | none | = |
| bad input → `packet[0] = 1; packet[1] = 5` | same, after the range tests | none | = |
| `argv.len() > 1048576 \|\| !ends_with('\0')` | after the range tests: `arg_spans(argv)[0] != 0` → the same `p[0] = 1; p[1] = 5` (same two tests, plus the per-NUL char-boundary test that cannot fail on a `str`) | the call computes spans the supervisor does not read; the bad-input store is written twice (one `if` per test) | U8 (pkgs gap) |
| `TcpStream::connect` fail → `packet[1] = 4` | `net_connect`; fail → `close_sock`, `p[1] = 4` | must-consume close of the failed handle | E2 (owned handle rule) |
| `File::create(out).ok()`, `packet[7] = is_none`, `packet[8] = is_some` | `file_create`, `if file_writable(f) { p[8] = 1 } else { p[7] = 1 }` | none | = |
| `Command::new(shell)`, `args`, `current_dir`, `spawn` | `build_args`, `proc_spawn(shell, a, cwd)` | cwd empty is handled by the host | = |
| spawn error → `packet[1] = 2`, `packet[2] = 1/3/2` | `proc_error` → `p[1] = 2`, `p[2] = code` (1..6, else 2), `proc_wait` + `close_sock` + `capfile::close` | the failed Process and the socket are must-consume; P4 close; codes are the spec's | E2 |
| `packet[1] = 1` | same, right after the spawn | none | = |
| (none: `Stdio::null()`) | `proc_close_stdin`; fail → kill, `failed` | host API | E2 |
| `write_all(&[1])` + `set_nonblocking` fail → `packet[4] = 2`, kill, `packet[5] = wait ok` | `net_write_bytes(started)` fail → `p[4] = 2`, `p[5] = kill_proc(..)` (kill + reap result), `close_sock`, `capfile::close` | P4 close | E2 |
| `Tail::new(tail_chars)` | `tail::new(tail_chars)` | none | = |
| `thread::scope`, two `read_pipe` threads, `sync_channel(2)` | `ev_new(2)`, `ev_add_process`, `ev_add_socket`, each fail → `failed` | event model | TH |
| `loop`: `stopped()` each turn, `recv_timeout(2 ms)` | `while tag == 0 && !broken && events < 10000000`: `ev_wait(-1)` (`ev_wait(0)` once the pipes ended), dispatch on event kind/slot | event model; the `events` cap and its `decreases` (a `while` needs a measure; reaching it ends as kind 3) | TH, UL |
| `Ok(Ok((stream, bytes)))` arm | `take(..)` per readable stream after `ev_proc_read_into` | arm body is `take` | see `take` |
| `Ok(Err(_))` → `packet[1] = 3`, kill, break | `ev_ok(l)` false → `broken` → `failed` | event model | TH |
| `Disconnected` → `try_wait`; `Some(status)` → `packet[5] = 1`, exit code or `-signal`, `packet[3] = 1`, break; `None` → `sleep(2 ms)` | `proc_state == 2` → `ended`; `try_wait`: `ev_proc_wait_for(l, pslot, 2)`, slot freed → `ev_exit` in i32 range → `p[2]`, `p[3] = 1`; `p[5] = 1` → tag 1; else poll again | exit range test; `try_wait` is split out of the loop body (the inline form took the verifier past 10 minutes) | E1, P, TH |
| (stop path continues receiving until the pipes close) | drain loop (`more`, `n < 10000000`), `ev_remove`; ok → `p[5] = 1`; `ev_close` `Err(8)` → `p[5] = 0` | event model; `n` cap | TH, UL |
| `drop(file)` | `capfile::close(move f, inout p)` | P4 | E2 |
| `packet[1] == 3` → `wait`, zero [4], [9], [10] | `failed`: close (P4), `p[1] = 3`, zero [2], [3], [4], [5], [9], [10] | P4 close; [2], [3], [5] because a failed `ev_close` comes after the reap recorded them | E2, TH |
| `tail.finish()`, `packet[11] = len`, `packet.extend(bytes.map(i64::from))` | `tail::finish(move tl)`, `p[11] = len(bytes)`, `arr_append(inout p, bytes, 0, len(bytes))` | none | = |
| `write`: `if let Some(output) = file` | `capfile::write`: `if !file_writable(f) { return move f; }` | `Option<File>` vs threaded handle | FO |
| `n = (file_limit - packet[10]).min(len)` | `n = file_limit - p[10]; if len < n { n = len }` | no `min` builtin; same value | = |
| `packet[6] \|= (n < len)` | `if n < len { p[6] = 1 }` | none | = |
| `if n != 0 { write_all(&bytes[..n]) ok → packet[10] += n else packet[7] = 2, file = None }` | `if n == 0 return`; `write_all(move f, subview(bytes, 0, n))`; writable → `p[10] += n` else `p[7] = 2` (the handle stays, not writable) | view through the wrapper | FV, FO |
| chunk arm `packet[9] += len` | `take`: `p[9] += len(bytes)` after the skip test `len > 1048576 \|\| p[9] > 2^62 - 1048576` | skip test keeps the counter provably in range | E1 |
| `tail.push(stream, &bytes)` | `tail::push(inout tl, stream, bytes)` (streams 0/1) | none | = |
| `read_pipe` (thread body, `buf[..n].to_vec()`) | none (`ev_proc_read_into` fills the reused buffer) | event model | TH |
| `stopped` (1-byte nonblocking read, 0/1/2) | socket event: `sock_state`, 1-byte `ev_read_into(bs)`; `ns > 0` → stop (`p[4] = 1`), EOF/hangup → disconnect (`p[4] = 2`) | event model | TH |
| (none) | `close_sock`, `close_loop`, `kill_proc`, `sock_state`, `proc_state` | must-consume handles and slot observers of the event API | TH, P |
| `core`: `assert!(t > 0 && chunk > 0)`, `Tail::new`, `chunks(chunk).enumerate()`, `push(index % 2, piece)`, `finish` | `core_root::core`: `requires 1 <= t <= 65536 && chunk >= 1`, `tail::new`, `while lo < n` with `hi`, `push(inout tl, index % 2, subview(data, lo, hi))`, `finish` | the assert is the precondition; `t <= 65536` is the package contract; cursor loop instead of `chunks()` | P, E1 |

Counts over the 32 rows (a row can carry two kinds): `=` 11, E1 3 (`take` skip test, exit range, `core` package bound), E2 7 (connect close, spawn codes, stdin close, started-byte path, `drop(file)`, `failed`, P4 on `close`), P 3 (`try_wait` split, `core` requires, slot observers), gap rows TH 9, UL 2, FO 2, FV 1, U8 1; one row points to `take`. No unexplained row.

## 3. Language and host gaps (rows added to `lib/RESHAPE.md`)

| id | gap | where | minimal change (Rust can do it) |
| --- | --- | --- | --- |
| TH | No threads/channels: a child's pipes are read through the host event `Loop` (readiness events, nonblocking reads into caller buffers, slot observers, fallible `ev_new`/`ev_add_*`), where the hand runs two reader threads and `recv_timeout` | `supervise` loop, drain loop, `sock_state`, `proc_state`, `close_loop`, the `failed` early returns | A scoped-thread + bounded-channel API (`thread::scope`, `sync_channel`, `recv_timeout`) over owned `Process` pipe halves; or accept the event model as the Pruv host shape (same as pkgs X1/X2 **W-c**) |
| FO | `File` cannot sit in an `Option` or be passed `inout` (SPEC §6h placement), so the hand's `&mut Option<File>` becomes a handle threaded through `take f -> File` and "Some" becomes `file_writable` | `capfile::write`, `take`, every `f = take(move f, ..)` | Allow `inout f: File` (Rust `&mut File`) and `Option<File>` with must-consume on both arms |
| FV | A builtin's array argument cannot be a `subview` (`type/view_context` is limited to Pruv calls), so `file_write_bytes(f, &bytes[..n])` needs a one-line Pruv wrapper (`capfile::write_all`) to avoid a copy | `capfile::write` | Extend R-6g-6 to builtin read-only `[u8]` parameters (`file_write_bytes`, `net_write_bytes`): Rust passes `&b[lo..hi]` |
| UL | No unbounded loop: `while` needs a `decreases` measure and `loop { }` is `parse/loop_unsupported`, so the hand's `loop` (runs until the child ends or the controller stops it) has an `events < 10000000` cap, and the drain loop an `n < 10000000` cap; reaching the event cap ends as kind 3 | `supervise` main loop and drain loop; `take`'s `2^62` skip test exists to keep `p[9]` in range under these caps | An unbounded `loop`/`while` without `decreases` in a function marked as possibly non-terminating (an effect such as `! diverge`), with partial-correctness proofs of its invariants |
| U8 | (existing, pkgs) `build_args` needs the per-NUL char-boundary fact that the verifier does not know of a `str`, so the validation calls `arg_spans` | `supervise` validation | as in `pkgs/RESHAPE.md` U8 |

P4 (close errors), the must-consume closes and `proc_close_stdin` are not gaps: they are the spec's policies or the
host API's ownership rule (E2).

## 4. Results

Tests (rs/rust and rs/llvm, after the last code change, which applies the Codex structure review). The 12 mutant
originals in `mutants.py` each match the code once; the mutant run itself was not repeated in this step.

| check | rs/rust | rs/llvm |
| --- | --- | --- |
| `test_a0.py` | 522/522 | 522/522 |
| `test_a1.py` | 410/410 | 410/410 |
| `test_a2.py` | 78/78 | 78/78 |
| `test_a3.py` | 35/35 | 35/35 |
| `q.py` (29 cases from A3 + faults vs the TS reference; the script has no count argument) | same 22, allowed 7, DIFF 0 | same 22, allowed 7, DIFF 0 |
| `mcp/test_impl.py` (shared packages unchanged; 4.1) | 22/22 | 22/22 |
| `mcp/q.py`-style run, 300 cases (340 requests), rs pairs only | 0 unexplained | 0 unexplained |

Performance was not remeasured in this step (§6 step 4 asks for shape first).
