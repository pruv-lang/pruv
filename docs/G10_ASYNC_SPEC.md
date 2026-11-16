# G10: the `async` effect and the owned `Loop` handle

Status: **A0–A3 implemented, 2026-11-16. Windows gate 8 passed: `tests/g10_async/windows.py` 7/7 on the Windows host (MSVC target), both compilers. Q done (RESULTS.md §127).**
Batch status will be recorded per batch below and in `RESULTS.md` with Q.

This document defines the multiplexing slice that `docs/G10_NET_SPEC.md` §9 and `docs/G10_PROC_SPEC.md` §9
deferred. A Pruv program can put sockets, listeners, child processes and one-shot timers into one `Loop`, then
wait for the next event on any of them, read what is ready without blocking, write, and cancel a slot. That is
one loop that accepts clients, reads their requests, streams a child's output and enforces timeouts.

This is readiness multiplexing with timers and cancellation. It is **not** `async fn` / `await` syntax, and
it adds no futures and no scheduler to the language.

Python `pruvc` gets each feature first and `pruvc-rs` ports it in the same batch. The Rust source backend
uses `std` threads, `std::net`, `std::process` and one `Mutex` + `Condvar`; no crates. The LLVM backend reaches
the same Rust runtime through the G6 Rust shim, exactly as `File`, `Process`, `Socket` and `Listener` do. There
is no C event runtime in this slice.

The rules and gates in §3–§8 are acceptance conditions, not measurements. `docs/G10_NET_SPEC.md` is the model
for this document (shape, ownership rules, error table, verification encoding, shim); `docs/G10_PROC_SPEC.md`
is the model for process slots. This document repeats only what differs. Section references written `net §x`
and `proc §x` point there.

## 1. Goals

- One `Loop` owns a fixed number of numbered slots, each holding one socket, one listener, one process or one
  timer.
- Wait for the next event on any slot, with an optional timeout. Exactly one event per wait.
- Read a socket or a process's output without blocking, once the loop says it is ready.
- Write to a socket or a process's stdin; half-close a socket; close a process's stdin.
- Accept a pending connection into a new slot without blocking.
- Cancel any slot: close it, kill and reap it, or drop the timer, and free the slot.
- A `Loop` can move, but on every normal control-flow path its last owner must consume it with `ev_close`.
  A handle moved into a loop is consumed; the must-consume duty moves to the loop.
- A failing slot does not fail the loop. Failures are values, as in net and proc.
- A busy slot cannot starve the others (round-robin scan).
- Both compilers and both backends agree on meaning, diagnostics and generated output.
- Local tests use only 127.0.0.1 and local fixture children and never depend on an outside network.

Non-goals of this slice: `async fn` / `await`, taking a handle back out of a loop, non-blocking writes with
output queues, UDP, TLS, sharing a loop across `par`, an OS poller (epoll, kqueue, IOCP), connect timeouts. See
§9. The must-consume rule becomes a rule over {`Process`, `Socket`, `Listener`, `Loop`} in the same analyzer.
The `File` auto-close policy does not change.

Why this slice exists. Handles cannot be stored in arrays, structs or collections (`type/foreign_owned_type`),
and every net and proc operation blocks. A Pruv program therefore cannot serve several connections, or watch a
child's output while also serving sockets. The Lair backend needs exactly that.

Repository facts this design relies on (HEAD 34d2e51):

- The effect list is `io`, `alloc`, `fs`, `proc`, `net` (`pruvc/vparser.py`, `C/parser.rs`). The `proc` and
  `net` slices are implemented through Q.
- Must-consume exists for `Process`, `Socket` and `Listener` in one analyzer: the Python `_Ownership` in
  `pruvc/foreign.py` and the Rust `Ownership` in `C/foreign.rs`. `type/proc_unconsumed` and
  `type/net_unconsumed` are reported from there and, for discarded temporaries, from `pruvc/check.py` and
  `C/check.rs`.
- The catalog is declared with `_add(...)` in `pruvc/foreign.py` (for example `net_read`), with the observer
  lists `PROC_OBSERVERS` and `NET_OBSERVERS` next to it. Catalog contracts are encoded by
  `FnVerifier.net_call` in `pruvc/verify.py` and by its Rust twin in `P/symbolic.rs`.
- An owned catalog type cannot sit inside `Result`, so a `Loop` is never a `Result` payload either.
- The `par` forbidden-effect collection lists `io`, `fs`, `proc` and `net` explicitly (`pruvc/check.py`
  `par_info`, `C/check.rs`), and the violation is `type/parallel_effect`. SPEC.md §6i states that the whole
  branch call graph may use only `alloc`.
- LLVM keeps `File`, `Process`, `Socket` and `Listener` as opaque pointers; consuming adapters unbox, updates
  return a new box; scope cleanup calls the drop adapter on non-null owned slots. Native `--emit exe|obj`
  refuses shim programs with `codegen/foreign_target`.

Path abbreviations: `C/` = `pruvc-rs/crates/pruv-core/src/`, `P/` = `pruvc-rs/crates/pruv-proof/src/`,
`D/` = `pruvc-rs/crates/pruvc/src/`.

## 2. User surface

### 2.1 The `async` effect

`async` is the sixth effect name (`io`, `alloc`, `fs`, `proc`, `net`, `async`).

```pruv
fn pump(take l: Loop) -> Loop ! async, net, alloc
```

`async` does not include `io`, `fs`, `proc` or `net`. It is the effect of waiting on a loop and of owning one.
Operations that touch a socket also need `net`, operations that touch a process also need `proc`, and every
operation that allocates (all effect operations, because the public buffer allocates) needs `alloc`. The
per-operation effect sets are in §2.3. `async` is **forbidden inside `par`** (§3.4).

Two operations take a union effect set because the kind of the slot is a run-time value: `ev_remove` needs
`async, net, proc, alloc`, and so does `ev_close`. A program that only uses timers still declares `net, proc`
when it removes a slot or closes the loop. This is deliberate and conservative: the effect set never depends on
a value.

### 2.2 Types

**`Loop`** — owned catalog type, internal key `loop`, generated Rust name `pruv_Loop`. A fixed-capacity table
of slots `0 .. cap-1`, the worker threads that serve them, and the public state described below.

It follows the placement limits of `Socket`:

- Allowed as a local, a `take` parameter, a read-only borrowed parameter, and the result of an internal
  function.
- No copies, no aliases, no storage inside owned containers (struct, enum, array, Option, Result,
  collections). So a `Loop` cannot hold other loops, and a handle cannot be stored next to it.
- Cannot be a parameter or result of a host entry.
- Moves use `move l`.
- Must-consume. Auto-drop cannot discharge it. A `Loop` is ended only by `ev_close`.

A user declaration named `Loop` keeps priority, as with `File`, `Process`, `Socket` and `Listener`.

**Slots.** A slot holds nothing, or exactly one resource of one kind (`ev_kind`): `0` free, `1` socket,
`2` listener, `3` process, `4` timer. Slot ids are plain `i64` values. Slots are reused: an add always takes the
lowest free slot, so an id can name a different resource after a remove. A program that keeps ids must track
their lifetime itself; the contracts in §2.3 say exactly which slots change.

**Ownership transfer.** `ev_add_socket`, `ev_add_listener` and `ev_add_process` take the handle with `take`.
The handle is consumed by the call, statically (the usual `type/foreign_move` rules apply to it afterwards) and
at run time. The duty to end it moves to the loop: `ev_remove`, `ev_proc_wait` or `ev_close` ends it. **There is
no way to take a handle back out in this slice.** The handle's own public state (a `Socket`'s buffer, EOF and
error) is not carried over; the slot starts with an empty published state.

**Public state.** A `Loop` has one public read buffer (`ev_size`, `ev_byte`, `ev_bytes`), the last wait's event
(`ev_event_slot`, `ev_event_kind`), the id produced by the last add or accept (`ev_last`), and the exit
status of the last waited process (`ev_exit`). Like net and proc, **observers never query the OS or the worker
threads**: the same Pruv value always gives the same observer results, whatever peers and children do in the
meantime. State changes only at the next explicit effect operation.

### 2.3 Catalog operations

Notation (descriptive only, not syntax). `L` = `Loop`. `M` = the existing array length bound `2^40`.
`cap(l)` = `ev_cap(l)`, `cnt(l)` = `ev_count(l)`, `last(l)` = `ev_last(l)`, `kind(l,i)` = `ev_kind(l,i)`,
`eof(l,i)` = `ev_eof(l,i)`, `open(l,i)` = `ev_write_open(l,i)`, `ok(l)` = `ev_ok(l)`, `size(l)` = `ev_size(l)`.
`slot ok` means `0 <= slot && slot < cap(l)`. Parameter observers in contracts mean the value **at call
entry**, so `kind(l,i)` inside an ensures of an operation that takes `l` is the entry value; the result's
values are written `kind(result,i)`.

Shorthand used in the table:

- **P** = `ok(l)`. Every operation except `ev_new` and `ev_close` requires it.
- **B** (base) = `cap(result) == cap(l)`, the event (`ev_event_slot`, `ev_event_kind`) preserved, `ev_exit`
  preserved, and the public buffer preserved (equal size and equal bytes).
- **F(S)** (frame) = for every slot `i` not in the set `S`, `kind(result,i) == kind(l,i)`,
  `eof(result,i) == eof(l,i)` and `open(result,i) == open(l,i)`. Each operation below names exactly which slots
  it may change.
- **N** = `last(result) == -1`.
- **O** = `ok(result)`. Only the add operations that start a thread (`ev_add_socket`, `ev_add_listener`,
  `ev_add_process`) and `ev_accept` can produce `!ok(result)`; every other operation preserves `ok`.
- **new slot s** (for an add or an accept that produced a slot) = `0 <= s && s < cap(result)`, `kind(l,s) == 0`
  (it was free at entry), `s == last(result)`, `cnt(result) == cnt(l) + 1`, and F({s}).

