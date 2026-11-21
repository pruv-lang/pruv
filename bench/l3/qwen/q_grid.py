"""Compare and measure the Pruv and hand Rust Qwen window importers on macOS."""

import argparse
import itertools
import json
import os
from pathlib import Path
import re
import shutil
import statistics
import string
import subprocess
import sys

sys.dont_write_bytecode = True
import test_impl as t
from q import line, msg

PRUV_ENTRY = "window_bytes"
HERE = t.HERE
WORK = t.WORK / "r05"
GRID = WORK / "grid"
SIZES_MIB = {"S": 2, "M": 4, "L": 8}
COLLISION_BUCKETS = 8
SUFFIX_ALPHABET = (string.ascii_letters + string.digits + "-_").encode()
RSS = re.compile(r"(?m)^\s*(\d+)\s+maximum resident set size\s*$")


def build():
    WORK.mkdir(parents=True, exist_ok=True)
    tools = t.configure()
    built = t.WORK / "rs_llvm"
    binding = built / "root_gen.rs"
    if not binding.is_file():
        raise RuntimeError("rs_llvm artifacts missing. Run python3 test_impl.py first.")

    hand = HERE / "handrust"
    # Copy the hand runner's I/O and host.rs's packet and JSON encoding.
    io = (hand / "src/main.rs").read_text().split("fn quote(", 1)[0]
    host = (HERE / "host.rs").read_text()
    packet = host[host.index("fn q("):host.index("fn head_json(")]
    record = host[host.index("fn record("):host.index("fn bytes(")].replace(
        "fn record(h:&Packet,w:&Packet)", "fn record(w:&Packet)"
    ).replace("title(h,w)", "w.field(4)")
    # The generated bindings use unsafe internally.
    io = io.replace("#![forbid(unsafe_code)]", "#![allow(dead_code)]")
    runner = WORK / "pruv.rs"
    runner.write_text(io + f"#[path = {json.dumps(str(binding), ensure_ascii=False)}] mod sol;\n"
                      + packet + record + f"""
// R05 C byte wire (boundary.pruv put_word): 8-byte LE words of v + 1, then raw text.
fn word(w: &[u8], i: usize) -> i64 {{
    i64::from_le_bytes(w[8 * i..8 * i + 8].try_into().unwrap()) - 1
}}
fn unwire(w: Vec<u8>) -> Packet {{
    assert!(w.len() >= 32 && word(&w, 0) == 0 && word(&w, 1) == 14, "wire failure {{}}", word(&w, 0));
    let (nt, nx) = (word(&w, 2) as usize, word(&w, 3) as usize);
    let hl = 8 * (18 + nt);
    assert_eq!(w.len(), hl + nx);
    let m: Vec<i64> = (4..18).map(|i| word(&w, i)).collect();
    assert_eq!(m[0], 1);
    assert_eq!(nt as i64, 10 * m[3]);
    let turns = (18..18 + nt).map(|i| word(&w, i)).collect();
    let mut text = w;
    text.drain(..hl);
    Packet {{ m, turns, text }}
}}

fn main() -> Result<(), Box<dyn std::error::Error>> {{
    let path = env::args_os().nth(1).ok_or("usage: qwen-pruv FILE")?;
    let (buffer, cut_before) = read_window(&path, 8388608)?;
    let start = Instant::now();
    // Time through the finished record JSON, the same end point as the hand runner.
    let json = record(&unwire(sol::{PRUV_ENTRY}(&buffer, i64::from(cut_before))));
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    println!("{{ms}} {{json}}");
    Ok(())
}}
""", encoding="utf-8")
    pruv = WORK / "qwen-pruv"
    t.run([
        tools.RUSTC, "-O", "--edition", "2021", runner, "-o", pruv,
        "--extern", f"pruv_str={t.WORK / 'libpruv_str.rlib'}",
        "-C", f"link-arg={built / 'qwen_gen.o'}",
        "-C", f"link-arg={built / 'qwen_rt.o'}",
    ])

    # Keep the lockfile, target directory and Cargo cache writes under work/r05.
    cargo_home = WORK / "cargo-home"
    stage = WORK / "handrust"
    stage.mkdir(exist_ok=True)
    shutil.copyfile(hand / "Cargo.toml", stage / "Cargo.toml")
    if not (stage / "src").exists():
        (stage / "src").symlink_to(hand / "src", target_is_directory=True)
    user_cargo = Path.home() / ".cargo"
    for name in ("index", "cache"):
        dest = cargo_home / "registry" / name
        if not dest.exists():
            shutil.copytree(user_cargo / "registry" / name, dest)
    env = os.environ | {"CARGO_HOME": str(cargo_home), "CARGO_TARGET_DIR": str(stage / "target")}
    env.setdefault("RUSTC", tools.RUSTC)
    subprocess.run(
        [user_cargo / "bin/cargo", "build", "--offline", "--release",
         "--manifest-path", stage / "Cargo.toml"],
        env=env, check=True,
    )
    return pruv, stage / "target/release/qwen-hand"


