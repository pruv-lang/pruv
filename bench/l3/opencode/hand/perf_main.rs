#![allow(dead_code, unused_imports, unused_variables)]
#[path="@SOL@"] mod sol;

use std::hint::black_box;
use std::time::Instant;

const OFFSET: u64 = 0xcbf29ce484222325;

fn fold_word(mut digest: u64, word: u64) -> u64 {
    for byte in word.to_le_bytes() {
        digest = (digest ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    digest
}

fn config(env: usize) -> (Vec<i64>, Vec<u8>) {
    let mut cfg = vec![0; 32 + 4 * env];
    let mut text = Vec::new();
    cfg[..6].copy_from_slice(&[1, 3, 1, 1500, 1000, 1]);
    cfg[18] = env as i64;
    cfg[19] = -1;
    let mut put = |index: usize, value: &str| {
        cfg[index] = text.len() as i64;
        cfg[index + 1] = value.len() as i64;
        text.extend_from_slice(value.as_bytes());
    };
    for (i, value) in ["/h", "f", "t", "/bin/oc", "/tmp", "/home"].into_iter().enumerate() {
        put(6 + 2 * i, value);
    }
    for i in 0..env {
        put(32 + 4 * i, &format!("K{i:05}"));
        put(34 + 4 * i, "v");
    }
    (cfg, text)
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 4, "expected LANE E S");
    let lane = args[1].as_str();
    let (cfg, text) = config(args[2].parse().unwrap());
    let steps: usize = args[3].parse().unwrap();
    let (ns, digest) = match lane {
        "begin" => {
            let started = Instant::now();
            let packet = sol::opencode_begin(black_box(&cfg), black_box(&text));
            let ns = started.elapsed().as_nanos();
            assert_eq!(packet[2], 2, "begin must admit the config");
            let digest = packet.into_iter().fold(OFFSET, |hash, word| fold_word(hash, word as u64));
            (ns, digest)
        }
        "step_checked" | "step_admitted" => {
            let packet = sol::opencode_begin(&cfg, &text);
            assert_eq!(packet[2], 2, "begin must admit the config");
            let ctl = &packet[12..76];
            let mut event = [0; 16];
            event[..2].copy_from_slice(&[1, 1]);
            let checked = sol::opencode_step(&cfg, &text, ctl, &[], &[], &[], &[], &[], &[], &event, &[]);
            let admitted = sol::opencode_step_admitted(&cfg, &text, ctl, &[], &[], &[], &[], &[], &[], &event, &[]);
            assert_eq!(checked, admitted);
            assert_eq!(checked[2], 1, "reentry must be rejected");
            drop((checked, admitted));

            let step = if lane == "step_checked" {
                sol::opencode_step
            } else {
                sol::opencode_step_admitted
            };
            let mut digest = OFFSET;
            let started = Instant::now();
            for _ in 0..steps {
                let packet = step(black_box(&cfg), black_box(&text), black_box(ctl),
                                  &[], &[], &[], &[], &[], &[], black_box(&event), &[]);
                digest = packet.into_iter().fold(digest, |hash, word| fold_word(hash, word as u64));
            }
            (started.elapsed().as_nanos(), digest)
        }
        _ => panic!("unknown lane {lane}"),
    };
    println!("{ns} {digest:016x}");
}
