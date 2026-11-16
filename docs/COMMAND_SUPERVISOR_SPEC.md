# Command supervisor port design

Status: proposed design (2026-11-16), track L3-2. Implementation belongs in `bench/l3/supervisor/`. It uses the
G10 `fs`, `proc`, `net` and `async` surfaces (SPEC §6h, §6l, §6m, §6n) and needs **no language changes**.

## Goal

Port the core of Lair's command supervisor (`src/backend/command-supervisor.ts` in the Lair repository, 363 lines)
to verified Pruv: run one shell with arguments, merge its stdout and stderr, write the first `fileLimit` bytes to
an output file, keep the last `tailChars` UTF-16 units of the decoded output, react to a stop request or a lost
controller, and report one result packet. The host keeps the parts Pruv has no surface for (process groups, pid,
environment, file mode, rename, JSON IPC).

The reference is the TS file at the commit the Q differential pins, run on Node with stub modules for its two
non-`node:` imports. Lair integration is not an acceptance requirement.

## Decisions

### Shape

One host entry:

```
pub fn supervise(shell: str, argv: str, cwd: str, out: str,
                 file_limit: i64, tail_chars: i64, ctl: i64) -> [i64] ! async, net, proc, fs, alloc
```

- `argv` is every argument followed by one NUL byte (`""` = no arguments, `"\0"` = one empty argument). A NUL can
  never be part of a spawned argument, so the encoding is lossless for every argument `spawn` accepts.
- `ctl` is a 127.0.0.1 port on which the host listens. Pruv connects, writes one byte (`started`) after a
  successful spawn, and then watches the socket: any received byte means `stop`, EOF or a socket error means
  `disconnect`. This replaces the TS `process.on('message' | 'disconnect')` and the signal handlers.
- The result is a flat `[i64]` packet (below), which the host turns into the TS `exit` message.

No `par`, no general allocator, no event bus. One `Loop` with two slots (process and control socket).

### Order of operations (as in TS `run`)

1. Check the arguments (bad input is a packet, not a panic).
2. Connect the control port (new: TS receives `run` over an existing IPC pipe).
3. Create the output file (`file_create`, truncate). Failure sets `fileError = create` and the command still runs;
   `fileBytes` is then absent.
4. Spawn `shell` with the split arguments and `cwd`, then close stdin at once (EOF stands for `'ignore'`).
   A spawn failure reports `spawn failed` with the `proc_error` code.
5. Send `started`, add the process and the socket to the loop and pump.
6. On normal end: drain both streams, `ev_proc_wait`, publish the exit status. `reaped` = the wait succeeded.
7. On stop or disconnect: drain bytes already queued, then `ev_remove` the process (kill and reap).
8. Close the output file, flush the tail decoders, cut the tail, build the packet, close the loop.

### Output file (TS `take`, file half)

`total` counts every byte read. While the file is open and has no error and is not truncated, the prefix of the
chunk that fits in `fileLimit - written` is written with one `file_write_bytes`; `written` adds its length only if
the write succeeded. A chunk longer than the room sets `truncated`. A failed write sets `fileError = write` and no
later chunk is written. After the run, `file_close`; a false result sets `fileError = write` if no error is set yet.

### Tail (TS `take`, tail half, and `finalTail`)

One UTF-8 decoder per stream (WHATWG decoder: maximal-subpart U+FFFD replacement, at most three pending bytes,
one U+FFFD for an incomplete sequence at the end). Decoded code points go, in read order, into one buffer of UTF-16
units (`2 * tailChars + 2` slots). When a code point pushes the count past `2 * tailChars`, the last `tailChars`
units move to the front, as TS keeps the last `tailChars` units of its string. At the end both decoders are flushed
(stream 1 then stream 2) and the tail is the last `tailChars` units, without a leading low surrogate (P5), encoded
as UTF-8.

Every trim keeps the last `tailChars` units, so the result is the last `tailChars` units of the whole decoded output
independently of chunk sizes.

## Scope

### Function mapping

| TS | Pruv (`bench/l3/supervisor/`) | Extent |
|---|---|---|
| `message.args` | `args.pruv::arg_spans`, `build_args` | NUL-terminated split; spans are proved in range and on character boundaries |
| `take` (file) | `capfile.pruv::cap_take`, `cap_finish` | One write per chunk, all-or-nothing count |
| `take` (tail), `decoder.end`, `finalTail` | `tail.pruv::tail_push`, `tail_flush`, `tail_bytes` | Buffer of UTF-16 units |
| `run`, `stop`, `report` | `supervise.pruv::supervise` | Loop with a process and a control socket |
| `exit` message fields | `packet.pruv::packet` | Flat record plus tail bytes |

### Host responsibilities

- The JSON IPC (`ready`, `run`, `started`, `exit`) and the control listener. `started` carries no pid.
- `env`: Pruv spawns with the supervisor's environment, so the host sets it before calling.
- Mode 0600: the host creates the output file with mode 0600 before the call; `file_create` truncates and keeps it.
- The result file (`<out>.result.json`, temporary file and rename, `finishedAt`, `delegationId`).
- Negative exit status to a signal name (`-15` → `SIGTERM`), and `code: null` when the status is unknown.
- Process groups, the TERM → 5 s → KILL ladder, the Windows job-object helper and `readStartId`.
- Translated error texts (`fileErrorI18n`, `spawnErrorI18n`); the packet only carries codes.

