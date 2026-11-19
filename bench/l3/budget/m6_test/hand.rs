// Hand-written Rust baseline for the M6 byte-output helpers. harness.rs links this file as `mod hand` (ops `hand_<op>`),
// so the perf lanes share one executable; `main` keeps it buildable on its own.
use std::io::{self, BufRead};
use std::time::Instant;

fn int_len(x: i64) -> usize {
    let mut value = x.unsigned_abs();
    let mut len = 1 + usize::from(x < 0);
    while value >= 10 {
        value /= 10;
        len += 1;
    }
    len
}

fn put_digits(out: &mut Vec<u8>, mut value: u64, digits: usize) {
    let start = out.len();
    out.resize(start + digits, 0);
    for byte in out[start..].iter_mut().rev() {
        *byte = b'0' + (value % 10) as u8;
        value /= 10;
    }
}

fn put_int(out: &mut Vec<u8>, value: i64, len: usize) {
    if value < 0 {
        out.push(b'-');
    }
    put_digits(out, value.unsigned_abs(), len - usize::from(value < 0));
}

fn int_text(value: i64) -> Vec<u8> {
    let len = int_len(value);
    let mut out = Vec::with_capacity(len);
    put_int(&mut out, value, len);
    out
}

// Inverse of the Gregorian days-from-civil transform.
fn civil(ms: i64) -> (i64, i64, i64, i64, i64, i64, i64) {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    (y, m, d, rem / 3_600_000, rem / 60_000 % 60, rem / 1000 % 60, rem % 1000)
}

fn iso(ms: i64) -> Vec<u8> {
    let (year, month, day, hour, minute, second, millis) = civil(ms);
    let extended = !(0..=9999).contains(&year);
    let mut out = Vec::with_capacity(if extended { 27 } else { 24 });
    if extended {
        out.push(if year < 0 { b'-' } else { b'+' });
    }
    put_digits(&mut out, year.unsigned_abs(), if extended { 6 } else { 4 });
    for (separator, value, digits) in [
        (b'-', month, 2),
        (b'-', day, 2),
        (b'T', hour, 2),
        (b':', minute, 2),
        (b':', second, 2),
        (b'.', millis, 3),
    ] {
        out.push(separator);
        put_digits(&mut out, value as u64, digits);
    }
    out.push(b'Z');
    out
}

const COUNTER: [&[u8]; 10] = [
    "시간".as_bytes(),
    "time".as_bytes(),
    "위임".as_bytes(),
    "delegations".as_bytes(),
    "빌드·명령".as_bytes(),
    "commands".as_bytes(),
    "감사".as_bytes(),
    "reviews".as_bytes(),
    "실제 확인".as_bytes(),
    "checks".as_bytes(),
];

const NO_LIMIT: [&[u8]; 2] = [
    "제한 없음".as_bytes(),
    "no limit".as_bytes(),
];

const REFUSAL_PREFIX: [&[u8]; 2] = [
    "예산 한도(".as_bytes(),
    "The request budget limit (".as_bytes(),
];

const REFUSAL_SUFFIX: [&[u8]; 2] = [
    ")에 닿아 사용자가 늘리거나 끝내기를 고를 때까지 멈춘다. 새 위임·명령을 시도하지 않고 지금까지 결과와 확인하지 못한 것만 짧게 보고한다.".as_bytes(),
    ") was reached, so work is paused until the user extends it or ends it. Do not try new delegations or commands; briefly report the results so far and what you could not verify.".as_bytes(),
];

const CARD_ERROR: [&[u8]; 14] = [
    "지금 예산을 찾을 수 없어 작업 카드를 저장하지 못했다.".as_bytes(),
    "The current budget was not found, so the work card was not saved.".as_bytes(),
    "deliverable 은 1~300자여야 한다.".as_bytes(),
    "deliverable must be 1–300 characters.".as_bytes(),
    "done 은 각 300자 이하 1~5줄이어야 한다.".as_bytes(),
    "done must be 1–5 lines of at most 300 characters each.".as_bytes(),
    "targets 의 수는 0 이상 정수여야 하고 note 는 300자 이하여야 한다.".as_bytes(),
    "targets counts must be non-negative integers and note at most 300 characters.".as_bytes(),
    "verify 는 label 1~300자와 status(planned·done·skipped)를 가진 0~5개 항목이어야 한다.".as_bytes(),
    "verify must be 0–5 items with a label of 1–300 characters and a status (planned, done or skipped).".as_bytes(),
    "conflicts 는 각 300자 이하 0~5줄이어야 한다.".as_bytes(),
    "conflicts must be 0–5 lines of at most 300 characters each.".as_bytes(),
    "state 는 working·done·waiting-user·blocked 중 하나여야 한다.".as_bytes(),
    "state must be one of working, done, waiting-user or blocked.".as_bytes(),
];


fn amount_or_refusal(
    reason: usize,
    unlimited: bool,
    used: i64,
    limit: i64,
    lang: usize,
    refusal: bool,
) -> Vec<u8> {
    let used = if reason == 0 { used.div_euclid(60_000) } else { used };
    let used_len = int_len(used);
    let limit_len = if unlimited { NO_LIMIT[lang].len() } else { int_len(limit) };
    let mut len = used_len + 1 + limit_len;
    let counter = COUNTER[2 * reason + lang];
    if refusal {
        len += REFUSAL_PREFIX[lang].len() + counter.len() + 1 + REFUSAL_SUFFIX[lang].len();
    }
    let mut out = Vec::with_capacity(len);
    if refusal {
        out.extend_from_slice(REFUSAL_PREFIX[lang]);
        out.extend_from_slice(counter);
        out.push(b' ');
    }
    put_int(&mut out, used, used_len);
    out.push(b'/');
    if unlimited {
        out.extend_from_slice(NO_LIMIT[lang]);
    } else {
        put_int(&mut out, limit, limit_len);
    }
    if refusal {
        out.extend_from_slice(REFUSAL_SUFFIX[lang]);
    }
    out
}

fn card_error(kind: usize, lang: usize) -> Vec<u8> {
    let text = CARD_ERROR[2 * kind + lang];
    let mut out = Vec::with_capacity(text.len());
    out.extend_from_slice(text);
    out
}

fn checksum(n: i64, mut render: impl FnMut(i64) -> Vec<u8>) -> i64 {
    let mut sum = 0;
    for i in 0..n {
        let bytes = render(i & 1);
        let len = bytes.len();
        if len > 0 {
            let parity = (bytes[0] ^ bytes[len / 2] ^ bytes[len - 1]) & 1;
            sum += len as i64 + i64::from(parity);
        }
    }
    sum
}

pub fn run(op: &str, w: &[i64]) -> i64 {
    let n = w[0];
    match op {
        "p_iso" => checksum(n, |d| iso(w[1] + d)),
        "p_int" => checksum(n, |d| int_text(w[1] + d)),
        "p_ref" | "p_amount" => checksum(n, |d| {
            amount_or_refusal(w[1] as usize, w[2] != 0, w[3] + d, w[4], w[5] as usize, op == "p_ref")
        }),
        "p_card_error" => checksum(n, |d| card_error((w[1] + d) as usize, w[2] as usize)),
        _ => panic!("unknown op {op}"),
    }
}

fn main() {
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let mut fields = line.split_whitespace();
        let Some(op) = fields.next() else { continue };
        let args: Vec<i64> = fields.map(|value| value.parse().unwrap()).collect();
        let start = Instant::now();
        let sum = run(op, &args);
        println!("ns {} {sum}", start.elapsed().as_nanos());
    }
}