A failed add or accept (`!ok(result)`) is the internal failure of §2.4: `1 <= ev_error(result) && ev_error(result)
<= 2`, N, `cnt(result) == cnt(l)`, `kind(result,i) == kind(l,i)` for every slot, and B. From then on only
`ev_close` can meet its requires.

#### Loop lifetime

| Operation | Effect | Contract / meaning |
|---|---|---|
| `ev_new(cap: i64) -> Loop` | `async, alloc` | requires `1 <= cap && cap <= 4096`. Ensures `ok(result)`, `cap(result) == cap`, `cnt(result) == 0`, every slot free (kind 0, EOF false, write-open false), N, no event (`ev_event_slot == -1`, `ev_event_kind == 0`), empty buffer, `ev_exit == 0`. No thread is started yet |
| `ev_close(take l: Loop) -> Result<i64>` | `async, net, proc, alloc` | no requires. Ends every remaining slot in ascending slot order (socket close, listener close, process kill and wait, timer drop), joins all worker threads, releases the loop. Result below |

`ev_close` result. With `e = ev_error(l)` at entry:

1. If `e != 0`, the result is `Err(c)` with `1 <= c && c <= 8`. The loop failed (or a slot failed earlier); the
   resources are still ended.
2. If `e == 0`, the result is `Ok(cnt(l))` (the number of slots ended), or `Err(c)` with `3 <= c && c <= 8`.
   `Err` here means some slot failed earlier (codes 3–7, see §2.4) or ending a process failed (code 8).
3. The error is the first published failure code of §2.4 (the first published error is kept). A slot that was
   marked failed and then removed still counts: the error is a fact about the loop's history, and a program that
   does not care discards the `Result` as `net_close` callers do.

`Ok(0)` therefore means "the loop had no slots and no recorded failure", unlike `net_close` where the payload
is always 0. OS errors from the final close of a socket or listener are not observable through `std` and are
not reported (as in net).

#### Adding resources

| Operation | Effect | Contract / meaning |
|---|---|---|
| `ev_add_socket(take l: Loop, take s: Socket) -> Loop` | `async, net, alloc` | requires P, `net_ok(s)`, `cnt(l) < cap(l)`. Consumes `s` and starts its reader thread. Success: O, new slot `last(result)` (the lowest free slot) with `kind(result,last) == 1`, `open(result,last) == net_write_open(s)`, `!eof(result,last)`, B |
| `ev_add_listener(take l: Loop, take x: Listener) -> Loop` | `async, net, alloc` | requires P, `net_listener_ok(x)`, `cnt(l) < cap(l)`. Consumes `x` and starts its accept thread. Success: new slot with `kind(result,last) == 2`, `!open(result,last)`, `!eof(result,last)`, B |
| `ev_add_process(take l: Loop, take p: Process) -> Loop` | `async, proc, alloc` | requires P, `proc_ok(p)`, `cnt(l) < cap(l)`. Consumes `p`; its output collectors now signal the loop. Success: new slot with `kind(result,last) == 3`, `open(result,last) == proc_stdin_open(p)`, `!eof(result,last)`, B |
| `ev_add_timer(take l: Loop, ms: i64) -> Loop` | `async, alloc` | requires P, `cnt(l) < cap(l)`, `0 <= ms && ms <= 86400000`. A one-shot timer that fires `ms` milliseconds after the call (monotonic clock). Cannot fail: O. New slot with `kind(result,last) == 4`, `!open`, `!eof`, B |

An add that fails (thread spawn failure, §2.4) still consumes the handle: the runtime ends the resource (socket
closed, listener closed, process killed and reaped, best effort) so nothing leaks, and the loop becomes failed.
For a `Socket` the read timeout the program set with `net_set_timeout` is cleared (a timeout on the shared
descriptor would break the reader thread); the **write timeout is kept** and bounds `ev_write`, see §2.5.

The new slot is the lowest free slot. That is runtime meaning, checked by runtime gates; the contract only
promises "a slot that was free".

#### Waiting and events

| Operation | Effect | Contract / meaning |
|---|---|---|
| `ev_wait(take l: Loop, timeout_ms: i64) -> Loop` | `async, alloc` | requires P, `-1 <= timeout_ms && timeout_ms <= 86400000`. Blocks until one slot has an event or the timeout passes (`0` = do not block, `-1` = no timeout). Reports exactly one event, see below |

`ev_wait` ensures O (it cannot fail), N, `cap` preserved, buffer preserved, `ev_exit` preserved, and with
`s = ev_event_slot(result)`, `k = ev_event_kind(result)`:

1. `-1 <= s && s < cap(l)` and `0 <= k && k <= 5`; `s == -1` exactly when `k == 0`.
2. `timeout_ms == -1 ==> k != 0` (a wait without timeout returns an event).
3. An event slot was non-free before the call, and its entry kind fits the event: `k == 1 ==> kind(l,s) == 1 ||
   kind(l,s) == 3`; `k == 2 ==> kind(l,s) == 2`; `k == 3 ==> kind(l,s) == 1 || kind(l,s) == 3`;
   `k == 4 ==> kind(l,s) == 4`; `k == 5 ==> kind(l,s) >= 1 && kind(l,s) <= 3`.
4. **A fired timer slot becomes free.** `k == 4 ==> kind(result,s) == 0 && cnt(result) == cnt(l) - 1` and
   F({s}). **A reported slot error marks the slot:** `k == 5 ==> !open(result,s) && !eof(result,s)` and F({s})
   (the observer values are snapshots taken at operations, so an error a worker thread found becomes visible
   here and not earlier). For every other outcome the slot keeps its entry kind (`k != 0 && k != 4 ==>
   kind(result,s) == kind(l,s) && kind(result,s) != 0`) and `cnt(result) == cnt(l)`; for `k` in 0..3 also
   F(empty).

Event kinds (`ev_event_kind`):

| Kind | Name | Reported for | Condition |
|---|---|---|---|
| `0` | timeout | none (`ev_event_slot == -1`) | the timeout passed (or `0` and nothing is ready) |
| `1` | readable data | socket; process | socket: its queue is non-empty. Process: the stdout queue or the stderr queue is non-empty. Level-triggered: reported again until the data is read |
| `2` | ready to accept | listener | a connection is waiting in its queue. Level-triggered |
| `3` | end of stream | socket; process | socket: the peer closed and its queue is drained. Process: both outputs ended and both queues are drained. Reported once |
| `4` | timer fired | timer | the deadline passed. Reported once; the slot becomes free |
| `5` | slot error | socket; listener; process | an I/O error was recorded for the slot (§2.4). Reported once; then only `ev_remove` |

Rules:

1. **One event per call.** The scan visits slots in ascending order starting **after the slot reported last
   time** and wrapping around (round-robin). The first slot with an event is reported. A busy slot therefore
   cannot starve the others: while several slots stay ready, successive waits visit them in turn.
2. Data comes before end: a socket with queued bytes reports `1` until they are read, and only then `3` or
   `5`.
3. Kinds `3` and `5` are reported **once** per slot. An end of stream or an error that the program already
   published through a slot operation (`ev_read` returned `eof` true, or an operation marked the slot failed) is
   not reported again.
4. Events are level-triggered for `1` and `2`: a program that does not read a ready slot gets the same slot
   reported again. A program must read, accept or remove.
5. A timeout (`0`) leaves the round-robin position unchanged.

#### Socket slots

A socket slot's reader thread keeps receiving into the slot's queue, so data arrives while the program does
other work. The slot operations never block on the network, except `ev_write`.

| Operation | Effect | Contract / meaning |
|---|---|---|
| `ev_read(take l: Loop, slot: i64, max: i64) -> Loop` | `async, net, alloc` | requires P, slot ok, `kind(l,slot) == 1`, `0 < max && max <= M`. **Never blocks.** Takes up to `max` already-received bytes (at most 1 MiB per call, however large `max` is) into the public buffer. Ensures O, B except the buffer, N, F({slot}), `kind(result,slot) == 1`, `0 <= size(result) && size(result) <= max`, `eof(l,slot) ==> eof(result,slot)`, `eof(result,slot) ==> size(result) == 0`, `open(result,slot) ==> open(l,slot)` |
| `ev_write(take l: Loop, slot: i64, data: [u8]) -> Loop` | `async, net, alloc` | requires P, slot ok, `kind(l,slot) == 1`, `open(l,slot)`. Writes everything; **may block** up to the socket's write timeout (the known limit, §2.5). Ensures O, B (the buffer is preserved), N, F({slot}), `kind(result,slot) == 1`, `eof(result,slot) == eof(l,slot)`. On success write-open is kept; on failure the slot is marked failed (write-open false, §2.4) |
| `ev_shutdown_write(take l: Loop, slot: i64) -> Loop` | `async, net, alloc` | requires P, slot ok, `kind(l,slot) == 1`, `open(l,slot)`. Half-close: the peer sees EOF, reading continues. Ensures O, B, N, F({slot}), `kind(result,slot) == 1`, `!open(result,slot)`, `eof(result,slot) == eof(l,slot)` |

`ev_read` details:

1. A size of 0 with `eof(result,slot)` false means **nothing yet**; "no bytes yet" is never reported as EOF.
2. EOF becomes true only when the stream has ended and the queue is already drained, so the chunk with the last
   bytes has EOF false and the next, empty read gives EOF true (net `net_read` rule 6). EOF stays true.
3. Reading a failed (marked) slot is a defined no-op: empty buffer, EOF false, the slot stays marked.
4. If a read fails midway nothing partial is published; earlier `[u8]` copies are unaffected.

#### Listener slots

