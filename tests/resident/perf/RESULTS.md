| build | S | step ns | noop ns | copy ns |
|---|---:|---:|---:|---:|
| rust [integrated (inlinable)] | 2000 | 1.4 | 0.7 | 867.3 |
| rust [integrated (inlinable)] | 20000 | 2.0 | 0.8 | 7888.2 |
| llvm | 2000 | 1.9 | 1.5 | 889.4 |
| llvm | 20000 | 2.4 | 1.5 | 7897.9 |
| rust-checked [integrated (inlinable)] | 2000 | 1.4 | 0.9 | 849.3 |
| rust-checked [integrated (inlinable)] | 20000 | 2.0 | 0.7 | 7748.1 |
| llvm-checked | 2000 | 2.0 | 1.7 | 852.2 |
| llvm-checked | 20000 | 2.3 | 1.7 | 8401.5 |

Median of 5 runs, 10,000 calls per measurement. Gates: step ratio S=20000/S=2000 < 2.0 (all gated builds); noop <= 2000 ns (llvm builds only: real external call boundary).
rust-backend rows are integrated (inlinable): generated code and driver share one crate, so their noop is measured but is not a call-boundary cost and is not gated.

- rust [integrated (inlinable)] step ratio 1.43 (< 2.0): PASS
- rust [integrated (inlinable)] noop 0.8 ns (not gated: no real call boundary)
- llvm step ratio 1.26 (< 2.0): PASS
- llvm noop 1.5 ns (<= 2000): PASS
- rust-checked [integrated (inlinable)] step ratio 1.43 (< 2.0): PASS (informational)
- rust-checked [integrated (inlinable)] noop 0.9 ns (not gated: no real call boundary)
- llvm-checked step ratio 1.15 (< 2.0): PASS (informational)
- llvm-checked noop 1.7 ns (<= 2000): PASS (informational)

verdict: PASS
