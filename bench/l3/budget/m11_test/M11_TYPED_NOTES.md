# M11 typed resident state

The Rust harness owns the Pruv session map. A session stores typed budget properties, the ID and state indexes used for lookup, and the admitted card input with its validated field offsets and timestamp. Pruv handles lookup, card judgment, mutation, and insertion ordinals.

The direct lane passes IDs as text after Pruv admits packed keys. The archived adapter decodes V words and projects typed replies to the same fixture format. The direct lane uses the same output projection for Q comparisons.

Run from the repository root with `PRUVC_RS` pointing at the fresh Rust compiler.

```sh
M11_LANE=direct python3 bench/l3/budget/m11_test/run_m11.py --typed --pairs rs,py
M11_LANE=adapter python3 bench/l3/budget/m11_test/run_m11.py --typed --pairs rs,py --no-build
python3 bench/l3/budget/m11_test/run_m11.py --typed --pairs rs --no-build --perf-cells work/p32b/d67_perf_cells.json
```
