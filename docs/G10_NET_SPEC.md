# G10: the `net` effect and the owned `Socket` and `Listener` handles

Status: **Design, 2026-11-16. A0 is next.**
Batch status will be recorded per batch below and in `RESULTS.md` with Q.

This document defines the network slice that `docs/G10_SPEC.md` §9 deferred (`net` with a `Socket` handle). A
Pruv program can connect to a host and port, listen on a port, accept connections, write and read bytes on a
TCP stream, half-close it, bound a single wait with a timeout, and end ownership with `net_close` or
`net_listener_close`.

Python `pruvc` gets each feature first and `pruvc-rs` ports it in the same batch. The Rust source backend
uses `std::net`. The LLVM backend reaches the same Rust runtime through the G6 Rust shim, exactly as `File`
and `Process` do. There is no C socket runtime in this slice.

The rules and gates in §3–§8 are acceptance conditions, not measurements. `docs/G10_PROC_SPEC.md` is the
model for ownership, failure values and the Result terminal adapter; this document repeats only what
differs.

## 1. Goals

- Open blocking TCP connections to a host name or a literal IPv4/IPv6 address.
- Listen on a port (an OS-chosen port is possible), accept connections one at a time.
- Write strings or bytes, half-close the write side, read in chunks or read everything that is left.
- Bound one read or write wait with a timeout.
- A `Socket` and a `Listener` can move, but on every normal control-flow path their last owner must consume
  them with `net_close` / `net_listener_close`.
- Connect, listen, accept, read and write failures are values.
- Both compilers and both backends agree on meaning, diagnostics and generated output.
- Local tests use only the 127.0.0.1 loopback address and never depend on an outside network.

Non-goals of this slice: TLS, UDP, HTTP, a DNS API, async, poll or multiplexing. Async is the next slice.
The `Process` must-consume rule becomes a rule over {`Process`, `Socket`, `Listener`} in the same analyzer.
The `File` auto-close policy does not change.

Repository facts this design relies on (HEAD 84404dd):

- The effect list is `io`, `alloc`, `fs`, `proc` (`pruvc/vparser.py`, `C/parser.rs`). The `proc` slice is
  implemented through A3 and Q.
- Must-consume exists for `Process`. The ownership analyzer is the Python `_Ownership` in `pruvc/foreign.py`
  and the Rust `Ownership` in `C/foreign.rs`; `type/proc_unconsumed` is reported from there and, for
  discarded temporaries, from `pruvc/check.py` and `C/check.rs`.
- The proc slice added owned catalog handles whose effect operations take a borrowed owned argument
  (`proc_spawn` borrows `ProcessArgs`), and the `Result<i64>` terminal adapter with `tag_out` / `value_out`.
- An owned catalog type cannot sit inside `Result`, so `Result<Socket>` is not allowed. A failed connect is
  therefore a failed `Socket` value, as with `file_ok` and `proc_ok`.
- The `par` forbidden-effect collection lists `io`, `fs` and `proc` explicitly (`pruvc/check.py` `par_info`,
  `C/check.rs`), and the violation is `type/parallel_effect`.
- LLVM keeps `File` and `Process` as opaque pointers; consuming adapters unbox, updates return a new box;
  scope cleanup calls the drop adapter on non-null owned slots. Native `--emit exe|obj` refuses shim programs
  with `codegen/foreign_target`.

Path abbreviations: `C/` = `pruvc-rs/crates/pruv-core/src/`, `P/` = `pruvc-rs/crates/pruv-proof/src/`,
`D/` = `pruvc-rs/crates/pruvc/src/`.

## 2. User surface

### 2.1 The `net` effect

`net` is the fifth effect name (`io`, `alloc`, `fs`, `proc`, `net`).

```pruv
fn fetch(host: str, port: i64, request: [u8]) -> i64 ! net, alloc
```

`net` does not include `io`, `fs` or `proc`. Reading from a socket is `net`; Pruv's own `print` stays `io`.

Effect operations (`net_listen`, `net_accept`, `net_connect`, writes, shutdown, timeout, reads, closes) need
`net, alloc`, because buffers allocate. `net_bytes` needs `alloc` only. Observers have no effect.

### 2.2 Types

**`Socket`** — owned catalog type, internal key `socket`, generated Rust name `pruv_Socket`. A connected TCP
stream.

**`Listener`** — owned catalog type, internal key `listener`, generated Rust name `pruv_Listener`. A bound,
listening TCP socket.

Both follow the placement limits of `Process`:

- Allowed as a local, a `take` parameter, a read-only borrowed parameter, and the result of an internal
  function.
- No copies, no aliases, no storage inside owned containers (struct, enum, array, Option, Result,
  collections).
- Cannot be a parameter or result of a host entry.
- Moves use `move s`.
- Both are must-consume. Auto-drop cannot discharge them. A `Socket` is ended only by `net_close`; a
  `Listener` is ended only by `net_listener_close`. A handle from a failed connect, listen or accept must
  also be consumed.
- A `Socket` owns the OS stream and the public read buffer. A `Listener` owns the OS listening socket.

