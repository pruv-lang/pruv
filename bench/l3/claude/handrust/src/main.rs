use l3_claude_handrust as sol;
use std::{
    env, fs,
    hint::black_box,
    io::{self, Read, Write},
    time::Instant,
};

fn call(op: &str, data: &[u8], flag: i64) -> Vec<i64> {
    match op {
        "head" => sol::head_packet(data, flag),
        "tail" => sol::tail_packet(data, flag),
        "window" => sol::window_packet(data, flag),
        _ => panic!("unknown op: {op}"),
    }
}

fn main() -> io::Result<()> {
    let a: Vec<String> = env::args().collect();
    if a.len() < 3 {
        eprintln!("usage: {} <input|-> <head|tail|window|check|dump> [flag] [op for check/dump]", a[0]);
        std::process::exit(2);
    }
    let data = if a[1] == "-" {
        let mut data = Vec::new();
        io::stdin().lock().read_to_end(&mut data)?;
        data
    } else {
        fs::read(&a[1])?
    };
    let flag = a.get(3).map_or(0, |s| s.parse().expect("invalid flag"));
    let mode = a[2].as_str();
    if matches!(mode, "check" | "dump") {
        let op = a.get(4).map_or("window", String::as_str);
        let out = call(op, &data, flag);
        if mode == "check" {
            let mut stdout = io::BufWriter::new(io::stdout().lock());
            for word in out {
                stdout.write_all(&word.to_le_bytes())?;
            }
            return stdout.flush();
        }
        println!("{}", out.iter().map(i64::to_string).collect::<Vec<_>>().join(","));
        return Ok(());
    }

    // The generated Pruv runner uses the same timer and owns the same packet type.
    for i in 0..6 {
        let start = Instant::now();
        let out = black_box(call(black_box(mode), black_box(data.as_slice()), black_box(flag)));
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        println!("{} {} {} {}", if i == 0 { "warm" } else { "run" }, ms, out.len(), out[0]);
    }
    Ok(())
}
