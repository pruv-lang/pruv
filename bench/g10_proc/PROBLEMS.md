# G10 proc problems

Use default verification. Each project has `qNN/root.pruv`. The grader fixes the public
headers below; other helpers are permitted. Programs, arguments and working directories are
host `str` values; no `Process` crosses the host boundary. The grader runs a fixture child
(`child.rs`) whose behaviour is described here; what the child prints is checked at run
time, not proved.

## q01 — a `--version` run

```
pub fn version(program: str, flag: str, cwd: str) -> i64 ! proc, alloc
  ensures -31 <= result && result < 256000000000000
```

Run `program` with exactly two arguments, `--version` and `flag` (never split or quoted by
you), in working directory `cwd` (`""` inherits). Close its stdin, collect all of stdout and
stderr, then wait for it.

Let `h(bytes)` start at 1 and, for every byte `b` in order, become `(h * 256 + b) % 999983`.
On a normal exit with code `0 <= code <= 255`, return
`code * 10^12 + h(stdout) * 10^6 + h(stderr)`.

Errors, first match wins:

- spawn failed: wait for the handle, return `-10 - error` (error is `proc_error` of the handle)
- closing stdin or collecting output failed: kill, return `-20 - error`
- the wait result is an error: `-30`
- the exit code is outside `0..255` (for example a signal exit): `-31`

The fixture child, run as `child --version FLAG`, prints each argument followed by LF on
stdout, its working directory followed by LF on stderr, reads stdin to EOF and exits with
`len(FLAG) % 256` (bytes). With `FLAG == "ABORT"` it aborts after printing.

```python
def h(data):
    v = 1
    for b in data:
        v = (v * 256 + b) % 999983
    return v

def version(program, flag, cwd):
    try:
        p = subprocess.Popen([program, '--version', flag], cwd=cwd or None, stdin=PIPE, stdout=PIPE, stderr=PIPE)
    except ValueError:  return -11   # empty program, NUL
    except OSError:     return -12
    out, err = p.communicate(b'')
    if not 0 <= p.returncode <= 255:
        return -31
    return p.returncode * 10**12 + h(out) * 10**6 + h(err)
```

Cases: empty flag; spaces, quotes, backslashes, `-x=1 'y'`; UTF-8; a 255-byte flag (code
255) and a 300-byte flag (code 44); inherited cwd and a subdirectory with a space and
non-ASCII name; empty program `-11`; missing program `-12`; missing cwd `-12`; `ABORT` `-31`.

## q02 — streaming line statistics

```
pub fn scan(program: str, mode: str, budget: i64) -> i64 ! proc, alloc
  requires 0 <= budget && budget <= 10000
  ensures -4 <= result && result <= 4096000040960000
```

Run `program` with the two arguments `lines` and `mode` (cwd inherited) and close its stdin.
Read stdout in chunks of at most 4096 bytes, making at most `budget` read calls in total; a
read that returns an empty chunk is EOF and also counts. Do not collect the whole output and
do not read stderr yourself (the child floods it while writing stdout).

Count lines by bytes only: every LF (byte 10) ends a line, including an empty line; a
non-empty rest after the last LF is one more line. The length of a line excludes its LF;
no other byte is special (CR is ordinary). Return `lines * 10^8 + longest`, where `longest`
is the length of the longest line (0 with no lines).

Errors: spawn failed `-1` (consume the handle); closing stdin or a read failed, or the wait
result is an error `-2` (kill on the first two); EOF not reached within `budget` reads: kill
the child, `-3`; after EOF, wait; an exit code other than 0 `-4`.

The fixture child `child lines KIND` writes the KIND text to stdout in 1777-byte pieces
while a thread writes 300 KiB to stderr, then exits 0 (an unknown KIND exits 2 with no
output; `fail` exits 3):

| KIND | stdout |
|---|---|
| empty | nothing |
| one | `abc` |
| blank | three LF |
| long | 20000 `x`, LF, `yz` |
| mix | for i in 0..300, `chr(97 + i % 26) * ((i * 37) % 5000)`, joined by LF |
| bin | bytes 0..255 except 10, then LF CR LF CR |
| big | 3000 lines of 999 `q` |
| fail | `partial` LF, exit 3 |
| forever | `y` LF until killed |

Cases (expected): empty/10 `0`, empty/1 `0`, empty/0 `-3`, one/10 `100000003`, one/0 `-3`,
blank/10 `300000000`, long/50 `200020000`, mix/1000, bin/10 `300000255`, big/10000
`300000000999`, big/10 `-3`, fail/100 `-4`, forever/50 `-3`, forever/0 `-3`, missing
program `-1`, unknown mode `-4`.