A user declaration named `Socket` or `Listener` keeps priority, as with `File` and `Process`.

### 2.3 Catalog operations

Notation (descriptive only, not syntax): `S` = `Socket`, `L` = `Listener`, `M` = the existing array length
bound `2^40`, `ok(s)` = `net_ok(s)`, `err(s)` = `net_error(s)`, `open(s)` = `net_write_open(s)`, `eof(s)` =
`net_eof(s)`, `size(s)` = `net_size(s)`, `lok(l)` = `net_listener_ok(l)`, `port(l)` =
`net_listener_port(l)`.

Parameter observers in contracts mean the value **at call entry**.

#### Listener

| Operation | Effect | Contract / meaning |
|---|---|---|
| `net_listen(host: str, port: i64) -> Listener` | `net, alloc` | requires `0 <= port && port <= 65535` (0 = OS-chosen ephemeral port). Success: `lok(result) && 1 <= port(result) && port(result) <= 65535`, the port actually bound. Failure: error 1 or 2 and `port(result) == 0` |
| `net_accept(l: Listener) -> Socket` | `net, alloc` | **borrows** the listener; its observable state never changes. requires `lok(l)`. Success: `ok(result) && open(result)`, empty buffer, EOF false. Failure: error 5, `!open(result)`, empty buffer, EOF false. Blocks until a connection arrives |
| `net_listener_close(take l: Listener) -> Result<i64>` | `net, alloc` | no requires. Closes and releases the listener. Entry error `e != 0` gives `Err(e)`, otherwise `Ok(0)` |

`net_listen` resolves `(host, port)` in OS order and binds the first address that succeeds. The backlog is the
OS default. 
#### Connect and write

| Operation | Effect | Contract / meaning |
|---|---|---|
| `net_connect(host: str, port: i64) -> Socket` | `net, alloc` | requires `1 <= port && port <= 65535`. Tries each resolved address in order. Success: `ok(result) && open(result)`, empty buffer, EOF false. Failure: error 1 (empty host or NUL in host) or 2 (resolution failed, refused, unreachable), `!open(result)`, empty buffer, EOF false |
| `net_write_str(take s: Socket, text: str) -> Socket` | `net, alloc` | requires `ok(s) && open(s)`; write_all of the UTF-8 bytes; on success buffer, EOF and write-open are preserved; failure is error 4 |
| `net_write_bytes(take s: Socket, data: [u8]) -> Socket` | `net, alloc` | same requires and contract; write_all of all bytes |
| `net_shutdown_write(take s: Socket) -> Socket` | `net, alloc` | requires `ok(s) && open(s)`; ensures `!open(result)`; on success buffer and EOF are preserved; failure is error 4 |
| `net_set_timeout(take s: Socket, ms: i64) -> Socket` | `net, alloc` | requires `ok(s)` and `0 <= ms && ms <= 86400000`; 0 = no timeout (the default). Sets both the read and the write timeout. On success all observed state is preserved |

`net_shutdown_write` is a half-close: the peer sees EOF, and reading on this socket continues. A write
timeout set with `net_set_timeout` makes a later write fail with error 6. Writes after a failure or after
`net_shutdown_write` cannot meet their requires.

#### Read

| Operation | Effect | Contract / meaning |
|---|---|---|
| `net_read(take s: Socket, max: i64) -> Socket` | `net, alloc` | requires `ok(s)`, `0 < max && max <= M`; puts the next chunk in the public buffer |
| `net_read_all(take s: Socket) -> Socket` | `net, alloc` | requires `ok(s)`. Reads to EOF |
| `net_bytes(s: Socket) -> [u8]` | `alloc` | independent copy of the public buffer. ensures `len(result) == size(s)` and every element equals the matching `net_byte` |

`net_read` success contract:

1. `ok(result)`.
2. `0 <= size(result) && size(result) <= max`.
3. `eof(result) == (size(result) == 0)`.
4. The write-open state is preserved.
5. Reading a stream already at EOF gives an empty buffer and EOF true.
6. The chunk with the last bytes may have EOF false; the next, empty read gives EOF true.
7. It blocks until bytes, EOF, an error or a timeout arrive. "No bytes yet" is never reported as EOF.
8. Failure is error 3, or error 6 when a timeout set by `net_set_timeout` expired.

`net_read_all` success contract:

1. `ok(result)`; EOF is true; the write-open state is preserved.
2. The buffer holds the bytes **not yet delivered by earlier reads**. Earlier buffers are not prepended.

`net_read_all` does not require the write side to be closed. A peer that waits for our EOF before it sends or
closes can therefore block `net_read_all` forever. Programs that talk to such a peer call
`net_shutdown_write` first (example below). This is documented, not solved.

`net_bytes` returns raw bytes, never `String`; UTF-8 validation and framing belong to user code.

#### Terminal

| Operation | Effect | Contract / meaning |
|---|---|---|
| `net_close(take s: Socket) -> Result<i64>` | `net, alloc` | no requires. Shuts down both directions (ignoring shutdown errors) and releases the socket. Entry error `e != 0` gives `Err(e)`, otherwise `Ok(0)` |

