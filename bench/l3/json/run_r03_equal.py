#!/usr/bin/env python3
"""Compare hand Rust, generated Rust, and the existing fixture oracles."""
import importlib.util
import json
import os
from pathlib import Path
import random
import shutil
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
import run_a2
import run_a4

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
WORK = REPO / "work/r03"
HAND = HERE / "hand/hand_json.rs"

HARNESS = r'''#![allow(dead_code)]
#[path = @PARSE@] mod a2;
#[path = @QUERY@] mod a4;
#[path = @HAND@] mod hand;
use std::io::{self, BufRead};

fn nums<T: std::str::FromStr>(items: &str) -> Vec<T> {
    items.split_whitespace().map(|s| s.parse().ok().unwrap()).collect()
}

fn main() {
    let mut mismatches = 0;
    for (index, line) in io::stdin().lock().lines().enumerate() {
        let line = line.unwrap();
        let fields: Vec<_> = line.split('|').collect();
        let (generated, hand, expected) = match fields[0] {
            "parse" => {
                let data = nums(fields[1]);
                (a2::parse_tape(&data), hand::parse_tape(&data),
                 (fields[2] != "?").then(|| nums(fields[2])))
            }
            "records" => {
                let children = fields[1].parse::<usize>().unwrap();
                let mut data = vec![b'['; 2 * children + 1];
                for pair in data[1..].chunks_exact_mut(2) {
                    pair.copy_from_slice(b"0,");
                }
                *data.last_mut().unwrap() = b']';
                (a2::parse_tape(&data), hand::parse_tape(&data), None)
            }
            "bytes" => {
                let size = fields[1].parse::<usize>().unwrap();
                let mut data = vec![b'a'; size];
                data[0] = b'"';
                data[size - 1] = b'"';
                (a2::parse_tape(&data), hand::parse_tape(&data), Some(nums(fields[2])))
            }
            "query" => {
                let data = nums(fields[1]);
                let ops = nums(fields[2]);
                let key = nums(fields[3]);
                let text = String::from_utf8(nums(fields[4])).unwrap();
                let mode = fields[5].parse().unwrap();
                let expected = (fields[6] != "?").then(|| nums(fields[6]));
                (a4::query(&data, &ops, &key, &text, mode),
                 hand::query(&data, &ops, &key, &text, mode), expected)
            }
            _ => unreachable!(),
        };
        let oracle_ok = match (fields[0], expected.as_ref()) {
            ("records", _) if fields[1] == "1048575" => {
                generated.len() == 6 * 1048576
                    && generated[..6] == [5, 0, 2097151, 1048576, -1, 1048575]
                    && generated[6..].chunks_exact(6).enumerate().all(|(i, row)| {
                        let r = (i + 1) as i64;
                        row == [3, 2 * r - 1, 2 * r, r + 1, 0, 0]
                    })
            }
            ("records", _) => generated == [-1, 2097151, 2],
            (_, Some(want)) => &generated == want,
            _ => true,
        };
        if generated != hand || !oracle_ok {
            let first = generated.iter().zip(&hand).position(|(a, b)| a != b).unwrap_or(0);
            eprintln!("case {} ({}), slot {}: generated len {} {:?}; hand len {} {:?}; oracle {:?}",
                index + 1, line.chars().take(140).collect::<String>(), first,
                generated.len(), generated.get(first..first.saturating_add(8).min(generated.len())),
                hand.len(), hand.get(first..first.saturating_add(8).min(hand.len())),
                expected.as_ref().map(|v| &v[..v.len().min(8)]));
            mismatches += 1;
        }
    }
    println!("{mismatches} mismatches");
    std::process::exit(i32::from(mismatches != 0));
}
'''


def run(command, **kwargs):
    return subprocess.run(command, check=True, text=True, timeout=600, **kwargs)


