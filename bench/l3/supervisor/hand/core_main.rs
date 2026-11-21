#[path = "@SOL@"]
mod sol;
#[path = "@PATTERN@"]
mod pattern;

use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let t: i64 = args[1].parse().unwrap();
    let chunk: i64 = args[2].parse().unwrap();
    let size: usize = args[3].parse().unwrap();
    let data: Vec<u8> = pattern::PATTERN.iter().copied().cycle().take(size).collect();
    let input = data.as_slice().into();
    let start = Instant::now();
    let result = sol::core(t, input, chunk);
    let ns = start.elapsed().as_nanos();
    let hash = result.iter().fold(0xcbf29ce484222325u64, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    println!("{} {} {:016x}", ns, result.len(), hash);
}