OS errors from the final close are not observable through `std` and are not reported. Terminal operations do
not return the public buffer; copy it first with `net_bytes`.

#### Observers

| Operation | Effect | Contract / meaning |
|---|---|---|
| `net_ok(s: Socket) -> bool` | none | `net_error(s) == 0` |
| `net_error(s: Socket) -> i64` | none | `0 <= result && result <= 6`; the first published failure code |
| `net_write_open(s: Socket) -> bool` | none | writes are allowed; true implies `net_ok(s)` |
| `net_eof(s: Socket) -> bool` | none | EOF seen by the last explicit read |
| `net_size(s: Socket) -> i64` | none | ensures `0 <= result && result <= M` |
| `net_byte(s: Socket, i: i64) -> i64` | none | requires `0 <= i && i < net_size(s)`; ensures `0 <= result && result <= 255` |
| `net_listener_ok(l: Listener) -> bool` | none | `net_listener_error(l) == 0` |
| `net_listener_error(l: Listener) -> i64` | none | `0 <= result && result <= 2` |
| `net_listener_port(l: Listener) -> i64` | none | ok implies `1 <= result && result <= 65535`; failed implies `result == 0` |

Observers are allowed in code and contracts. **Observers never query the OS.** The same Pruv value always
gives the same observer results, whatever the peer does in the meantime. Data becomes public only at the next
explicit effect operation.

### 2.4 Failures and error codes

These codes live only in `net_error`, `net_listener_error` and in `Err` of the terminal `Result<i64>`. They
are not compiler diagnostic codes.

Socket:

| Value | Meaning |
|---|---|
| `0` | no published failure |
| `1` | bad input: empty host, NUL in host |
| `2` | connect failed: resolution failed, refused, unreachable |
| `3` | read failed, including a connection reset |
| `4` | write or shutdown failed, including a broken pipe |
| `5` | accept failed |
| `6` | timed out (a read or write wait exceeded the timeout) |

Listener:

| Value | Meaning |
|---|---|
| `0` | no published failure |
| `1` | bad input: empty host, NUL in host |
| `2` | bind or listen failed (address in use, no permission, resolution failed) |

Rules (as in proc §2.4):

1. A failed `net_connect`, `net_listen` or `net_accept` returns an empty failed handle (the `file_ok`
   pattern). `Result<Socket>` stays disallowed.
2. The first published error is kept. Later cleanup errors do not overwrite it.
3. When an update fails: the write side is closed, the public buffer is emptied, EOF becomes false. The
   socket is still owned and must be closed.
4. So after a failure, ordinary read/write/shutdown/timeout cannot meet their requires. `net_close` and
   `net_listener_close` are always allowed and return that error as `Err`.
5. A write can fail after a partial write. `Err` does not mean "nothing was sent". No automatic retry.
6. If a read fails midway, that call's partial data is not published. Earlier `[u8]` copies are unaffected.
7. A reset peer or broken pipe never aborts the process (no `SIGPIPE`): reads fail with 3, writes with 4.
8. `Ok(0)` from a terminal operation only means the handle was consumed without a published entry error.
   Unlike `proc_wait` there is no exit status; the payload is always 0.
9. OOM and contract-violation aborts are distinct from I/O errors and keep the existing policy.

### 2.5 Blocking

Every operation blocks the calling thread. There are no internal threads, unlike `proc`.

- Protocol deadlocks (both sides waiting to read, a peer that never writes) are not solved.
- `net_set_timeout` bounds a single read or write wait, not a whole operation: a slow peer that trickles one
  byte per interval can keep `net_read_all` running for longer than the timeout.
- `net_accept` and `net_connect` have no timeout in this slice.
- `TCP_NODELAY` is set on connected and accepted sockets, because request/response latency matters more than
  small-packet coalescing. This is a runtime choice, not a contract.
- TCP is a byte stream. Message boundaries are not preserved: one write may arrive as several reads and
  several writes as one read. Framing (length prefixes, delimiters) belongs to user code.

### 2.6 Examples

These compile, verify and run once A1–A3 land.

```pruv
pub fn fetch(host: str, port: i64, request: [u8]) -> i64 ! net, alloc
{
  let mut s: Socket = net_connect(host, port);
  if !net_ok(s) {
    let r: Result<i64> = net_close(move s);
    return -1;
  }

  s = net_write_bytes(move s, request);
  if !net_ok(s) { let r: Result<i64> = net_close(move s); return -1; }
  s = net_shutdown_write(move s);
  if !net_ok(s) { let r: Result<i64> = net_close(move s); return -1; }
  s = net_read_all(move s);
  if !net_ok(s) { let r: Result<i64> = net_close(move s); return -1; }

  let reply: [u8] = net_bytes(s);
  let n: i64 = len(reply);
  let r: Result<i64> = net_close(move s);
  return n;
}
```

Every failure path closes the socket; the half-close tells the peer the request is complete. A bounded server
helper takes a finite budget of connections, because Pruv loops need `decreases`:

