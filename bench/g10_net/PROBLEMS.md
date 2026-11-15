# G10 net problems

Use default verification. Each project has `qNN/root.pruv`. The grader fixes the public
headers below; other helpers are permitted. Hosts are `str` values and ports `i64`; no
`Socket` or `Listener` crosses the host boundary. The grader's Rust driver hosts every peer
itself, in threads, on `127.0.0.1` only (no child process). What the peers send is checked at
run time, not proved. Every peer operation has a 30 s timeout and the driver only uses bounded
waits, so a wrong solution cannot hang the grader.

## q01 — a length-prefixed request/response client

```
pub fn call(host: str, port: i64, request: [u8]) -> i64 ! net, alloc
  requires 1 <= port && port <= 65535
  requires len(request) <= 1000000
  ensures -5 <= result && result < 1000001000000
```

Connect to `(host, port)`, set a 2000 ms timeout, then write the request frame: a 4-byte
big-endian length of `request` followed by the request bytes (one write for the header and one
for the body, or one combined write; either is fine). Then read the reply frame: a 4-byte
big-endian length `L`, then exactly `L` payload bytes.

Read with `net_read` in chunks of at most 4096 bytes and never more than the bytes still needed
for the current header or payload, so bytes after the frame are never read. Never use
`net_read_all`. The header may arrive split across reads.

Let `h` start at 1 and, for every payload byte `b` in order, become `(h * 256 + b) % 999983`.
On success close the socket and return `L * 1000000 + h`.

Errors, first match wins; every path closes the socket and ignores the close result:

- connect failed: `-1`
- setting the timeout or a write failed: `-2`
- a read failed (including a timeout): `-3`
- EOF (an empty chunk) before the frame is complete: `-4`
- declared `L > 1000000`: `-5` (decided as soon as the header is complete)

```python
def call(host, port, request):
    try:    s = connect(host, port)
    except: return -1
    s.settimeout(2.0)
    try:    s.sendall(len(request).to_bytes(4, 'big') + request)
    except: return close(s) or -2
    def need(n):                       # exactly n bytes, in chunks of <= 4096 and <= what is still needed
        data = b''
        while len(data) < n:
            chunk = s.recv(min(4096, n - len(data)))   # an exception here is -3
            if not chunk: raise EOFError               # -4
            data += chunk
        return data
    L = int.from_bytes(need(4), 'big')
    if L > 1000000: return close(s) or -5
    h = 1
    for b in need(L):
        h = (h * 256 + b) % 999983
    close(s)
    return L * 1000000 + h
```

Harness servers (Rust threads in the driver). Each accepts one connection, reads the 4-byte
header and exactly that many request bytes, and reports the request back to the driver, so the
request framing is checked too, then:

| Mode | Behaviour | Result |
|---|---|---|
| echo | replies with a frame whose payload is every request byte plus 1 mod 256; the header is written in two 2-byte pieces with a 30 ms sleep between, the payload in 1021-byte writes | `L * 10^6 + h` |
| extra | like echo, then writes 50 extra bytes and keeps the socket open about 200 ms (the client must ignore them) | `L * 10^6 + h` |
| short | header says 10, sends 5 bytes, closes | `-4` |
| noreply | closes right after reading the request | `-4` |
| huge | header 2000000, then closes | `-5` |
| silent | reads the request, sleeps 4 s without writing, then closes | `-3` (client timeout) |

Cases: echo with an empty request; `"a"`; all 256 byte values; 100000 bytes of
`(i*7+3) % 251`; 300000 bytes of `(i*13+5) % 256`; extra with `"hello"`; short; noreply; huge;
silent; a refused port (bind `127.0.0.1:0`, take the port, drop the listener) `-1`; empty host
`""` `-1`.

## q02 — a bounded line server

```
pub fn serve(host: str, ctl: i64, budget: i64) -> i64 ! net, alloc
  requires 1 <= ctl && ctl <= 65535
  requires 0 <= budget && budget <= 1000
  ensures -3 <= result && result <= 100010001000
```

Listen on `(host, 0)`. If listening failed: close the listener, return `-1`.

Connect to the control port `(host, ctl)`, write the bound port (`net_listener_port`) as 2 bytes
big-endian, close the control socket. If that connect or write failed: close the control socket
and the listener, return `-2`.

Then accept exactly `budget` connections, one after another. Per connection:

- if accept failed, count it as dropped
- otherwise set a 2000 ms timeout and read chunks of at most 4096 bytes and never more than is
  needed to hold 1025 line bytes (a line buffer `alloc(1025, 0)`), copying bytes into the buffer
  until the first LF (byte 10). Lines are bytes only: CR and every other byte are ordinary, no
  UTF-8.

Outcomes:

- an LF found after `n <= 1024` line bytes: reply the `n` line bytes in reverse order followed
  by LF (one `net_write_bytes`); count it as served
- 1025 bytes without an LF: reply the two bytes `!` LF; count it as long
- EOF before an LF, a read failure or timeout, a failed timeout set, or a failed write: count it
  as dropped (no reply)

Close every accepted socket. After the budget, close the listener; if that close returns `Err`,
return `-3`. Otherwise return `served * 10^8 + long * 10^4 + dropped`.

```python
def serve(host, ctl, budget):
    l = listen(host, 0)
    if l is None: return -1
    try:    c = connect(host, ctl); c.sendall(l.port.to_bytes(2, 'big')); c.close()
    except: return close_all() or -2
    served = long = dropped = 0
    for _ in range(budget):
        s = accept(l)
        if s is None: dropped += 1; continue
        line = read_until_lf_or_1025(s, timeout=2.0)       # bytes up to the first LF, or None on failure/EOF
        if line is FAILED:       dropped += 1
        elif line is TOO_LONG:   write(s, b'!\n') and (long := long + 1) or (dropped := dropped + 1)
        else:                    write(s, line[::-1] + b'\n') and (served := served + 1) or (dropped := dropped + 1)
        s.close()
    return -3 if close(l) is Err else served * 10**8 + long * 10**4 + dropped
```

Harness (driver): a control listener on `127.0.0.1:0`; a thread accepts the control connection,
reads the 2 bytes (the port), then runs the clients one after another against `127.0.0.1:port`
and collects their replies; the main thread calls `serve("127.0.0.1", ctl_port, budget)`. Each
client reads the reply to EOF with a 10 s timeout and never sends bytes after its LF (unread
data at close would reset the connection and could destroy the reply).

| # | Client | Reply |
|---|---|---|
| 1 | `hello` LF | `olleh` LF |
| 2 | `ab`, sleep 50 ms, `cd` LF (a line across chunks) | `dcba` LF |
| 3 | LF | LF |
| 4 | bytes 0..=255 except 10, then LF | the reverse, then LF |
| 5 | 1024 bytes of `(i*5+1) % 251` (10 mapped to 11), sent in 7-byte pieces, then LF | the reverse, then LF |
| 6 | exactly 1025 such bytes, no LF, then waits for the reply | `!` LF |
| 7 | `no newline`, then shutdown(Write) | empty (dropped) |
| 8 | connects, sends nothing, reads to EOF (the server times out after 2 s) | empty (dropped) |
| 9 | connects and closes at once | none read (dropped) |
| 10 | `a` CR LF | CR `a` LF |

Expected for that run: `6 * 10^8 + 1 * 10^4 + 3 = 600010003`. Other cases: budget 0 `0` (the
control message still arrives); host `""` `-1` (the driver does not wait for a control
connection); a refused control port (a dropped free port) `-2`.