| Operation | Effect | Contract / meaning |
|---|---|---|
| `ev_accept(take l: Loop, slot: i64) -> Loop` | `async, net, alloc` | requires P, slot ok, `kind(l,slot) == 2`, `cnt(l) < cap(l)`. **Never blocks.** Takes one pending connection from the listener's queue and adds it as a new **socket** slot. Success: O, **new slot** `last(result)` with `kind(result,last) == 1`, `open(result,last)`, `!eof(result,last)`. No pending connection: an error-free no-op, `last(result) == -1` and F(empty). Either way `kind(result,slot) == 2`, B except `last`, and the listener slot is unchanged |

The accepted socket gets `TCP_NODELAY`, no read timeout and no write timeout (the program may not set one: the
socket is already in a loop, so a blocking `ev_write` waits without a bound). Programs that need a write bound
on accepted sockets use `net_accept` + `net_set_timeout` + `ev_add_socket` instead, with their own listener.
A failed accept marks the listener slot failed (code 6): `last(result) == -1`, the slot stays kind 2, and it
reports event 5 once.

#### Process slots

| Operation | Effect | Contract / meaning |
|---|---|---|
| `ev_proc_read(take l: Loop, slot: i64, stream: i64, max: i64) -> Loop` | `async, proc, alloc` | requires P, slot ok, `kind(l,slot) == 3`, `stream == 1 \|\| stream == 2` (1 stdout, 2 stderr), `0 < max && max <= M`. **Never blocks.** Takes up to `max` already-collected bytes of that stream into the public buffer. Ensures O, B except the buffer, N, F({slot}), `kind(result,slot) == 3`, `0 <= size(result) && size(result) <= max`, `eof(l,slot) ==> eof(result,slot)`, `eof(result,slot) ==> size(result) == 0`, `open(result,slot) ==> open(l,slot)` |
| `ev_proc_write(take l: Loop, slot: i64, data: [u8]) -> Loop` | `async, proc, alloc` | requires P, slot ok, `kind(l,slot) == 3`, `open(l,slot)` (stdin open). Writes everything to the child's stdin; may block (the known limit). Ensures O, B, N, F({slot}), `kind(result,slot) == 3`, `eof(result,slot) == eof(l,slot)` |
| `ev_proc_close_stdin(take l: Loop, slot: i64) -> Loop` | `async, proc, alloc` | requires P, slot ok, `kind(l,slot) == 3`, `open(l,slot)`. Ensures O, B, N, F({slot}), `kind(result,slot) == 3`, `!open(result,slot)`, `eof(result,slot) == eof(l,slot)` |
| `ev_proc_wait(take l: Loop, slot: i64) -> Loop` | `async, proc, alloc` | requires P, slot ok, `kind(l,slot) == 3`. Closes stdin, keeps draining and **discarding** the output, waits for the child to exit, joins its collectors, frees the slot. Ensures O, `cap` preserved, event preserved, buffer preserved, N, `kind(result,slot) == 0`, `!eof(result,slot)`, `!open(result,slot)`, `cnt(result) == cnt(l) - 1`, F({slot}). `ev_exit(result)` is the exit status (proc §2.4 rules 8–10); it is not tied to any number by the contract |

`ev_eof` for a process slot is true once **both** output streams have been seen at their end by explicit
`ev_proc_read` calls; the stream state behind it is hidden. The "ends" event `3` is reported when both outputs
have ended and both queues are empty, once. A program that wants EOF of one stream reads that stream until a
read returns size 0 after the end event.

`ev_proc_wait` on a failed (marked) process slot does not wait for the child to exit by itself: it kills the
child first, like `ev_remove`, then reaps it, so a failed slot can never block the wait forever. This is the one
operation on a failed slot that does more than a no-op; all other slot operations on a failed slot are no-ops
(`ev_proc_read`, `ev_read`, `ev_accept`: empty buffer, `last == -1`; `ev_shutdown_write`,
`ev_proc_close_stdin`: unchanged) and writes cannot meet their requires because write-open is false.
Reaping failure is code 8: the slot is freed anyway, `ev_exit(result) == 0`, and the failure is remembered for
`ev_close`.

#### Cancellation

| Operation | Effect | Contract / meaning |
|---|---|---|
| `ev_remove(take l: Loop, slot: i64) -> Loop` | `async, net, proc, alloc` | requires P, slot ok, `kind(l,slot) != 0`. Cancellation: ends the slot's resource (socket close; listener close; process kill and wait; timer drop), joins the slot's threads, frees the slot. Ensures O, B, N, `kind(result,slot) == 0`, `!eof(result,slot)`, `!open(result,slot)`, `cnt(result) == cnt(l) - 1`, F({slot}) |

A removed slot's queued data is discarded. Removing the slot named by the last event leaves the event values as
they are (B): they are a report of the last wait, not a live view. Removing a process slot that is still
running kills it; this differs from `ev_proc_wait`, which waits for it.

#### Public buffer and observers

| Operation | Effect | Contract / meaning |
|---|---|---|
| `ev_bytes(l: Loop) -> [u8]` | `alloc` | independent copy of the public buffer. Ensures `len(result) == size(l)` and every element equals the matching `ev_byte` |

Observers (no effect; allowed in code and contracts):

| Operation | Meaning |
|---|---|
| `ev_ok(l: Loop) -> bool` | `ev_error(l) == 0` |
| `ev_error(l: Loop) -> i64` | `0 <= result && result <= 2`; the loop-level failure code (§2.4) |
| `ev_cap(l: Loop) -> i64` | `1 <= result && result <= 4096` |
| `ev_count(l: Loop) -> i64` | `0 <= result && result <= ev_cap(l)` |
| `ev_last(l: Loop) -> i64` | `-1 <= result && result < ev_cap(l)`; the slot produced by the last add or accept, `-1` after every other operation or when an accept found nothing |
| `ev_kind(l: Loop, slot: i64) -> i64` | requires `0 <= slot && slot < ev_cap(l)`; ensures `0 <= result && result <= 4` (0 free, 1 socket, 2 listener, 3 process, 4 timer) |
| `ev_eof(l: Loop, slot: i64) -> bool` | requires slot ok; EOF seen by the last explicit read of that slot. Implies `ev_kind` is 1 or 3 |
| `ev_write_open(l: Loop, slot: i64) -> bool` | requires slot ok; writes are allowed (socket write side open, process stdin open). Implies `ev_kind` is 1 or 3 |
| `ev_event_slot(l: Loop) -> i64` | `-1 <= result && result < ev_cap(l)`; the slot of the last `ev_wait`, `-1` for a timeout or before any wait |
| `ev_event_kind(l: Loop) -> i64` | `0 <= result && result <= 5`; `0` exactly when `ev_event_slot(l) == -1` |
| `ev_size(l: Loop) -> i64` | `0 <= result && result <= M` |
| `ev_byte(l: Loop, i: i64) -> i64` | requires `0 <= i && i < ev_size(l)`; ensures `0 <= result && result <= 255` |
| `ev_exit(l: Loop) -> i64` | the exit status stored by the last `ev_proc_wait`; `0` for a new loop |

`ev_count` equals the number of non-free slots at run time. The verifier does **not** encode it as a sum (§4);
it is promised by the contracts of the adding and removing operations instead.

All slot operations require a slot in range **and** the right `ev_kind` (and `ev_write_open` for writes), so a
program that ignores which kind of resource a slot holds does not verify.

### 2.4 Failures and error codes

These codes live only in `ev_error` and in `Err` of `ev_close`. They are not compiler diagnostic codes.

Loop-level (`ev_error`, these fail the whole loop):