```pruv
fn serve(take input: Listener, rounds: i64) -> Listener ! net, alloc
  requires net_listener_ok(input)
  requires 0 <= rounds && rounds <= 1000000
{
  let mut l: Listener = move input;
  let mut remaining: i64 = rounds;

  while remaining > 0 && net_listener_ok(l)
    invariant 0 <= remaining && remaining <= rounds
    invariant net_listener_ok(l)
    decreases remaining
  {
    let mut s: Socket = net_accept(l);
    if net_ok(s) {
      s = net_set_timeout(move s, 5000);
      if net_ok(s) {
        s = net_read(move s, 4096);
      }
      if net_ok(s) && net_write_open(s) {
        s = net_write_str(move s, "ok\n");
      }
    }
    let r: Result<i64> = net_close(move s);
    remaining = remaining - 1;
  }

  return move l;
}
```

Each round accepts, sets a 5000 ms timeout, reads one chunk, writes a reply and closes the socket. `net_accept`
borrows the listener, so `l` stays valid across rounds, and the loop keeps `net_listener_ok(l)` as an
invariant. The helper hands the ownership duty back: the caller ends it with `net_listener_close(move l)`.
`decreases` does not prove that `net_accept` returns, and a loop budget is not a wall-clock timeout.

## 3. Static rules

### 3.1 Effect and names

1. Add `net` to both parsers and the AST effect description.
2. Catalog effect sets feed the existing `used_effects` check.
3. The unknown-effect hint becomes `io, alloc, fs, proc, net`.
4. `Socket` and `Listener` normalize to their internal keys like `File` and `Process`. A user declaration
   with the same name keeps priority, as today.
5. Generated Rust uses `pruv_Socket` and `pruv_Listener`, never a user type name.

### 3.2 Reused ownership rules

The rules of proc §3.2 apply to both new types unchanged:

- Use after move, double consume, owned alias: `type/foreign_move`.
- Consuming or returning a borrowed handle: `type/foreign_borrow` (`net_accept` only borrows its listener).
- A loop consumes an outer owner and does not restore it: `type/foreign_loop_move`.
- Storage inside struct, enum, array, Option, Result or a collection: `type/foreign_owned_type`.
- A handle in a host entry signature: `type/entry_signature` for a `pub fn`, `type/foreign_export` for
  `fn main`.
- A trusted Rust declaration that creates or consumes a Socket or Listener around the catalog:
  `type/foreign_signature`.

No new public host ABI. Internal functions may return `Socket` and `Listener`.

### 3.3 Must-consume for `Process`, `Socket` and `Listener`

The proc rule is generalized to the set {`Process`, `Socket`, `Listener`} in the **same** analyzer: Python
`_Ownership` (`pruvc/foreign.py`) and Rust `Ownership` (`C/foreign.rs`), plus the temporary check in
`pruvc/check.py` and `C/check.rs`. Do not write a second analyzer. All ten proc rules apply to the new types:

1. **Block end:** a handle leaving scope while `live` or `maybe` is rejected.
2. **return:** consume the return expression first, then every owned handle leaving the function must be
   dead. `return move s` transfers the duty.
3. **Fallthrough:** live `take` parameters and locals are checked the same way.
4. **Overwrite:** after evaluating and consuming the RHS, a destination still holding a live/maybe handle is
   rejected. `s = net_read(move s, 4096)` is allowed; overwriting a live `s` with a new `net_connect` is not.
5. **Temporaries:** an unnamed handle result also carries the duty. `net_close(net_connect(...))` is allowed
   (taken at once). `net_ok(net_connect(...))` (borrowed then dropped) is rejected, and so is an update
   result that is never used. `net_accept(l)` as a temporary is rejected unless consumed at once.
6. **Branches:** consumed in one branch only gives maybe at the join, rejected at scope end and overwrite.
   Consumed in both branches is allowed.
7. **Loops:** the outer-owner restoration rule stays. A handle created inside a loop body (such as the
   accepted socket in `serve`) must be consumed or moved out before that iteration's scope ends.
8. **Helpers:** a `take` helper must terminate the handle, pass it to another take helper, or return it.
9. **Failed handles:** a failed connect, listen or accept is not exempt, so the checker never needs to know
   whether the OS call succeeded.
10. **Modes:** `--no-verify`, `--trust-callers` and `--foreign-checks off` cannot turn this rule off.

The rule guarantees exactly-once termination or transfer on normal control flow. It does not prove eventual
cleanup across infinite blocking or forced aborts. The language has no user `break`/`continue`, and this
slice does not add them.

### 3.4 `par` and contracts

- Add `net` to the `par_info` forbidden effects. Direct and indirect `net` calls are rejected with
  `type/parallel_effect`.
- Define a `NET_OBSERVERS` list. Calling connect/listen/accept/read/write/shutdown/timeout/close or the
  allocating `net_bytes` in a contract is `type/foreign_contract`.
- Observer index requires (`net_byte`) apply to the domain checks of both code and contracts.

### 3.5 Drop

`Socket` and `Listener` never end normally through auto-drop. Dropping a `TcpStream` or `TcpListener` closes
the descriptor (unwinding, internal bugs) but does not satisfy must-consume. Other drop policies are unchanged.