## Boundary representation

Packet (`[i64]`):

| index | field | values |
|---|---|---|
| 0 | status | 0 ok, 1 bad input |
| 1 | kind | 1 exited, 2 spawn failed, 3 loop broken, 4 control connect failed, 5 bad input |
| 2 | exit_status | exit code or negative signal (kind 1, when known); `proc_error` (kind 2); else 0 |
| 3 | exit_known | 1 when exit_status is a real exit status |
| 4 | stopped | 0 none, 1 stop, 2 disconnect |
| 5 | reaped | 1 when the child was reaped |
| 6 | truncated | 0/1 |
| 7 | file_error | 0 none, 1 create, 2 write |
| 8 | file_open | 1 when the output file was created (TS sends `fileBytes`) |
| 9 | total | bytes read from both streams |
| 10 | file_bytes | bytes written to the file |
| 11 | tail_len | n |
| 12.. | tail | n UTF-8 bytes |

`packet.pruv` ensures the header length and value ranges; `check_packet` in the tests is an independent
Python checker.

## Policies (differences from TS)

- **P1 stop is a kill.** Stop and disconnect drain queued output, then `ev_remove` kills the shell directly. There is
  no TERM grace period and no process group (host ladder). `ev_remove` publishes no status, so `exit_known = 0`;
  TS reports the signal the ladder sent.
- **P2 reaped.** `reaped` means the shell itself was reaped. TS reports whether the whole group is gone.
- **P3 whole writes.** `file_bytes` counts only successful whole writes; TS counts partial `writeSync` results.
- **P4 close errors.** A failed final close sets `fileError = write`; TS ignores close errors.
- **P5 surrogate split.** When the final cut falls inside a surrogate pair, the half pair is dropped; TS keeps a lone
  surrogate.
- **P6 stream order.** Chunks are taken in read order (stream 1 before stream 2 in each pump step). TS takes them in
  Node event order; both are nondeterministic across streams.
- **P7 end wait.** The result is reported when both pipes close. A descendant that keeps a pipe open delays the
  report until stop; TS reports at most 2 s after the shell exits.
- **P8 bounds.** `1 <= tail_chars <= 65536`, `0 <= file_limit <= 2^40`, reads of at most 65536 bytes; outside these
  the packet is bad input. Event and drain loops are bounded (10^7 events); exceeding the bound reports kind 3.

## Proof targets

Safety (proved): every index, slot range and kind check before each loop operation, every `Process`, `Socket` and
`Loop` consumed on every path, argument spans in range and on character boundaries, ring indexes in range, packet
header ranges.

Semantics (proved): `arg_spans` spans partition the blob; `cap_take` never lets `written` exceed `file_limit`;
`packet` copies the header fields and tail bytes; the tail buffer count never exceeds `2 * tailChars` between pushes.

Tested rather than proved: decoder output against a WHATWG model whose per-stream text is checked against Python's
`bytes.decode('utf-8', 'replace')` (Python's incremental decoder holds back some invalid bytes at a chunk end, so it
cannot order two streams), the tail cut against a direct UTF-16 slice, and the whole run against the TS reference.

## Batches

- **A0** `args.pruv`, `packet.pruv`: spans, packet builder, Python checker.
- **A1** `tail.pruv`: decoder, unit buffer, flush and cut, against a WHATWG model over random chunkings.
- **A2** `capfile.pruv`: capped writer over real files, including limit 0, exact fit and create failure.
- **A3** `supervise.pruv`: the whole entry with real children (local Python fixtures) and a Rust host that plays
  the controller: normal exit, nonzero exit, signal exit, spawn failure, stop, disconnect, big output, truncation.
- **Q** differential against the TS reference on Node, mutants, fault injections and a measurement.

All batches run on the four pairs (py/rs compiler × rust/llvm backend).

## Q reference and oracle

`bench/l3/supervisor/reference/` holds a verbatim copy of the TS file and stubs for `../shared/i18n/wire` (types
only) and `./windows-system`. It is run with Node's type stripping as a forked child; the driver sends `run` and
`stop` messages and records `exit`. Both supervisors run the same fixture commands; the comparison maps the packet
through the host rules above and compares `code`, `signal` (via the host mapping), `stopped`, `truncated`, `bytes`,
`fileBytes`, `fileError` presence, the output file and `tail`, with the policies P1-P7 as the only allowed
differences.

## Deferred

pid and start id in `started`, environment, file mode, group kill ladder, Windows job objects, result file,
partial-write counts.

## Risks

- P7: a background job started by the command holds the report until stop.
- The Loop runtime uses a reader thread per slot; this is fine for one child but not an OS poller.
- Python's `'replace'` decoder and Node's `StringDecoder` might differ across chunk boundaries for invalid input;
  Q records any such case as found.
