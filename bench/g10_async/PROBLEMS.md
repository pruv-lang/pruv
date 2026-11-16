# G10 async Q problems

Use `SPEC.md` and the public Loop catalog. Programs must verify with default flags;
do not use `--no-verify`, trusted callers, host implementations, or compiler changes.
Every normal path consumes all owned handles exactly once, and ends its Loop with
`ev_close`. Network traffic is exclusively IPv4 loopback. See below for the concrete
protocol choices left open by G10_ASYNC_SPEC section 8 Q.

## 1. Multi-client line server

Implement:

```pruv
pub fn serve(ctl: i64, budget: i64) -> i64 ! async, net, proc, alloc
  requires 1 <= ctl && ctl <= 65535
  requires 0 <= budget && budget <= 8
```

Listen on `127.0.0.1:0`. Connect to `127.0.0.1:ctl`, write the assigned port
as exactly two unsigned big-endian bytes, then close the control socket. Serve
`budget` clients, with up to eight simultaneous clients, using one Loop (capacity
17: one listener, eight sockets, eight timers). Do not handle clients sequentially.
Zero budget still announces the port and closes the loop, returning zero.

Each client sends zero or more byte lines terminated by LF (10). LF may occur in
any chunk position; several lines may share a chunk. CR, NUL and bytes 128..255
are ordinary data. Assemble each line byte by byte in a separate slot-indexed
buffer. For each LF reply with exactly two unsigned big-endian bytes: the number
of bytes preceding that LF since the preceding LF or connection start. The LF is
excluded. Empty lines produce `00 00`. Inputs contain at most 1024 bytes per line;
the reference saturates at 1024 for longer input. Partial lines at EOF are discarded.
All complete lines queued before EOF must be answered before removing the socket.

Use one 400 ms one-shot idle timer per client. On every nonempty read remove the
previous timer and add a fresh timer; when its timer fires, drop that client.
Other clients must continue to work while one is silent. A client is finished when
it reaches EOF, encounters an error, or times out. Remove its remaining timer too.
Stop after the budget is finished and call `ev_close`. Return the number finished;
return -1 if listening fails, -2 if the control connect fails. For bounded failure
handling, stop after 100000 events or a 2000 ms wait with no event, returning the
number finished. The oracle supplies all budgeted clients promptly and stays
below the event limit. A failed Loop is closed and returns the number finished.

## 2. Child supervisor

Implement:

```pruv
pub fn supervise(program: str, ctl: i64, deadline_ms: i64) -> i64 ! async, net, proc, alloc
  requires 1 <= ctl && ctl <= 65535
  requires 1 <= deadline_ms && deadline_ms <= 86400000
```

Listen and announce the port using the same two-byte control protocol. Accept one
client, close the listener, and spawn `program` without arguments or a working
directory override. The fixture is an executable local child. Put its process,
the client socket and a deadline timer into one capacity-three Loop. The deadline
starts when the timer is added after the child. Drain stderr without forwarding
it, even when it exceeds the runtime's 1 MiB per-stream queue limit. Forward stdout
bytes in order, without loss, duplication or encoding conversion; child output is
LF-terminated byte lines, including lines split between child writes.

Any nonempty client input is a cancel request (the oracle sends `C`). On cancel,
stop the child and reply; on deadline, stop the child and reply; on child EOF,
wait/reap the child and reply with its exit status. Queued stdout must be drained
before publishing child exit. End with `ev_close` on all paths, which kills and
reaps a child still present. Cancel/deadline need not forward bytes still queued
when they occur. A terminal reply is a nine-byte suffix following forwarded stdout:
one tag byte, then eight unsigned big-endian bytes encoding `status + 2147483648`.
Tag 1 = child exit; 2 = deadline; 3 = peer EOF/error; 4 = cancellation. Status is
the child's signed 32-bit exit status for tag 1, zero otherwise. The reference
maps any status outside signed 32-bit range to zero. Readers identify the suffix
as the final nine bytes, not by searching stdout for a tag.

Return that tag on normal completion, -1 for network setup failure, -2 for spawn
failure, -3 for Loop setup failure, -5 for a recorded Loop/close failure. A
1000000-event safety bound is allowed; the oracle remains below it. The fixtures
write `one` LF `two` LF to stdout, 1200000 bytes to stderr, then either exit 7 or
sleep ten seconds. The oracle checks exit, cancel, and deadline separately.

## Verification boundary

Ownership, catalog preconditions and buffer safety are static obligations. Correct
framing, timer association, scheduling and interpretation of external bytes are
checked by an external oracle, not claimed as fully proved.