| Value | Meaning |
|---|---|
| `0` | no failure |
| `1` | bad input: an argument outside its requires reached the runtime (only possible with `--foreign-checks off`; the runtime's representation defense, not a normal path) |
| `2` | internal failure: a worker thread could not be spawned, a descriptor could not be duplicated, or the read timeout of an added socket could not be cleared |

Slot-level (never change `ev_ok`; they mark the slot, are published as event kind 5 once, and are remembered for
the `Err` of `ev_close`):

| Value | Meaning |
|---|---|
| `3` | socket read failed, including a connection reset |
| `4` | socket write or shutdown failed, including a broken pipe |
| `5` | socket write timed out (the write timeout set before adding) |
| `6` | accept failed (the listener slot) |
| `7` | process pipe I/O failed (output read, stdin write or close) |
| `8` | ending a process failed (kill request or reap); also used when `ev_close`/`ev_remove` cannot end a slot cleanly |

Rules (as in net §2.4 and proc §2.4):

1. A failing slot does **not** fail the loop. The slot is marked: write-open false, EOF false, nothing more is
   delivered from it. It is reported with event kind 5 once (by the next `ev_wait` that reaches it, or earlier
   if the error was found by a slot operation, in which case rule 3 of the event rules applies). `ev_ok(l)`
   stays true. From then on only `ev_remove` (and `ev_proc_wait` on a process slot) acts on it; the other slot
   operations are defined no-ops (§2.3).
2. Only an internal failure (code 2; code 1 under the defense) fails the loop. A failed loop is
   `!ev_ok(l)`: its resources are still owned and `ev_close` ends them; no other operation can meet its
   requires. A failed loop reports no event: `ev_event_slot == -1`, `ev_last == -1`, empty buffer.
3. The first published error is kept. Later cleanup errors do not overwrite it. A loop-level error is also
   remembered as the first error unless an earlier slot-level error was already recorded.
4. A write can fail after a partial write. `Err` or a marked slot does not mean "nothing was sent". No
   automatic retry.
5. If a read fails midway, that call's partial data is not published. Data that arrived before the failing
   read is delivered first; the error is reported after it (event rule 2).
6. A reset peer or broken pipe never aborts the process (no `SIGPIPE`): reads fail with 3, writes with 4. A
   panic in a worker thread is a failure of its slot (3 or 7), never success.
7. `Ok(n)` from `ev_close` means `n` slots were ended with no recorded failure. A process killed by
   `ev_remove` or `ev_close` is not a failure; its exit status is dropped.
8. OOM and contract-violation aborts are distinct from I/O errors and keep the existing policy.

### 2.5 Blocking and limits

`ev_wait` blocks the calling thread until an event or the timeout, as designed. The following also block and
are the known limits of this slice:

- **`ev_write` and `ev_proc_write` may block.** They write everything. A socket write blocks until the peer
  has drained enough, up to the socket's write timeout set with `net_set_timeout` **before** `ev_add_socket`
  (then it fails with code 5); with no timeout it blocks without bound. Writing to an accepted socket has no
  timeout (`ev_accept` cannot set one). A program that wants bounded writes keeps messages small, uses
  `net_set_timeout` before adding, or waits for the peer's reply before sending more. Non-blocking writes with
  output queues are later work.
- **`ev_proc_wait`, `ev_remove` of a process, and `ev_close` may block** while the child exits or is reaped,
  and while collector threads are joined. A grandchild that holds an output pipe can delay the join (proc §2.5).
- **A wait on a loop that can never produce an event blocks forever** when `timeout_ms == -1`, for example an
  empty loop or a loop whose only slots already reported their one-time events. Programs pass a timeout or keep
  a timer slot. This is documented, not solved by a requires.
- **Backpressure.** Each socket slot, and each output stream of a process slot, has a queue of at most 1 MiB. A
  full queue stops that slot's reader until the program has read some of it. A peer or child that outpaces the
  program therefore stalls on its own side instead of growing memory. The pending queue of a listener holds at
  most 64 connections; further ones wait in the OS backlog.
  Output a child wrote **before** `ev_add_process` is kept in full (the proc runtime's queues are unbounded,
  G10_PROC_SPEC §2.5) and moves into the slot as is, so that one queue may start above 1 MiB. Nothing is
  dropped; the collector stays stopped until the program has read the queue below the bound. Programs that
  spawn a noisy child should add it to the loop at once.
- **Threads.** The runtime uses one reader thread per socket slot, one accept thread per listener slot, and two
  collector threads per process slot. A loop with the maximum capacity can therefore run up to 8192 threads.
  An OS poller (§9) is the long-term fix; this slice keeps `std` only.
- **Timers** use the monotonic clock. A timer never fires before its deadline; it may be reported later,
  because the program only sees it at its next `ev_wait`. There is no ordering promise between timers and
  other events beyond the round-robin scan.
- **Chunk boundaries** are not preserved: one write may arrive as several reads and several writes as one
  (TCP and pipes are byte streams). Framing belongs to user code, as in net §2.5.
- Protocol deadlocks, a child that never exits, and peers that never write are not solved.

The reader, accept and collector threads are an implementation means. They do not add user-facing threads or
`par` sharing.

### 2.6 Examples

These compile, verify and run once A1–A3 land. Slot ids are kept in plain `i64` variables; the invariants say
which slot holds what.

**A two-client line server.** One loop holds a listener and at most two clients. It echoes every chunk back,
stops accepting after the second client, and ends when no slots are left or after 5 s without any event. It
returns the number of echoed bytes.

```pruv
pub fn echo_two(port: i64) -> i64 ! async, net, proc, alloc
  requires 0 <= port && port <= 65535
{
  let ls: Listener = net_listen("127.0.0.1", port);
  if !net_listener_ok(ls) {
    let r0: Result<i64> = net_listener_close(move ls);
    return -1;
  }

  let mut l: Loop = ev_new(3);
  l = ev_add_listener(move l, move ls);
  if !ev_ok(l) {
    let r1: Result<i64> = ev_close(move l);
    return -2;
  }
  let lslot: i64 = ev_last(l);
  let mut listening: i64 = 1;
  let mut accepted: i64 = 0;
  let mut echoed: i64 = 0;
  let mut rounds: i64 = 100000;

  while rounds > 0 && ev_ok(l) && ev_count(l) > 0
    invariant 0 <= rounds && rounds <= 100000
    invariant ev_cap(l) == 3
    invariant 0 <= listening && listening <= 1
    invariant accepted == 2 ==> listening == 0
    invariant forall i in 0..ev_cap(l): ev_kind(l, i) == 2 ==> listening == 1 && i == lslot
    invariant 0 <= accepted && accepted <= 2
    invariant 0 <= echoed && echoed <= 4096 * (100000 - rounds)
    invariant 0 <= lslot && lslot < ev_cap(l)
    invariant (listening == 1) ==> ev_kind(l, lslot) == 2
    decreases rounds
  {
    l = ev_wait(move l, 5000);
    let slot: i64 = ev_event_slot(l);
    let ek: i64 = ev_event_kind(l);
    if ek == 0 {
      rounds = 0;
    } else {
      if ek == 2 {
        // a client is connecting: take it into a new slot
        if ev_count(l) < ev_cap(l) {
          l = ev_accept(move l, slot);
          if ev_ok(l) && ev_last(l) != -1 {
            accepted = accepted + 1;
          }
        }
        // two clients are enough: stop listening
        if ev_ok(l) && accepted == 2 && listening == 1 {
          l = ev_remove(move l, lslot);
          listening = 0;
        }
      } else {
        if ek == 1 {
          // data from a client: echo it back
          if ev_kind(l, slot) == 1 {
            l = ev_read(move l, slot, 4096);
            let n: i64 = ev_size(l);
            if n > 0 && ev_write_open(l, slot) {
              let chunk: [u8] = ev_bytes(l);
              l = ev_write(move l, slot, chunk);
              echoed = echoed + n;
            }
          }
        } else {
          // end of stream (3) or a slot error (5): drop that slot
          if ek == 3 || ek == 5 {
            l = ev_remove(move l, slot);
            if slot == lslot { listening = 0; }
          }
        }
      }
      rounds = rounds - 1;
    }
  }

  let r: Result<i64> = ev_close(move l);
  return echoed;
}
```

Every path ends the loop with `ev_close`, which closes the remaining client sockets and the listener. `ev_wait`
keeps `ev_ok`, so the loop guard is only needed after `ev_accept`, which can fail the loop. Because the
operations name which slots change, the verifier knows the listener stays kind 2 while a client is echoed or
removed. A client slot is removed once its end of stream arrives; a client that never sends again and never
closes is bounded by the 5 s idle timeout and `rounds`, which are not wall-clock proofs. The echo may block in
`ev_write` for a peer that does not read (§2.5).

**A child-output relay with a timer.** The program connects to a client, runs a child, relays its stdout to the
client, drains stderr so the child never stalls, and kills the child when a deadline timer fires. The
outcome is returned: `1` the child finished with exit status 0, `4` it finished with a non-zero status, `2` the
deadline fired, `3` the client or the child failed, `0` the round budget ran out; negative values are setup
failures.

```pruv
pub fn relay(program: str, host: str, port: i64, deadline_ms: i64) -> i64 ! async, net, proc, alloc
  requires 1 <= port && port <= 65535
  requires 1 <= deadline_ms && deadline_ms <= 86400000
{
  let mut c: Socket = net_connect(host, port);
  if !net_ok(c) { let r0: Result<i64> = net_close(move c); return -1; }
  c = net_set_timeout(move c, 5000);
  if !net_ok(c) { let r1: Result<i64> = net_close(move c); return -1; }

  let args: ProcessArgs = proc_args_new();
  let p: Process = proc_spawn(program, args, "");
  if !proc_ok(p) {
    let r2: Result<i64> = proc_wait(move p);
    let r3: Result<i64> = net_close(move c);
    return -2;
  }

  let mut l: Loop = ev_new(3);
  l = ev_add_socket(move l, move c);
  if !ev_ok(l) {
    let r4: Result<i64> = proc_kill(move p);
    let r5: Result<i64> = ev_close(move l);
    return -3;
  }
  let sslot: i64 = ev_last(l);
  l = ev_add_process(move l, move p);
  if !ev_ok(l) {
    let r6: Result<i64> = ev_close(move l);
    return -3;
  }
  let pslot: i64 = ev_last(l);
  l = ev_add_timer(move l, deadline_ms);

  let mut outcome: i64 = 0;
  let mut rounds: i64 = 1000000;
  while outcome == 0 && rounds > 0 && ev_ok(l)
    invariant 0 <= rounds && rounds <= 1000000
    invariant 0 <= outcome && outcome <= 4
    invariant 0 <= sslot && sslot < ev_cap(l)
    invariant 0 <= pslot && pslot < ev_cap(l)
    invariant sslot != pslot
    invariant (outcome == 0) ==> ev_kind(l, sslot) == 1
    invariant (outcome == 0) ==> ev_kind(l, pslot) == 3
    decreases rounds
  {
    l = ev_wait(move l, -1);
    let slot: i64 = ev_event_slot(l);
    let ek: i64 = ev_event_kind(l);
    if ek == 4 {
      outcome = 2;                      // the deadline: leave the loop, ev_close kills the child
    } else {
      if slot == pslot {
        if ek == 1 {
          l = ev_proc_read(move l, pslot, 1, 4096);
          if ev_size(l) > 0 && ev_write_open(l, sslot) {
            let chunk: [u8] = ev_bytes(l);
            l = ev_write(move l, sslot, chunk);
          }
          l = ev_proc_read(move l, pslot, 2, 4096);   // drop stderr so the child cannot stall
        } else {
          if ek == 3 {
            l = ev_proc_wait(move l, pslot);
            if ev_exit(l) != 0 { outcome = 4; } else { outcome = 1; }
          } else {
            outcome = 3;
          }
        }
      } else {
        if ek == 1 {
          l = ev_read(move l, sslot, 4096);           // discard what the client sends
        } else {
          outcome = 3;                                // the client left or failed
        }
      }
    }
    rounds = rounds - 1;
  }

  let r: Result<i64> = ev_close(move l);
  match r {
    Ok(n) => { return outcome; }
    Err(e) => { return -5; }
  }
}
```

The socket's write timeout is set before the socket is added, as §2.5 requires. The process handle is consumed
on every path: waited or killed before the loop exists, moved into the loop afterwards. `ev_close` ends
whatever is left: it drops the timer, closes the socket and, if the deadline fired, kills and reaps the child.
`ev_proc_wait` is used only after the end-of-stream event, so it does not block long; killing is `ev_remove` or
`ev_close`. `ev_close` returning `Err` (a failure that happened earlier in the loop, such as a reset client)
becomes `-5` here, and a program that only wants the outcome can ignore it.

## 3. Static rules

### 3.1 Effect and names

1. Add `async` to both parsers and the AST effect description.
2. Catalog effect sets feed the existing `used_effects` check (§2.3 lists them).
3. The unknown-effect hint becomes `io, alloc, fs, proc, net, async`.
4. `Loop` normalizes to its internal key `loop` like `File` and `Process`. A user declaration with the same
   name keeps priority, as today.
5. Generated Rust uses `pruv_Loop`, never a user type name.
6. The catalog names are `ev_*`; they are declared with `_add(...)` in `pruvc/foreign.py` and the matching Rust
   table, next to `net_*`. `ASYNC_OPS` is the tuple of catalog names starting with `ev_`.

### 3.2 Reused ownership rules

The rules of net §3.2 apply to `Loop` unchanged:

- Use after move, double consume, owned alias: `type/foreign_move`. This includes a `Socket`, `Listener` or
  `Process` after `ev_add_*` consumed it.
- Consuming or returning a borrowed handle: `type/foreign_borrow`.
- A loop statement consumes an outer owner and does not restore it: `type/foreign_loop_move`.
- Storage inside struct, enum, array, Option, Result or a collection: `type/foreign_owned_type`.
- A handle in a host entry signature: `type/entry_signature` for a `pub fn`, `type/foreign_export` for
  `fn main`.
- A trusted Rust declaration that creates or consumes a `Loop` around the catalog: `type/foreign_signature`.

No new public host ABI. Internal functions may return `Loop`.

### 3.3 Must-consume for `Process`, `Socket`, `Listener` and `Loop`

The set is generalized once more, to {`Process`, `Socket`, `Listener`, `Loop`}, in the **same** analyzer:
Python `_Ownership` (`pruvc/foreign.py`) and Rust `Ownership` (`C/foreign.rs`), plus the temporary check in
`pruvc/check.py` and `C/check.rs`. Do not write a second analyzer. All ten rules of net §3.3 apply to `Loop`:

1. **Block end:** a `Loop` leaving scope while `live` or `maybe` is rejected.
2. **return:** consume the return expression first, then every owned handle leaving the function must be
   dead. `return move l` transfers the duty.
3. **Fallthrough:** live `take` parameters and locals are checked the same way.
4. **Overwrite:** after evaluating and consuming the RHS, a destination still holding a live/maybe handle is
   rejected. `l = ev_wait(move l, 100)` is allowed; overwriting a live `l` with a new `ev_new` is not.
5. **Temporaries:** an unnamed `Loop` result also carries the duty. `ev_close(ev_new(4))` is allowed (taken at
   once). `ev_ok(ev_new(4))` (borrowed then dropped) is rejected, and so is an update result that is never
   used.
6. **Branches:** consumed in one branch only gives maybe at the join, rejected at scope end and overwrite.
   Consumed in both branches is allowed.
7. **Loops:** the outer-owner restoration rule stays. A `Loop` created inside a loop body must be consumed or
   moved out before that iteration's scope ends.
8. **Helpers:** a `take` helper must terminate the handle, pass it to another take helper, or return it.
9. **Failed handles:** a failed loop is not exempt, so the checker never needs to know whether a thread
   spawned.
10. **Modes:** `--no-verify`, `--trust-callers` and `--foreign-checks off` cannot turn this rule off.

Moving a `Socket`, `Listener` or `Process` into a loop is a consume like any other: the moved binding is dead
(rule 2 and rule 4 hold for it), and its duty is now the loop's. The checker does not track which slot holds
which resource; that is the job of the contracts in §2.3.

The rule guarantees exactly-once termination or transfer on normal control flow. It does not prove eventual
cleanup across infinite blocking or forced aborts. The language has no user `break`/`continue`, and this slice
does not add them.

### 3.4 `par` and contracts

- Add `async` to the `par_info` forbidden effects (`type/parallel_effect`), next to `io`, `fs`, `proc` and
  `net`. Direct and indirect `async` calls are rejected. A loop is single-owner and its threads are internal;
  sharing it with `par` branches is later work (§9).
- Define an `ASYNC_OBSERVERS` list: `ev_ok`, `ev_error`, `ev_cap`, `ev_count`, `ev_last`, `ev_kind`, `ev_eof`,
  `ev_write_open`, `ev_event_slot`, `ev_event_kind`, `ev_size`, `ev_byte`, `ev_exit`. Calling any other `ev_*`
  operation, or the allocating `ev_bytes`, in a contract is `type/foreign_contract`.
- Observer domain requires apply to the domain checks of both code and contracts: `ev_kind`, `ev_eof` and
  `ev_write_open` need a slot in range; `ev_byte` needs an index below `ev_size`.

### 3.5 Drop

A `Loop` never ends normally through auto-drop. The generated runtime has a defensive `Drop` (best-effort:
shut sockets down, close listeners, kill and reap processes, join threads) for unwinding and internal bugs, but
it does not satisfy must-consume. The handles inside a loop are owned by the loop; their own drop adapters are
not used. Other drop policies are unchanged.

## 4. Verification encoding

Like net §4, plus the slot table. The encoding is added next to the net encoding: Python `pruvc/verify.py`
(`FnVerifier.net_call`, with a new `async_call` beside it) and Rust `P/symbolic.rs` (the `net_*` arms), with
the shared obligations in `P/obligation.rs`.

1. **Loop value.** A symbolic record with the public state only:

   ```text
   error: Int
   cap: Int
   count: Int
   last: Int
   ev_slot: Int
   ev_kind_: Int            -- the last wait's event kind
   exit: Int
   buf: (length: Int, bytes: Array Int Int)
   kind: Array Int Int      -- slot -> 0..4
   eof: Array Int Bool      -- slot -> bool
   write_open: Array Int Bool
   ```

   `ev_ok` is `error == 0`. The OS descriptors, threads, queues, timers, round-robin position, per-stream
   process flags, the pending-error flags and the first recorded error are **not** modeled.

2. **Well-formedness.**
   - `0 <= error <= 2`; `1 <= cap <= 4096`; `0 <= count <= cap`; `-1 <= last < cap`; `-1 <= ev_slot < cap`;
     `0 <= ev_kind_ <= 5`; `(ev_slot == -1) == (ev_kind_ == 0)`.
   - Buffer length in `0..M`; bytes at valid indexes in `0..255`.
   - For slots in `0..cap-1`: `0 <= kind[i] <= 4`; `eof[i] ==> kind[i] == 1 || kind[i] == 3`;
     `write_open[i] ==> kind[i] == 1 || kind[i] == 3`. These facts are quantified over the slot range with the
     same pattern the existing byte-map facts use for `0..len-1`; no sum is quantified.
   - `last != -1 ==> kind[last] != 0`.
   - `error != 0 ==> last == -1 && ev_slot == -1 && length == 0`. A failed loop keeps its slot table (its
     resources are still owned and `ev_close` ends them), so no fact is required about `kind`, `eof` or
     `write_open` when `error != 0`.

   EOF true and an empty buffer are not made globally equivalent, as in net.

3. **Slot table and frames.** `kind`, `eof` and `write_open` are SMT arrays from slot to value. Each catalog
   contract states **exactly which slots change**: the result's arrays are the entry arrays with `store` at the
   named slot(s) and nothing else. So `ev_read(l, a, ..)` leaves `kind`, `eof` and `write_open` of every slot
   `b != a` untouched, and the verifier can prove that after `ev_accept` the listener slot is still kind 2 (the
   new slot was free, so it differs from it). Write the success and failure cases exactly as the §2.3 and §2.4
   transitions; do not drop frame facts because the result is fresh.

4. **Count.** `count` is **not** encoded as the number of non-free slots (that would be a quantified sum). Only:
   `0 <= count <= cap` as well-formedness; an add or accept that produced a slot sets `count + 1`; `ev_remove`,
   `ev_proc_wait` and a fired timer (`ev_wait` with event kind 4) set `count - 1`; every other operation keeps
   `count`. The requires `count < cap` of the adding operations is then provable from these facts.

5. **Wait.** `ev_wait` produces a fresh event pair constrained exactly by §2.3: ranges, the relation between
   `-1` and kind 0, `timeout_ms == -1 ==> kind != 0`, the entry kind of the event slot being non-free and
   matching the event kind, the freed slot and decremented count on a timer, write-open and EOF false at the
   event slot on a slot error, and an unchanged table otherwise.
   Two `ev_wait` calls on equal loop values are independent external results and are never treated as equal.

6. **Effect calls.** Same order as `File`, `Process` and `Socket`: catalog requires → fresh result →
   well-formedness → ensures. Results are never memoized. Two `ev_accept` calls on the same slot, two reads,
   two waits: independent results.

7. **Preservation.** The buffer (length and byte map), event, exit and `cap` are preserved by every operation
   whose §2.3 row says B. `ev_read` and `ev_proc_read` replace the buffer with a fresh one of length in
   `0..max`. `ev_proc_wait` sets `exit` freely (no number is promised); everything else keeps it. `ev_new`
   constrains every field.

8. **Observers and copies.** Observers are field reads or array selects. `ev_bytes` is a new array with the
   snapshot's length and byte map; at runtime its memory is independent.

9. **Terminal result.** Use the existing `Result<i64>` encoding.
   - If `error != 0` at entry: `is_err(result)` and `1 <= err_val(result) && err_val(result) <= 8`.
   - Otherwise either `is_ok(result) && ok_val(result) == count` (the entry count) or
     `is_err(result) && 3 <= err_val(result) && err_val(result) <= 8`.
   - An unconditional `Ok` is **not** provable (a slot may have failed), and `Ok(n)` with `n != count` is
     never provable.

10. **Entry, return, havoc.** Function parameters, user function returns, loop havoc, counterexample output and
    type recovery handle `Loop`. After havoc only well-formedness and proven invariants remain; earlier `ok`,
    kinds, counts, sizes, events and EOFs are not kept. A loop invariant that mentions `ev_kind(l, s)` is how a
    program keeps slot facts across iterations.

11. **Domains.** Register `ev_kind`, `ev_eof`, `ev_write_open` (slot) and `ev_byte` (index) for
    contract-domain checks like the File byte observer. A postcondition cannot become true through an observer
    with an unchecked slot or index.

12. **Ownership and termination.** Must-consume is static and is not re-encoded in SMT. No axioms about
    peers, data arrival, event timing, timer expiry, thread scheduling, fairness, time or total memory. The
    round-robin promise and "lowest free slot" are runtime meaning checked by A2, not proved.

13. **Inference.** `--infer` guesses no OS-state invariants, slot-table invariants or termination measures; it
    only gains the branches needed so `Loop` is not treated as a scalar.

## 5. Rust source backend

### 5.1 Representation

```rust
struct pruv_Loop {
    shared: std::sync::Arc<pruv_LoopShared>,   // Mutex<LoopState> + one Condvar
    slots: Vec<pruv_Slot>,                     // cap entries, owner-thread only
    // public state, changed only by explicit operations
    error: i64,
    first_err: i64,                            // first published failure, for ev_close
    last: i64,
    event_slot: i64,
    event_kind: i64,
    cursor: usize,                             // slot reported last, for the round-robin scan
    buf: Vec<u8>,
    exit: i64,
}

enum pruv_Slot {
    Free,
    Socket { stream: TcpStream, reader: JoinHandle<()>, write_open: bool, eof: bool, failed: bool },
    Listener { local: SocketAddr, acceptor: JoinHandle<()>, failed: bool },
    Process { child: Child, stdin: Option<ChildStdin>, collectors: [JoinHandle<()>; 2],
              write_open: bool, eof: [bool; 2], failed: bool },
    Timer { deadline: Instant },
}
```

The shared state holds, per slot, the data the worker threads fill: `queue: Vec<u8>` (for a process, one per
stream), `ended`, `err`, `closing`, `reported_end`, `reported_err`, and for a listener the `pending` queue of
accepted streams (`Ok(stream)` or an error marker). Public state and worker state are separate: observers read
only the public fields and the owner-side slot fields, never the shared queues.

There is one `Mutex` and one `Condvar` for the whole loop. Every worker signals the condvar after pushing data,
an end or an error, and after a queue is drained (to wake a reader stopped by backpressure).

### 5.2 Adding

1. `ev_new` allocates the slot table and the shared state; it starts no thread.
2. `ev_add_socket` clears the read timeout (`set_read_timeout(None)`; the timeout is a property of the shared
   descriptor and would make the reader fail), keeps the write timeout, duplicates the stream with
   `try_clone`, and spawns the reader. Failure of any of these is loop error 2 and the stream is dropped.
3. `ev_add_listener` duplicates the listener, records its `local_addr`, and spawns the accept thread. Failure is
   loop error 2.
4. `ev_add_process` attaches the loop's notifier and the 1 MiB bound to the process's two output queues (the
   collectors already exist, from the proc runtime) and signals the condvar once. Bytes already queued move
   over unchanged; the bound only stops further collection (§2.5 backpressure). Failure is loop error 2, and
   the process is killed and reaped.
5. `ev_add_timer` stores `Instant::now() + ms`. No thread.

The slot index is the lowest `Free` entry.

The proc collectors are reused, not rewritten (proc §5.3): the same two threads per process feed a queue. In a
loop slot that queue is bounded and signals the loop's condvar. A program that does not use `Loop` must keep
the generated proc runtime byte-for-byte; A2 either emits an extended collector only when `Loop` is used or
makes the bound and notifier inert for standalone processes, and gate A2.12 checks that identical text.

### 5.3 Worker threads

- **Socket reader.** Loop: lock; while the queue holds 1 MiB (or more) and the slot is not `closing`, wait on
  the condvar; unlock; read into a buffer of `min(64 KiB, 1 MiB - queued)` bytes with a blocking `read`
  (retrying `Interrupted`); lock; push the bytes, or set `ended` on 0 bytes, or set `err` (3) on any other error;
  signal. It exits on `ended`, `err` or `closing`. A panic is recorded as `err` 3 by a guard.
- **Accept thread.** Loop: wait while the pending queue holds 64 streams or the slot is `closing`; blocking
  `accept` (retrying `Interrupted`); on success set `TCP_NODELAY` and push the stream; on error push an error
  marker and exit; signal. It exits on `closing`, and a stream accepted while closing is dropped.
- **Process collectors.** The proc runtime's two threads per child (proc §5.3), with the bounded queue and the
  loop's condvar. A read error is slot error 7.

### 5.4 `ev_wait`

1. Lock. Compute readiness for each slot in round-robin order starting at `cursor + 1` (modulo `cap`), per the
   event table of §2.3: queue non-empty (1), pending connection (2), drained end not yet reported (3),
   unreported error (5), timer deadline passed (4). A failed or already-reported slot is skipped.
2. The first ready slot is the event. Set `cursor` to it. A fired timer's slot is set to `Free` here.
3. If nothing is ready and the timeout is not `0`: wait on the condvar with
   `min(remaining timeout, nearest timer deadline - now)` (no timeout when both are absent) and rescan after
   every wakeup. Spurious wakeups only cause a rescan. A timeout of `-1` with no timers waits until a worker
   signals.
4. On timeout, `event_slot = -1`, `event_kind = 0`, `cursor` unchanged.

### 5.5 Slot operations

- `ev_read` takes up to `min(max, 1 MiB)` bytes from the front of the socket's queue; if the queue is empty and
  the slot has `ended`, EOF becomes true (and counts as the end event having been published); if it has an
  `err`, the slot is marked failed (code 3), the buffer is empty and EOF false. It signals the condvar so a
  reader stopped by backpressure resumes.
- `ev_write` uses `write_all` on the owner's stream handle without holding the loop lock; a write error is code 4,
  a write timeout code 5; a broken pipe never aborts the process (`std` suppresses `SIGPIPE`; no handlers are
  installed). `ev_shutdown_write` calls `shutdown(Shutdown::Write)`; failure is code 4.
- `ev_accept` pops the front of the pending queue. `Ok(stream)`: take the lowest free slot, clear timeouts, set
  `TCP_NODELAY` (already set), `try_clone`, spawn a reader, set `last`. `Err`: mark the listener slot failed
  (code 6). Empty queue: no-op, `last = -1`.
- `ev_proc_read` takes up to `min(max, 1 MiB)` bytes from the chosen stream's queue, marks that stream ended
  when its queue is empty and its collector has finished (EOF true when both are), or marks the slot failed
  (code 7) on a collector error. `ev_proc_write` and `ev_proc_close_stdin` follow proc §5.4 (`write_all`,
  dropping the stdin endpoint).
- `ev_proc_wait` closes stdin, keeps draining and discarding both outputs, waits, joins the collectors, frees
  the slot, and stores the exit status (proc §2.4 rules 8–10) in `exit`. On a failed slot it kills first.
  A reap failure is code 8 with `exit = 0`.
- `ev_exit` is only changed by `ev_proc_wait`.

### 5.6 Removal and close

`ev_remove` ends the slot, then joins its threads, then frees the slot. Per kind:

- **Socket.** Mark `closing`, `shutdown(Both)` (wakes a reader blocked in `read` on every supported OS), join the
  reader, drop the streams.
- **Listener.** Mark `closing`, wake the accept thread with **one loopback connect to its own port**
  (`local_addr`; an unspecified address is replaced by `127.0.0.1` or `::1`), join the thread, drop the
  listener. If the wake-up connect fails the thread is detached instead of joined and the port stays bound
  until the process ends; this is recorded as loop error 2. Connections still in the pending queue are
  dropped.
- **Process.** Kill the child, reap it, join the collectors. A kill or reap failure is code 8 and the slot is
  freed anyway.
- **Timer.** Drop the deadline.

`ev_close` removes every non-free slot in ascending order, counts them, keeps the first failure, then drops the
shared state. Its result is built from the counted slots and the recorded first failure (§2.3).

### 5.7 Runtime contract checks

Follows the existing `--foreign-checks requires|all|off`.

- An enabled requires violation uses the existing `pruv: foreign requires violated:` prefix and exit 101.
- Under `--no-verify`, the default foreign requires checks still catch an out-of-range slot, the wrong slot kind,
  a write to a closed side, a full loop on add, and bad cap, ms, max, stream and timeout values.
- With checks off, a bad slot or kind still cannot cause UB; the runtime's representation defense fails the
  loop with error 1.
- OS failures never use this panic path.

### 5.8 Test-only fault hooks

Failures the OS cannot produce reliably are tested through a compile-time environment variable
`PRUV_ASYNC_TEST_HOOK`, like `PRUV_NET_TEST_HOOK` and `PRUV_PROC_TEST_HOOK`. It is read when the generated
runtime is built for a test, forces the named failure, and does not exist in normal builds. It is not part of
the user API.

| Hook | Forced failure | Error |
|---|---|---|
| `spawn` | the next worker thread spawn fails | loop error 2 |
| `read` | a socket reader reports a read error | slot code 3 |
| `write` | a socket write or shutdown fails | slot code 4 |
| `write_timeout` | a socket write times out | slot code 5 |
| `accept` | an accept thread reports an accept error | slot code 6 |
| `proc_io` | a process collector or stdin write fails | slot code 7 |
| `reap` | reaping or killing a child fails | code 8 |

The test build also keeps a counter of live loop threads that the Rust host driver can read, so thread leaks are
testable. Neither is a user API.

## 6. LLVM backend

### 6.1 Same Rust shim as File, Process and Socket

1. LLVM holds `Loop` as an opaque pointer.
2. The generated Rust bindings' G6 shim contains **the same runtime/helper code** as §5 (and the net and proc
   runtime it builds on).