def compile_roots(build, query_root):
    """Compile the real a2 entry point and the selected query entry point."""
    os.environ["TMPDIR"] = str(build)
    spec = importlib.util.spec_from_file_location("g2", REPO / "bench/g2/run.py")
    g2 = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(g2)
    binary = Path(os.environ.get("PRUVC_RS", g2.PRUVC_RS)).resolve()
    git_file = REPO / ".git"
    if not binary.is_file() and git_file.is_file():
        original = Path(git_file.read_text().strip().removeprefix("gitdir: ")).parents[2]
        binary = original / "pruvc-rs/target/release/pruvc"
    if not binary.is_file():
        raise FileNotFoundError("pruvc binary not found; set PRUVC_RS to an existing build")
    g2.COMPILERS["rs"] = [str(binary)]
    adapter = build / "libpruv_str.rlib"
    run([g2.RUSTC, "--crate-name", "pruv_str", "--crate-type", "rlib", "--edition", "2021",
         "-O", HERE / "str_adapter/src/lib.rs", "-o", adapter])
    rustflags = ["--extern", f"pruv_str={adapter}"]
    sources = []
    for root in (HERE / "a2_root.pruv", query_root):
        manifest = root.with_suffix(".pruv.json")
        if not manifest.is_file():
            # Keep dependency paths absolute when the fallback manifest moves.
            manifest = build / f"{root.stem}.pruv.json"
            template = json.loads((HERE / "a4_root.pruv.json").read_text())
            template["root"] = str(root)
            for dep in template["dependencies"].values():
                dep["path"] = str((HERE / dep["path"]).resolve())
            manifest.write_text(json.dumps(template))
        print(f"Compiling {root.name}", flush=True)
        result = g2.compile_pruv("rs", str(root), str(build), "rust", [
            "--manifest", str(manifest), "--rust-extern", f"pruv_str={adapter}",
            "--rust-search", str(build),
        ])
        if result["rc"]:
            raise RuntimeError(f"{root.name}: {result}")
        sources.append(result["rs"])
    return g2.RUSTC, rustflags, sources


def build_runner(build, template, query_root):
    build.mkdir(parents=True, exist_ok=True)
    rustc, flags, (parse, query) = compile_roots(build, query_root)
    harness = build / "main.rs"
    harness.write_text(template.replace("@PARSE@", json.dumps(parse))
                       .replace("@QUERY@", json.dumps(query))
                       .replace("@HAND@", json.dumps(str(HAND))))
    exe = build / "runner"
    run([rustc, "-O", "--edition", "2021", harness, "-o", exe, *flags])
    return exe


def numbers(items):
    return " ".join(map(str, items))


def parse_case(data, expected):
    return f"parse|{numbers(data)}|{numbers(expected)}"


def query_case(data, ops, key, text, mode, expected=None):
    return "|".join(("query", numbers(data), numbers(ops), numbers(key),
                     numbers(text.encode()), str(mode),
                     "?" if expected is None else numbers(expected)))


def random_cases():
    rng = random.Random(303)
    for i in range(400):
        text = rng.choice(["", "value", "한글", "😀", "a\nb"])
        value = rng.choice([None, False, True, rng.randrange(-10**17, 10**17),
                            -0.25, text, [text, i], {"nested": [i, text]}])
        slot = rng.randrange(4)
        array = [rng.randrange(100) for _ in range(4)]
        array[slot] = {text: value, "other" + text: i}
        data = json.dumps(array, ensure_ascii=bool(i % 2)).encode()
        yield parse_case(data, run_a2.expected_tape(data))
        op = rng.choice([2, 3])
        ops = [1, slot, op, 0]
        if i % 7 == 0:
            ops += [0, rng.choice([-5, 1, 2**63 - 1])]
        if i % 11 == 0:
            ops += [99, 0]
        if i % 13 == 0:
            ops.append(1)
        mode = rng.randrange(8)
        key = run_a4.units(text)
        tape = run_a4.document(data)
        # The fixture oracle uses only documented opcodes/modes.
        expected = None
        if mode < 7 and len(ops) == 4:
            expected = run_a4.answer(tape, ops, key, text, mode)
        yield query_case(data, ops, key, text, mode, expected)
        malformed = data[:rng.randrange(len(data))] + rng.choice([b"", b"]", b"\xff", b"\\u1", b"?"])
        yield f"parse|{numbers(malformed)}|?"


def main():
    a2 = [parse_case(data, want) for _, data, want in run_a2.FIXTURES]
    a2 += ["records|1048575", "records|1048576",
           "bytes|16777216|4 0 16777216 1 -1 0", "bytes|16777217|-1 0 1"]
    a4 = [query_case(*fixture) for fixture in run_a4.fixtures()]
    generated_cases = list(random_cases())
    WORK.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="equal-build-", dir=WORK) as directory:
        exe = build_runner(Path(directory), HARNESS, HERE / "a4_root.pruv")
        print(f"a2: {len(a2)} fixtures; a4: {len(a4)} fixtures; "
              f"{len(generated_cases)} additional random cases (400 documents)", flush=True)
        run([exe], input="\n".join(a2 + a4 + generated_cases) + "\n")


if __name__ == "__main__":
    main()