def bucket(key, size):
    value = 0
    for byte in key:
        value = (value * 257 + byte) % size
    return value


def collision_keys(count, modulo, denominator):
    # Appending three bytes contributes a precomputed residue. Vary the
    # prefix until a printable suffix puts the full UUID in the target bucket.
    suffixes = {}
    for a, b, c in itertools.product(SUFFIX_ALPHABET, repeat=3):
        suffixes.setdefault(((a * 257 + b) * 257 + c) % modulo, bytes((a, b, c)))
    prefix_factor = pow(257, 3, modulo)
    next_prefix = 0
    for i in range(count):
        if i % denominator:
            yield f"n{i}"
            continue
        target = (i // denominator) % COLLISION_BUCKETS
        while True:
            prefix = f"c{next_prefix:06d}".encode()
            next_prefix += 1
            offset = bucket(prefix, modulo) * prefix_factor
            suffix = suffixes.get((target - offset) % modulo)
            if suffix is not None:
                yield (prefix + suffix).decode("ascii")
                break


def chain_rows(family, count, denominator):
    if family == "dupbranch":
        # The branch is off the main chain. Main-chain entries 1, 2 and 3 form a cycle.
        for row in range(count):
            if row == 10:
                yield "assistant", "branch", "2"
                continue
            block, offset = divmod(row - (row > 10), 8)
            index = 7 * block + min(offset, 6)
            parent = "3" if index == 1 else str(index - 1) if index else ""
            yield "user" if index % 2 == 0 else "assistant", str(index), parent
        return

    keys = (
        collision_keys(count, 2 * (count + 1) + 1, denominator)
        if family == "collide"
        else map(str, range(count))
    )
    parent = ""
    for i, uuid in enumerate(keys):
        yield "user" if i % 2 == 0 else "assistant", uuid, parent
        parent = uuid


def grid_file(family, size, denominator=1):
    total = SIZES_MIB[size] * 1024 * 1024
    count = total // (8192 if family == "linear" else 200)
    width, extra = divmod(total, count)
    path = GRID / f"{family}-{size}.jsonl"
    with path.open("wb") as file:
        for i, (role, uuid, parent) in enumerate(chain_rows(family, count, denominator)):
            row = msg(role, f"{role} {i}", uuid=uuid, parentUuid=parent, padding="")
            padding = width + (i < extra) - len(line(row))
            if padding < 0:
                raise ValueError(f"entry {i} exceeds the grid width")
            row["padding"] = "p" * padding
            file.write(line(row))
        if file.tell() != total:
            raise ValueError(f"{path} has the wrong byte count")
    return path


def invoke(exe, path, *, timed=False, timeout=None):
    command = [exe, path]
    if timed:
        command = ["/usr/bin/time", "-l", *command]
    result = subprocess.run(command, capture_output=True, encoding="utf-8", timeout=timeout)
    if result.returncode:
        raise RuntimeError(f"{exe.name} {path}: exit {result.returncode}\n{result.stderr}")
    elapsed, record = result.stdout.removesuffix("\n").split(" ", 1)
    rss = None
    if timed:
        match = RSS.search(result.stderr)
        if match is None:
            raise RuntimeError(f"missing macOS peak RSS for {exe.name} {path}\n{result.stderr}")
        rss = int(match[1])
    return float(elapsed), record, rss


def equal(pruv, hand, path, timeout=None):
    _, left, _ = invoke(pruv, path, timeout=timeout)
    _, right, _ = invoke(hand, path)
    if left == right:
        return True
    print(f"불일치 {path}\nPruv {left}\nRust {right}", flush=True)
    return False


def check(pruv, hand):
    fixtures = sorted((HERE / "fixtures").glob("*.jsonl"))
    GRID.mkdir(exist_ok=True)
    grid = []
    mismatches = sum(not equal(pruv, hand, path) for path in fixtures)
    for family in ("linear", "dense", "dupbranch"):
        for size in SIZES_MIB:
            path = grid_file(family, size)
            grid.append((family, size, path))
            mismatches += not equal(pruv, hand, path)

    denominator = 1
    rows = SIZES_MIB["L"] * 1024 * 1024 // 200
    while True:
        # Probe L first so an expensive full collision does not delay all sizes.
        collision_grid = []
        collision_mismatches = 0
        try:
            for size in reversed(SIZES_MIB):
                path = grid_file("collide", size, denominator)
                collision_grid.append(("collide", size, path))
                collision_mismatches += not equal(pruv, hand, path, timeout=60)
        except subprocess.TimeoutExpired:
            if collision_mismatches:
                # Already failing: report the checked files instead of retrying.
                grid.extend(reversed(collision_grid[:-1]))
                mismatches += collision_mismatches
                break
            if 2 * denominator > rows:
                raise RuntimeError(f"collide-{size} times out even at 1/{denominator}")
            denominator = 16 if denominator == 1 else denominator * 2
            print(f"collide-{size} Pruv 실행 60초 초과. 충돌 비율을 1/{denominator}로 낮춥니다.",
                  flush=True)
        else:
            grid.extend(reversed(collision_grid))
            mismatches += collision_mismatches
            break
    total = len(fixtures) + len(grid)
    print(f"일치 검사 {total - mismatches}/{total} 통과 (픽스처 {len(fixtures)}개, 그리드 {len(grid)}개)",
          flush=True)
    return grid, mismatches


def measure(pruv, hand, grid, runs, sizes):
    cells = []
    print(f"{'입력':<12} {'크기':<4} {'Pruv ms':>10} {'Rust ms':>10}"
          f" {'Pruv RSS (B)':>14} {'Rust RSS (B)':>14}", flush=True)
    for family, size, path in grid:
        if size not in sizes:
            continue
        samples = {exe: [] for exe in (pruv, hand)}
        for _ in range(runs):
            for exe in (pruv, hand):
                ms, _, rss = invoke(exe, path, timed=True)
                samples[exe].append((ms, rss))
        values = []
        for i, (axis, unit) in enumerate((("time", "ms"), ("rss", "bytes"))):
            p, h = (statistics.median(sample[i] for sample in samples[exe])
                    for exe in (pruv, hand))
            cells.append(dict(name=family, axis=axis, unit=unit, kind="proportional",
                              size=size, pruv=p, hand=h, runs=runs))
            values.extend((p, h))
        print(f"{family:<12} {size:<4} {values[0]:10.3f} {values[1]:10.3f}"
              f" {values[2]:14.0f} {values[3]:14.0f}", flush=True)
    report = WORK / "perf_cells.json"
    report.write_text(json.dumps(cells, indent=2) + "\n")
    print(f"측정 결과 {report.relative_to(HERE)}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="build and check equality without timing")
    parser.add_argument("--runs", type=int, default=7, help="fresh process runs per implementation")
    parser.add_argument("--sizes", default="M,L", help="sizes to time (S, M, L)")
    args = parser.parse_args()
    sizes = args.sizes.split(",")
    if not sizes or any(size not in SIZES_MIB for size in sizes):
        parser.error("--sizes takes a comma list of S, M, L")
    if args.runs < 1:
        parser.error("--runs must be positive")
    pruv, hand = build()
    grid, mismatches = check(pruv, hand)
    if mismatches:
        return 1
    if not args.check:
        measure(pruv, hand, grid, args.runs, sizes)
    return 0


if __name__ == "__main__":
    sys.exit(main())