3. Catalog adapters evaluate arguments once, in order.
4. A take adapter unboxes its owned box exactly once. `ev_add_socket`, `ev_add_listener` and `ev_add_process`
   unbox **both** the loop and the handle, once each, and return one new `Loop` box. The handle's box is gone.
5. Updates return a new owned box. `ev_close` does not create a new box.
6. A moved LLVM slot becomes null so scope cleanup does not consume it again. This holds for the handle moved
   into a loop, too.
7. The `Loop` drop adapter is defensive only; a live `Loop` drop in a normal program is rejected by §3.
8. Observers that appear only in contracts are still collected for the shim.

### 6.2 Adapter ABI (shim-internal, not host ABI)

| Kind | Shape |
|---|---|
| new | `cap: i64` → Loop pointer |
| add handle | Loop pointer, handle pointer (Socket, Listener or Process) → Loop pointer |
| add timer | Loop pointer, `ms: i64` → Loop pointer |
| wait | Loop pointer, `timeout_ms: i64` → Loop pointer |
| slot update | Loop pointer plus scalar/slice arguments (`slot`, `max`, `stream`, `data` as `(ptr,len)`) → Loop pointer |
| remove, process wait | Loop pointer, `slot: i64` → Loop pointer |
| observer | borrowed pointer plus scalars → `i64`; bool is 0/1 |
| close | Loop pointer, `tag_out: ptr`, `value_out: ptr` → void |
| bytes snapshot | borrowed Loop pointer → borrowed `(data,len)` view through out parameters |