## 4. Verification encoding

1. **Socket value.** A symbolic record with the public state only:

   ```text
   error: Int
   write_open: Bool
   eof: Bool
   buf: (length: Int, bytes: Array Int Int)
   ```

   `net_ok` is `error == 0`. The OS descriptor, peer, address and kernel buffers are not modeled.

   **Listener value.** `{error: Int, port: Int}`. `net_listener_ok` is `error == 0`.

2. **Well-formedness.**
   - Socket: `0 <= error <= 6`; buffer length in `0..M`; bytes at valid indexes in `0..255`;
     `write_open ==> error == 0`; `error != 0` implies `!write_open`, empty buffer and `!eof`.
   - Listener: `0 <= error <= 2`; `error == 0 ==> 1 <= port <= 65535`; `error != 0 ==> port == 0`.

   EOF true and an empty buffer are not made globally equivalent: `net_read_all` can return EOF true with a
   non-empty final buffer.

3. **Effect calls.** Same order as `File` and `Process`: catalog requires → fresh result → well-formedness →
   ensures. Results are never memoized. Two identical `net_connect` calls, or two `net_accept` calls on the
   same listener, are independent external results and are never treated as equal because their inputs are
   equal.

4. **Preservation.** A successful write, shutdown or timeout preserves the buffer snapshot and EOF; write and
   timeout also preserve write-open. A successful read preserves write-open. Encode buffer preservation as
   equal length and equal byte map. Write the success/failure cases exactly as the §2 transitions; do not
   drop preservation facts because the result is fresh.

5. **Borrowed listener.** `net_accept` reads the listener and does not havoc it: after the call the same
   listener value, with the same `ok` and `port`, is still known.

6. **Observers and copies.** Observers are field reads or array selects. `net_bytes` is a new array with the
   snapshot's length and byte map; at runtime its memory is independent.

7. **Terminal results.** Use the existing `Result<i64>` encoding.
   - With an entry error, `result == Err(error)`.
   - Otherwise `result` is exactly `Ok(0)` for `net_close` and `net_listener_close`. Unlike proc, the Ok
     payload is 0 because there is no exit status.
   - On Err, `1 <= err_val(result)`, at most 6 for sockets and at most 2 for listeners.

8. **Entry, return, havoc.** Function parameters, user function returns, loop havoc, counterexample output
   and type recovery handle both types. After havoc only well-formedness and proven invariants remain;
   earlier `ok`, sizes, ports and EOFs are not kept.

9. **Domains.** Register `net_byte` for contract-domain checks like the File byte observer. A postcondition
   cannot become true through `net_byte` with an unchecked index.

10. **Ownership and termination.** Must-consume is static and is not re-encoded in SMT. No axioms about
    peers, data arrival, connection success, time or total memory.

11. **Inference.** `--infer` guesses no OS-state invariants or termination measures; it only gains the
    branches needed so the new types are not treated as scalars.

## 5. Rust source backend

### 5.1 Representation

```rust
struct pruv_Socket {
    stream: Option<std::net::TcpStream>,
    // public state, changed only by explicit operations
    error: i64,
    write_open: bool,
    eof: bool,
    buf: Vec<u8>,
}

struct pruv_Listener {
    listener: Option<std::net::TcpListener>,
    error: i64,
    port: i64,
}
```

There are no worker threads and no queues.

### 5.2 Connect, listen, accept

1. Check an empty host and NUL in host before any OS call; failure is error 1 (socket and listener).
2. `net_connect` resolves `(host, port)` with `ToSocketAddrs` and tries each address in order with
   `TcpStream::connect`. If every address fails, or resolution fails, the error is 2. Literal IPv4 and IPv6
   addresses are accepted by the same path.
3. `net_listen` resolves `(host, port)` and binds the first address that succeeds, using
   `TcpListener::bind`. The port is read back with `local_addr()`, so port 0 reports the real bound port. A
   failed bind is listener error 2.
4. `net_accept` calls `accept` on the borrowed listener. An error, including a retry-exhausted interruption,
   is socket error 5. The accepted stream gets `TCP_NODELAY`, no timeouts, an empty buffer, EOF false.

### 5.3 Reading, writing, timeouts

- A read makes one `read` call into a buffer of `max` bytes and retries on `Interrupted`. The bytes read
  become the public buffer. Zero bytes read is EOF.
- `WouldBlock` or `TimedOut` from a socket timeout is error 6. Any other read error, including a reset, is
  error 3.
- `net_read_all` loops over reads until EOF, appending to the buffer; the same error mapping applies.
- Writes use `write_all`. A write error is error 4, a write timeout is error 6, and a broken pipe is error 4
  and never aborts the process. Rust `std` already suppresses `SIGPIPE` on sockets, so the runtime does not
  install signal handlers.
- `net_shutdown_write` calls `shutdown(Shutdown::Write)`; failure is error 4.
- `net_set_timeout` sets `set_read_timeout` and `set_write_timeout` to `None` for 0, otherwise to the given
  milliseconds. Error 6 is published only when a later wait actually times out. If the OS refuses to set
  the timeout, the call fails with error 4.
