# G10 async Q measurement

```sh
/Users/spica4345/Projects/pruv/.venv/bin/python bench/g10_async/perf.py --compiler both --backend both --repeat 3
```

`perf.py` accepts `--compiler py|rs|both` and `--backend rust|llvm|both`.
Set `PRUVC_RS` to an existing Rust compiler binary if the copy has not been built.
All Pruv builds use default verification. `host.rs` links the generated Pruv server;
`threaded.rs` is the handwritten Rust server, one worker per client.

Both servers bind only `127.0.0.1:0` and announce the assigned port. The external
`client.py` connects all 50 clients before a barrier releases the same deterministic
65536-byte payload on every client: byte i is `(i * 7 + 3) % 251`. Each client
half-closes its sending side, reads through EOF and compares the entire returned byte
array, detecting loss, duplication, corruption and trailing bytes. Total: 3276800 bytes.

The Pruv server uses one Loop with 51 slots, stops accepting after 50 clients and
removes every ended socket before final `ev_close`. The current Loop runtime itself
uses worker threads per slot; this is not an OS-poller performance claim.

The two modes are separate end-to-end measurements:

- `per_byte`: read at most one byte, observe it through `ev_byte`, allocate one byte,
  and write one byte (native Rust also reads/writes one byte).
- `bulk_copy`: read at most 4096 bytes, copy through `ev_bytes`, write the whole chunk
  (native Rust explicitly copies its chunk too).

These modes include runtime, allocation and socket costs; their difference is not a
claim of isolated instruction/shim overhead. Rust-backend runs do not exercise an LLVM
shim. No performance target is declared.

Server wall time starts before the external client process is launched and ends when
the server exits. Server RSS and live OS threads are sampled with `ps` at 10ms plus
sampling overhead; these are observed maxima, not guaranteed absolute peaks. Client
memory/threads are excluded. Missing samples produce null fields rather than zeros.
Results retain each run, not just a median, in `../results/perf_<compiler>.json`.
Bind/connect permission errors are reported without requesting elevation or working
around sandbox restrictions. Only subprocesses created by this driver are terminated.

## Recorded run (2026-11-16, macOS arm64, `--repeat 3`)

Medians of three runs, every run `EXACT_ECHO OK` (3,276,800 bytes). RSS and threads are the
highest sampled values. The first Pruv per-byte run of the Python compiler took 18.4 s
(a cold first run); the medians below are not affected by it.

| compiler | backend | mode | Pruv | Rust (thread per client) |
|---|---|---|---|---|
| py | rust | per_byte | 5.04 s, 8.4 MB | 1.91 s, 2.8 MB |
| py | llvm | per_byte | 5.33 s, 8.9 MB | 1.91 s, 2.8 MB |
| rs | rust | per_byte | 5.11 s, 7.8 MB | 1.91 s, 2.8 MB |
| rs | llvm | per_byte | 5.46 s, 8.7 MB | 1.94 s, 2.8 MB |
| py | rust | bulk_copy | 38 ms, 1.7 MB | 37 ms, 1.6 MB |
| py | llvm | bulk_copy | 41 ms, 1.8 MB | 39 ms, 1.6 MB |
| rs | rust | bulk_copy | 37 ms, 1.7 MB | 39 ms, 1.6 MB |
| rs | llvm | bulk_copy | 52 ms, 1.8 MB | 43 ms, 1.6 MB |

Live threads: the Rust server shows 51 in every per-byte run. The Pruv server shows 2-12:
each reader thread queues its client's 64 KiB and ends at EOF long before the program has
consumed the queue one byte at a time, so few readers are alive at any sample. Bulk-copy
runs last about 40 ms, so only a few samples are taken and thread counts are noise.

The Rust per-byte server writes through a 4 KiB `BufWriter`: with one `write` call per byte,
macOS loopback lost the tail of an echo in about one run in three (clean EOF, correct prefix).