Terminal shim tags are fixed: `1 = Ok`, `0 = Err`. LLVM assembles the existing internal Result from them. Host
Result returns keep the existing target ABI lowering.

`ev_bytes` copies the borrowed view at once with the existing `pruv_copy_v2(..., size=1, align=1)`; the handle
is not updated or consumed in between. Rust-allocated buffers are never passed to C `free`. Copying a whole
buffer never calls the shim once per byte.

### 6.3 `runtime.c`

**No new functions in `runtime.c`.** Only the existing v2 byte copy, allocation, free and representation checks
are used. No C threads, `poll`, `select`, `epoll` or timer code. Platform I/O for both backends lives in the
shared Rust runtime. The C runtime ABI and allocator markers do not change. A standalone event runtime without
the Rust shim is later work and needs its own C runtime design.

### 6.4 Build limits

- Run with `--backend llvm --rust-bindings PATH` and a Rust host build.
- Native `--emit exe|obj` for programs that need the shim keeps `codegen/foreign_target`.
- A program that only declares `! async` without using the catalog does not get the shim limit.

## 7. Diagnostics and compatibility

### 7.1 New diagnostic

| Stage / code | Condition | Hint |
|---|---|---|
| `type/async_unconsumed` | a `Loop` duty is left at scope end, return, overwrite or a discarded temporary | consume it with `ev_close(move l)`, or return / transfer ownership |