- `net_close` shuts down both directions, ignoring shutdown errors, then drops the stream. A second close on
  the same handle is a requires violation of the static rules, not a runtime state.

### 5.4 Runtime contract checks

Follows the existing `--foreign-checks requires|all|off`.

- An enabled requires violation uses the existing `pruv: foreign requires violated:` prefix and exit 101.
- Under `--no-verify`, the default foreign requires checks still catch writes after shutdown, reads on a
  failed socket, and bad port/max/index/timeout values.
- With checks off, a bad index or handle still cannot cause UB; the existing representation defenses stay.
- OS failures never use this panic path.

### 5.5 Test-only fault hooks

Failures the OS cannot produce reliably (a failing accept, a mid-stream read error, a failing write or
shutdown) are tested through a compile-time environment variable `PRUV_NET_TEST_HOOK` with the values
`accept`, `read`, `write`, `shutdown`, like `PRUV_PROC_TEST_HOOK`. It is read when the generated runtime is
built for a test, forces the named operation to fail with its error value, and does not exist in normal
builds. It is not part of the user API.

## 6. LLVM backend

### 6.1 Same Rust shim as File and Process

1. LLVM holds `Socket` and `Listener` as opaque pointers.
2. The generated Rust bindings' G6 shim contains **the same runtime/helper code** as §5.
3. Catalog adapters evaluate arguments once, in order.
4. A take adapter unboxes its owned box exactly once.
5. Updates return a new owned box. `net_close` and `net_listener_close` do not create a new box.
6. `net_accept` takes a **borrowed** listener pointer, does not unbox it, and returns a new `Socket` box.
7. A moved LLVM slot becomes null so scope cleanup does not consume it again.
8. The drop adapters are defensive only; a live Socket or Listener drop in a normal program is rejected by
   §3.
9. Observers that appear only in contracts are still collected for the shim.

### 6.2 Adapter ABI (shim-internal, not host ABI)

| Kind | Shape |
|---|---|
| listen, connect | host `(ptr,len)`, port `i64` → Listener / Socket pointer |
| accept | borrowed Listener pointer → new Socket pointer |
| update | Socket pointer plus scalar/slice arguments → Socket pointer |
| observer | borrowed pointer plus scalars → `i64`; bool is 0/1 |
| close, listener close | pointer, `tag_out: ptr`, `value_out: ptr` → void |
| bytes snapshot | borrowed Socket pointer → borrowed `(data,len)` view through out parameters |

Terminal shim tags are fixed: `1 = Ok`, `0 = Err`. LLVM assembles the existing internal Result from them.
Host Result returns keep the existing target ABI lowering.

`net_bytes` copies the borrowed view at once with the existing `pruv_copy_v2(..., size=1, align=1)`; the
handle is not updated or consumed in between. Rust-allocated buffers are never passed to C `free`. Copying a
whole buffer never calls the shim once per byte.

### 6.3 `runtime.c`

**No new functions in `runtime.c`.** Only the existing v2 byte copy, allocation, free and representation
checks are used. No C socket, `getaddrinfo` or `poll` code. Platform I/O for both backends lives in the shared
Rust runtime. The C runtime ABI and allocator markers do not change. A standalone socket runtime without the
Rust shim is later work and needs its own C runtime design.

### 6.4 Build limits

- Run with `--backend llvm --rust-bindings PATH` and a Rust host build.
- Native `--emit exe|obj` for programs that need the shim keeps `codegen/foreign_target`.
- A program that only declares `! net` without using the catalog does not get the shim limit.

## 7. Diagnostics and compatibility

### 7.1 New diagnostic

| Stage / code | Condition | Hint |
|---|---|---|
| `type/net_unconsumed` | a Socket or Listener duty is left at scope end, return, overwrite or a discarded temporary | consume it with `net_close(move s)` or `net_listener_close(move l)`, or return / transfer ownership |

Process keeps `type/proc_unconsumed`. The two diagnostics share one analyzer; the code follows the type of the
handle that loses its duty. Locations follow proc §7.1 (return, assignment, temporary expression, binding).

### 7.2 Reused diagnostics

| Condition | Diagnostic |
|---|---|
| missing `net` or `alloc` | `effect/missing_effect` |
| unknown effect | existing parse diagnostic, longer hint list |
| double close, use after move | `type/foreign_move` |
| consuming a borrowed Socket or Listener | `type/foreign_borrow` |
| loop does not restore an outer owner | `type/foreign_loop_move` |
| bad placement of an owned type | `type/foreign_owned_type` |
| handle in a host entry | `type/entry_signature` (`pub fn`), `type/foreign_export` (`fn main`) |
| trusted declaration bypassing signatures | `type/foreign_signature` |
| effect operation in a contract | `type/foreign_contract` |
| `net` on a `par` path | `type/parallel_effect` |
| unproved catalog requires | `verify/call_requires` |
| contract observer domain not met | existing `verify/contract_domain` |
| staging guard for unimplemented batches | `codegen/unsupported_type` |
| native output without the shim | `codegen/foreign_target` |

