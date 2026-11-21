#![forbid(unsafe_code)]

use serde_json::Value;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};

// Each script entry is an expected request followed by its ready-to-send reply.
// A blank reply represents a notification. Compare requests as JSON.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    assert_eq!(args.next().as_deref(), Some("app-server"));
    assert_eq!(args.next().as_deref(), Some("--stdio"));
    let mut script = BufReader::new(File::open(args.next().ok_or("missing replay script")?)?);
    let mut input = io::stdin().lock();
    let mut output = io::BufWriter::new(io::stdout().lock());
    let (mut expected, mut request, mut reply) = (String::new(), String::new(), Vec::new());
    loop {
        expected.clear();
        request.clear();
        if script.read_line(&mut expected)? == 0 || input.read_line(&mut request)? == 0 {
            break;
        }
        let want: Value = serde_json::from_str(&expected)?;
        let got: Value = serde_json::from_str(&request)?;
        if want != got {
            return Err(format!("unexpected request: {got}, wanted {want}").into());
        }
        reply.clear();
        if script.read_until(b'\n', &mut reply)? == 0 {
            return Err("missing replay reply".into());
        }
        if reply != b"\n" {
            output.write_all(&reply)?;
            output.flush()?;
        }
    }
    Ok(())
}