`type/proc_unconsumed` and `type/net_unconsumed` stay. The three diagnostics share one analyzer; the code
follows the type of the handle that loses its duty. A `Socket`, `Listener` or `Process` that was moved into a
loop has no duty left, so it produces none of them. Locations follow proc §7.1 (return, assignment, temporary
expression, binding declaration).

### 7.2 Reused diagnostics

| Condition | Diagnostic |
|---|---|
| missing `async`, `net`, `proc` or `alloc` | `effect/missing_effect` |
| unknown effect | existing parse diagnostic, longer hint list |
| double close, use after move (including a handle after `ev_add_*`) | `type/foreign_move` |
| consuming a borrowed Loop | `type/foreign_borrow` |
| loop does not restore an outer owner | `type/foreign_loop_move` |
| bad placement of an owned type | `type/foreign_owned_type` |
| handle in a host entry | `type/entry_signature` (`pub fn`), `type/foreign_export` (`fn main`) |
| trusted declaration bypassing signatures | `type/foreign_signature` |
| effect operation in a contract | `type/foreign_contract` |
| `async` on a `par` path | `type/parallel_effect` |
| unproved catalog requires (slot range, kind, free slot, write-open, ranges) | `verify/call_requires` |
| contract observer domain not met | existing `verify/contract_domain` |
| staging guard for unimplemented batches | `codegen/unsupported_type` |
| native output without the shim | `codegen/foreign_target` |

A0 searches the repository once for name clashes of `async_unconsumed`. Runtime error values 1–8 are read only
inside `ev_error` and the `ev_close` Result, so they do not clash with compiler diagnostics or OS exit codes.

### 7.3 Compatibility

- Programs that do not use the new features keep their ownership and drop meaning and generate identical Rust,
  LLVM and bindings. This includes programs that use `Process`, `Socket` or `Listener` without `Loop`: the
  generated proc and net runtimes do not change (§5.2).
- The unknown-effect hint change is an intended diagnostic change.
- An existing user struct or enum named `Loop` keeps its meaning.
- `File`, `Process`, `Socket` and `Listener` are not redesigned. The generalized must-consume rule must leave
  every existing `Process`, `Socket` and `Listener` diagnostic (code, location, message) unchanged.
- The `ev_*` names are new catalog names. A user function with such a name keeps priority, as with `net_*`
  today.

## 8. Batches and gates

Each batch finishes the Python implementation and the Rust port together. New frontend, proof and runtime rows
must agree between the compilers. Generated text must be byte-identical for the same target and options.
Network timing, chunk boundaries and event timing are not compared; reassembled bytes and the sequence of
event kinds a test fixes are. All runtime tests use 127.0.0.1 only and local fixture children. During a batch
only that batch's tests run; the full suite and row comparison follow the repository commit procedure.

### A0. Effect, types, must-consume

Files: `pruvc/vparser.py`, `pruvc/vast.py`, `pruvc/typeinfo.py`, `pruvc/check.py`, `pruvc/foreign.py`,
`pruvc/pruvc.py`, `C/parser.rs`, `C/typeinfo.rs`, `C/gtype.rs`, `C/check.rs`, `C/foreign.rs`, `D/driver.rs`,
`tests/run_all.py`, `tests/run_all_rows.txt`, new `tests/g10_async/frontend/*.pruv`, new
`tests/must_reject/g10a_*.pruv` with `.json`, this document.

Work: the `async` effect, the `Loop` type, catalog signatures and effect registration (`ev_*`, `ASYNC_OPS`,
`ASYNC_OBSERVERS`); the must-consume generalization over {Process, Socket, Listener, Loop} in the existing
analyzer, with the new `type/async_unconsumed`; handles moved into `ev_add_*` as consumes; the `par` effect; a
staging guard (`codegen/unsupported_type`) for unimplemented parts; the diagnostic name clash search for
`async_unconsumed`.

Gates:

1. Moves into a loop, helper returns, a loop passed to a `take` helper, and the two §2.6 programs
   (`echo_two`, `relay`) type-check.
2. Each of these is rejected with its diagnostic: double `ev_close`, use after move, consuming a borrowed loop;
   a loop consumed in one branch only, missing on an early return, a forgotten take parameter; overwriting a
   live loop, discarding a borrowed temporary (`ev_ok(ev_new(4))`), discarding an update result; a loop local
   left unconsumed, an outer loop not restored; placement in Result/struct/collection, host entry exposure; a
   missing `async` effect, a missing `net` or `proc` effect on `ev_remove` and `ev_close`, an indirect `par`
   effect; use of a `Socket`, `Listener` or `Process` after `ev_add_*` took it; a socket closed with
   `ev_close` or a loop closed with `net_close` (type mismatch); a `Loop` stored in an array or struct.
3. Consumed in both branches, `l = update(move l)` and `return move l` are allowed.
4. Existing File auto-drop cases and every proc and net case (including `type/proc_unconsumed` and
   `type/net_unconsumed`) are unchanged.
5. Programs using `Loop` stop at the staging guard in every mode. A function with only `! async` is not
   guarded.
6. Frontend diagnostics (stage/code/location) and output agree between the compilers.

The A0 catalog contract placeholders can never run past the staging guard or be reported as proved.

### A1. Verification

Files: `pruvc/foreign.py`, `pruvc/verify.py`, `pruvc/pruvc.py`, `C/foreign.rs`, `P/symbolic.rs`,
`P/obligation.rs`, `D/driver.rs`, `tests/run_all.py`, `tests/run_all_rows.txt`, new
`tests/g10_async/proof/*.pruv`, new `tests/must_reject/g10a_v_*.pruv` with `.json`.

Work: the §4 encoding and the full catalog contracts, including the slot table with its frames, the wait
ensures and the terminal result; observer domains, call-entry snapshots, havoc and counterexamples; move the
guard to the codegen entry, after verification.

Gates:

1. add a socket, a listener, a process and a timer; wait; read, write, shutdown, accept and remove with the
   right kinds verify; both §2.6 programs verify.
2. Paths that close a failed loop (`!ev_ok`) verify.
3. Rejected: an operation on a failed loop; a slot out of range; `ev_read` on a listener, free or timer slot;
   `ev_write` without `ev_write_open`; `ev_write` after `ev_shutdown_write`; an add to a full loop; a bad
   cap (0 and 4097), ms, max, stream, timeout or index; `ev_remove` of a slot that was just removed.
4. Frames verify: after `ev_read` on slot `a`, another slot `b` keeps its kind, EOF and write-open; after
   `ev_accept` the listener slot is still kind 2; after `ev_remove` of `a`, `b` is untouched.
5. The wait ensures verify: `-1 <= ev_event_slot < cap`, the event kind range, an event slot was non-free
   before the call, a timer event frees its slot and decrements the count, `timeout_ms == -1` gives a non-zero
   kind.