A0 searches the repository once for name clashes of `net_unconsumed`. Runtime error values 1–6 are read only inside `net_error`, `net_listener_error` and the net terminal Results, so they
do not clash with compiler diagnostics or OS exit codes.

### 7.3 Compatibility

- Programs that do not use the new features keep their ownership and drop meaning and generate identical
  Rust, LLVM and bindings.
- The unknown-effect hint change is an intended diagnostic change.
- An existing user struct named `Socket` or `Listener` keeps its meaning.
- `File` and `Process` are not redesigned. The generalized must-consume rule must leave every existing
  `Process` diagnostic (code, location, message) unchanged.

## 8. Batches and gates

Each batch finishes the Python implementation and the Rust port together. New frontend, proof and runtime
rows must agree between the compilers. Generated text must be byte-identical for the same target and
options. Network timing and chunk boundaries are not compared; reassembled bytes are. All runtime tests use
127.0.0.1 only. During a batch only that batch's tests run; the full suite and row comparison follow the
repository commit procedure.

### A0. Effect, types, must-consume

Files: `pruvc/vparser.py`, `pruvc/vast.py`, `pruvc/typeinfo.py`, `pruvc/check.py`, `pruvc/foreign.py`,
`pruvc/pruvc.py`, `C/parser.rs`, `C/typeinfo.rs`, `C/gtype.rs`, `C/check.rs`, `C/foreign.rs`,
`D/driver.rs`, `tests/run_all.py`, `tests/run_all_rows.txt`, new `tests/g10_net/frontend/*.pruv`, new
`tests/must_reject/g10n_*.pruv` with `.json`, this document.

Work: the `net` effect, the `Socket` and `Listener` types, catalog signatures and effect registration; the
must-consume generalization over {Process, Socket, Listener} in the existing analyzer, with the new
`type/net_unconsumed`; the `par` effect; a staging guard (`codegen/unsupported_type`) for unimplemented
parts; the diagnostic name clash search for `net_unconsumed`.

Gates:

1. Moves, borrows, helper returns, a borrowed listener passed to `net_accept`, and the `fetch` and `serve`
   shapes type-check.
2. Each of these is rejected with its diagnostic: double close, use after move, consuming a borrow;
   consumed in one branch only, missing on an early return, a forgotten take parameter; overwriting a live
   owner, discarding a borrowed temporary, discarding an update result; a loop local (an accepted socket)
   left unconsumed, an outer owner not restored; placement in Result/struct/collection, host entry
   exposure; a missing `net` effect, an indirect `par` effect; a listener closed with `net_close` or a
   socket closed with `net_listener_close` (type mismatch).
3. Consumed in both branches, `s = update(move s)` and `return move s` are allowed.
4. Existing File auto-drop cases and every proc case (including `type/proc_unconsumed`) are unchanged.
5. Programs using Socket or Listener stop at the staging guard in every mode. A function with only `! net` is
   not guarded.
6. Frontend diagnostics (stage/code/location) and output agree between the compilers.

The A0 catalog contract placeholders can never run past the staging guard or be reported as proved.

### A1. Verification

Files: `pruvc/foreign.py`, `pruvc/verify.py`, `pruvc/pruvc.py`, `C/foreign.rs`, `P/symbolic.rs`,
`P/obligation.rs`, `D/driver.rs`, `tests/run_all.py`, `tests/run_all_rows.txt`, new
`tests/g10_net/proof/*.pruv`, new `tests/must_reject/g10n_v_*.pruv` with `.json`.

Work: the §4 encoding and the full catalog contracts; observer domains, call-entry snapshots, havoc and
counterexamples; move the guard to the codegen entry, after verification.

Gates:

1. write, read and shutdown after a confirmed connect or accept verify.
2. Paths that close a failed handle verify.
3. Operations on an unconfirmed socket, a write after shutdown, and a bad max, port, index or timeout are
   rejected.
4. Preservation of buffer and EOF on write, shutdown and timeout, and the exact copy relation of
   `net_bytes`, verify.
5. `net_read_all` with EOF true and a non-empty buffer is consistent.
6. Claims that use an earlier `ok`, size, EOF or port after loop havoc without an invariant are rejected.
7. Claims that two identical connects or accepts give equal results, and claims about the content a peer
   sends, are rejected.
8. `net_close` and `net_listener_close` returning `Ok(0)` when there is no entry error is provable; an
   unconditional `Ok` is not.
9. Wrong observer domains in contracts are rejected.
10. A finite accept-budget server (`serve`) verifies. A loop claiming to decrease by a peer event alone does
    not pass.
11. New obligations, conclusions and diagnostics agree between py and rs.

### A2. Rust runtime and source generation

Files: `pruvc/codegen.py`, `pruvc/foreign_codegen.py`, `pruvc/foreign.py`, `C/codegen.rs`,
`C/foreign_codegen.rs`, `C/foreign.rs`, `pruvc/pruvc.py`, `D/driver.rs`, `tests/run_all.py`,
`tests/run_all_rows.txt`, new `tests/runtime/g10n_r*.pruv`, `.rs`, `.json`.