6. Count: `0 <= ev_count <= ev_cap` verifies; an add increments, a remove, an `ev_proc_wait` and a fired timer
   decrement. A claim that `ev_count` equals the number of non-free slots is not needed and is not provable.
7. Claims that use an earlier `ok`, kind, count, size, EOF or event after loop havoc without an invariant are
   rejected. The same claims with an invariant verify.
8. Claims that two identical waits or two accepts give equal results, claims about the content a peer or child
   sends, and claims about how long a wait takes are rejected.
9. `ev_close` returning `Ok(ev_count)` or `Err` in `3..8` (`1..8` if `ev_error != 0`) is provable; an
   unconditional `Ok` is not.
10. Wrong observer domains in contracts are rejected.
11. A finite wait-budget server (`echo_two`) verifies. A loop claiming to decrease by a peer event alone does
    not pass.
12. New obligations, conclusions and diagnostics agree between py and rs.

### A2. Rust runtime and source generation

Files: `pruvc/codegen.py`, `pruvc/foreign_codegen.py`, `pruvc/foreign.py`, `C/codegen.rs`,
`C/foreign_codegen.rs`, `C/foreign.rs`, `pruvc/pruvc.py`, `D/driver.rs`, `tests/run_all.py`,
`tests/run_all_rows.txt`, `tests/g10_proc/child.rs` (new fixture modes if needed), new
`tests/runtime/g10a_r*.pruv`, `.rs`, `.json`.

Work: the §5 runtime and helpers (an async prelude emitted only when `Loop` is used), the snapshot copy, the
test hooks and the thread counter; remove the Rust backend guard (the LLVM guard stays). The `.rs` test driver
is the Rust host main. It starts its own 127.0.0.1 peer threads with `std::net` (echo clients, a silent peer, a
flooding sender, a client that connects to the Pruv listener) and uses the proc fixture child, and passes ports
and paths to the Pruv `pub fn`. There is no external fixture program.

Gates:

1. A two-client echo server (the `echo_two` program): two peers send interleaved chunks of different sizes and
   each receives exactly its own bytes back, in order.
2. A child streaming while a socket is served: the fixture child writes to stdout and stderr (more than the
   pipe capacity on both) while a client is served by the same loop; the relay program delivers the child's
   stdout bytes exactly, the child never stalls on stderr, and the exit status arrives through `ev_exit`.
3. A timer timeout: `ev_wait` with timeout 0 and nothing ready returns kind 0 and slot `-1`; a 50 ms timer
   fires exactly once with kind 4, not before 50 ms, and its slot is free afterwards; the relay's deadline
   timer kills a child that never exits.
4. Cancellation of a blocked peer: removing a socket slot whose reader is blocked (a silent peer) returns
   promptly and the peer sees EOF or a reset; removing a listener slot wakes its accept thread, returns, and
   the port can be bound again; removing a running child kills it; `ev_close` ends all of them and returns
   `Ok` with the number of slots.
5. Fairness: with two slots that stay ready, successive waits report them strictly alternately; a flooding
   peer cannot keep a slow peer from being served.
6. Backpressure: while the program does not read, a peer sending more than 1 MiB is stopped (its write times
   out) and, once the program reads, every byte arrives exactly once. A child that writes more than the queue
   bound blocks and then resumes.
7. Events: data is reported before the end; end of stream and errors are reported once; `ev_read` reports
   EOF true only after the drained end; accepting with nothing pending is a no-op with `ev_last == -1`;
   `ev_read` on a failed slot is a no-op.
8. Hooks (§5.8): `read`, `write`, `write_timeout`, `accept`, `proc_io` mark the slot (3, 4, 5, 6, 7), report
   event kind 5 once, keep `ev_ok` true, and leave only `ev_remove`; the loop keeps serving its other slots;
   `reap` gives code 8 at close; `spawn` fails the loop with `ev_error == 2` and `ev_close` returns `Err(2)`.
9. A reset peer and a broken pipe do not abort the process (hooks where the OS cannot produce them reliably).
10. Termination after a failed loop, a failed slot or an early return releases every descriptor, listener port
    and thread (the thread counter is back to its baseline), and kills and reaps children.
11. Runtime requires violations exit 101 and are distinct from OS failures.
12. Generated Rust is identical between py and rs, and programs that do not use `Loop` generate the same Rust
    as before this batch (including the proc and net runtimes).

### A3. LLVM shim, Windows, user spec

Files: `pruvc/llvmgen.py`, `pruvc/foreign_shim.py`, `C/llvmgen.rs`, `C/foreign_shim.rs`, `D/driver.rs`,
`tests/run_all.py`, `tests/run_all_rows.txt`, `tests/runtime/g10a_r*.pruv`, `.rs`, `.json`, new
`tests/g10_async/windows.py`, `SPEC.md`, this document. `pruvc/runtime.c` does not change.

Work: `Loop` adapters (including the add adapters that take two owned boxes), the Result out-parameter
adapter, borrowed bytes view plus v2 copy; remove the LLVM guard; a new SPEC.md §6n of at most one page of
async user rules (after §6m, in the form of §6l and §6m: a table of operations, the error codes, one example).

Gates:

1. The A2 runtime cases pass on LLVM too.
2. `Result` Ok/Err and payloads are never swapped.
3. G6 live boxes are 0 after adds, waits, slot operations, removes, branches, helper returns and `ev_close`,
   including the handle boxes consumed by the add adapters.
4. An `ev_bytes` copy stays valid after the loop is updated or closed.
5. Observers that appear only in contracts are linked.
6. Generated IR and bindings are identical between py and rs.
7. The native output limit is `foreign_target` and does not conflict with the existing File, Process and net
   limits.
8. On Windows, `tests/g10_async/windows.py` runs the cases for real: a two-client echo loop on 127.0.0.1, a
   timer, a child streaming while a socket is served, removal of a blocked reader and of a blocked accept
   thread (`shutdown` and the loopback wake-up connect work), and a backpressured peer. Binding 127.0.0.1
   does not raise the Windows Defender firewall prompt.

Building a Windows artifact is not a Windows run. Without a run environment, gate 8 stays pending, recorded as
such.

### Q. Quality

Files: new `bench/g10_async/PROBLEMS.md`, `bench/g10_async/solutions/q01.pruv`, `q02.pruv`,
`bench/g10_async/run.py`, `mutate.py`, `perf.py`, `bench/g10_async/faults/`, `bench/g10_async/results/`,
`RESULTS.md`, `BLUEPRINT.md`, this document.

Problems:

1. A multi-client echo/line server with a per-client timeout: listen on port 0, announce the port on a control
   connection, serve up to 8 clients on one loop, assemble lines per client byte by byte (LF across chunks,
   a per-client buffer indexed by slot), reply to each line with its length, drop a client that is silent for
   a fixed time using one timer slot per client (re-armed by removing the old timer and adding a new one), and
   end after a budget of clients with a final `ev_close`. Line assembly is byte-based; no UTF-8 conversion is
   assumed.
2. A supervisor: accept a client, run a fixture child, stream the child's output lines to the client while
   accepting a cancel request from it; on cancel kill the child and reply, on child exit reply with its exit
   status, on a deadline timer kill the child and reply; stderr is drained; every path ends the loop with
   `ev_close` and consumes every handle exactly once.

Gates:

1. Both reference solutions pass on all four py/rs × rust/llvm combinations.
2. An independent author solves both from `SPEC.md` and the problems only; first result and fix count are
   recorded.
3. 16 explicit wrong mutants: missing `ev_close`, overwrite of a live loop, close in one branch only, a
   discarded temporary, an unchecked `ev_ok` before an add or accept, a slot used after `ev_remove`, `ev_read`
   on a listener slot, a write after `ev_shutdown_write`, a slot or cap out of range, an add to a full loop,
   an end of stream never removed (the slot spins), a timer slot kept after it fired, line framing off by one,
   a chunk dropped after the end event, a cancel request ignored, a child's stderr never drained.
4. Static and contract mutants are caught at compile time; wrong handling of external data or event order is
   caught by the harness oracle. The two are reported separately; no claim that all external behavior is
   proved. The split is recorded at Q, not predicted here.
5. Four compiler faults are caught by named tests: removing `Loop` must-consume, removing a catalog requires
   (the slot-kind requires of `ev_read`), a wrong frame (`ev_remove` that leaves the count or another slot's
   kind changed), an inverted Result tag of `ev_close`.
6. One fixed workload (the measurement): N = 50 concurrent loopback clients each send 64 KiB (the same
   deterministic bytes) to the loop server, which echoes them back, with zero loss and duplication; the Pruv
   server is measured against a hand-written Rust server that uses one thread per client. Time, peak memory
   and live thread count are reported for both. **No target.**
7. No performance target is declared without evidence. Per-byte shim cost and bulk copy cost are recorded
   separately, as in net Q.
8. Results and limits go into `RESULTS.md`; the async status in `BLUEPRINT.md` is updated. Existing File, proc
   and net measurements are not replaced.

## 9. Later work

- `async fn` / `await` syntax over the same runtime.
- Taking handles back out of a loop (`ev_take_socket` and similar), so a program can hand a connection to a
  `net_*` routine.
- Non-blocking writes with output queues, so `ev_write` never blocks, and write readiness events.
- UDP and TLS in loops.
- Sharing a loop across `par` branches.
- An OS poller (epoll, kqueue, IOCP) instead of one thread per slot, which removes the per-slot threads and the
  thread count limit.
- Connect timeouts and a non-blocking connect that becomes a slot.
- A per-slot error observer (the cause of a failed slot, today only visible in the `Err` of `ev_close`).
- Per-slot socket options (a write timeout for accepted sockets, `TCP_NODELAY` control).
- Waiting for one stream of a process, process groups and descendant trees (as in proc §9).
- A C event runtime for LLVM without the Rust shim.
- Verified framing or protocol libraries over loops, and a fairness proof (today fairness is a runtime gate).