Work: the §5 runtime and helpers, the snapshot copy, the test hook; remove the Rust backend guard (the LLVM
guard stays). The `.rs` test driver is the Rust host main. It starts its own loopback peer threads with
`std::net` (an echo server, a silent peer for timeouts, a closing peer for broken pipe, a large sender, a
client that connects to the Pruv listener) and passes the port to the Pruv `pub fn`. There is no external
fixture program.

Gates:

1. Exact bytes in both directions across chunk boundaries, including a large transfer.
2. Half-close: the peer sees EOF after `net_shutdown_write`, and reading continues.
3. Connect to the port of a closed listener is error 2; empty host and NUL in host are error 1.
4. Listening on a port already in use is listener error 2; an ephemeral port is reported and usable.
5. `net_read_all` does not repeat chunks already delivered.
6. A read against a silent peer with a timeout is error 6.
7. A write to a closed peer is error 4 and the program does not abort.
8. A reset peer is error 3 (through the hook if the reset is not portable).
9. An accept failure is error 5 through the hook; read, write and shutdown hooks give 3, 4, 4.
10. Termination after a failed handle or an I/O failure releases the descriptor; a listener port can be
    reused after `net_listener_close`.
11. Runtime requires violations exit 101 and are distinct from OS failures.
12. Generated Rust is identical between py and rs.

### A3. LLVM shim, Windows, user spec

Files: `pruvc/llvmgen.py`, `pruvc/foreign_shim.py`, `C/llvmgen.rs`, `C/foreign_shim.rs`, `D/driver.rs`,
`tests/run_all.py`, `tests/run_all_rows.txt`, `tests/runtime/g10n_r*.pruv`, `.rs`, `.json`, new
`tests/g10_net/windows.py`, `SPEC.md`, this document. `pruvc/runtime.c` does not change.

Work: Socket and Listener adapters, the borrowed-listener accept adapter, the Result out-parameter adapter,
borrowed bytes view plus v2 copy; remove the LLVM guard; at most one page of net user rules in `SPEC.md`.

Gates:

1. The A2 runtime cases pass on LLVM too.
2. `Result` Ok/Err and payloads are never swapped.
3. G6 live boxes are 0 after updates, branches, helper returns, accept loops, close and listener close.
4. A `net_bytes` copy stays valid after the socket is updated or closed.
5. Observers that appear only in contracts are linked.
6. Generated IR and bindings are identical between py and rs.
7. The native output limit is `foreign_target` and does not conflict with the existing File and Process
   limits.
8. On Windows, `tests/g10_net/windows.py` runs the loopback cases for real: connect, accept, both
   directions, half-close, timeout, refused connect and an ephemeral port. Binding 127.0.0.1 does not raise
   the Windows Defender firewall prompt.

Building a Windows artifact is not a Windows run. Without a run environment, gate 8 stays pending, recorded as such.

### Q. Quality

Files: new `bench/g10_net/PROBLEMS.md`, `bench/g10_net/solutions/q01.pruv`, `q02.pruv`,
`bench/g10_net/run.py`, `mutate.py`, `perf.py`, `bench/g10_net/faults/`, `bench/g10_net/results/`,
`RESULTS.md`, `BLUEPRINT.md`, this document.

Problems:

1. A length-prefixed request/response client against a harness server: exact framing, partial reads, and
   every failure path closes the socket.
2. A bounded line server: listen on port 0, accept a budget of connections, per connection set a timeout,
   read until LF across chunks, reply, close, and finally close the listener. Byte-based line assembly; no
   UTF-8 conversion assumed.

Gates:

1. Both reference solutions pass on all four py/rs × rust/llvm combinations.
2. An independent author solves both from `SPEC.md` and the problems only; first result and fix count are
   recorded.
3. 16 explicit wrong mutants: missing close, overwrite, branch, temporary, unchecked I/O, write after
   shutdown, range errors, EOF misreading, `net_read_all` duplication, framing errors.
4. Static and contract mutants are caught at compile time; wrong handling of external data is caught by the
   harness oracle. The two are reported separately; no claim that all external behavior is proved.
5. Four compiler faults are caught by named tests: removing Socket must-consume, removing a catalog requires,
   a wrong write preservation, an inverted Result tag.
6. One fixed workload moves 1 MiB over loopback in 4 KiB chunks with zero loss and duplication, and reports
   time and memory against hand-written Rust.
7. No performance target is declared without evidence. Per-byte shim cost and bulk copy cost are recorded
   separately.
8. Results and limits go into `RESULTS.md`; the net status in `BLUEPRINT.md` is updated. Existing File and
   proc measurements are not replaced.

## 9. Later work

- User-facing async, poll, multiplexing and cancellation.
- Connect and accept timeouts.
- TLS.
- UDP.
- HTTP clients and servers, and verified framing or protocol libraries.
- Unix domain sockets.
- A DNS API (explicit resolution, address lists).
- Socket options beyond the timeout (keep-alive, buffer sizes, `TCP_NODELAY` control).
- Peer and local address observers.
- Sharing a listener across `par` branches.
- A C socket runtime for LLVM without the Rust shim.
