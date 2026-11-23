// Hand Rust twin of borrow_demo.pruv (hand_rpc.rs style: a request that borrows its input, and `text_of` whose
// result borrows the input or a reused scratch buffer).
// Build: rustc -O --edition 2021 borrow_hand.rs; the output must equal the compiled borrow_demo.pruv.

struct Req<'a> {
    method: &'a [u8],
    path: &'a [u8],
    n: i64,
}

fn find(src: &[u8], start: usize, end: usize, c: u8) -> usize {
    src[start..end].iter().position(|&b| b == c).map_or(end, |k| start + k)
}

fn parse_req(src: &[u8]) -> Req<'_> {
    let a = find(src, 0, src.len(), b' ');
    let s = if a < src.len() { a + 1 } else { a };
    let b = find(src, s, src.len(), b' ');
    Req { method: &src[..a], path: &src[s..b], n: src.len() as i64 }
}

fn text_of<'a>(src: &'a [u8], lo: usize, hi: usize, buf: &'a mut Vec<u8>) -> &'a [u8] {
    if find(src, lo, hi, b'\\') == hi {
        return &src[lo..hi];
    }
    buf.clear();
    buf.extend(src[lo..hi].iter().copied().filter(|&b| b != b'\\'));
    &buf[..]
}

fn sum(x: &[u8]) -> i64 {
    x.iter().map(|&b| b as i64).sum()
}

fn main() {
    let r: Vec<u8> = b"GET /p\\q/r 1".to_vec();
    let mut buf: Vec<u8> = Vec::new();
    let q = parse_req(&r);
    println!("{}", q.method.len());
    println!("{}", q.path.len());
    println!("{}", sum(q.method));
    println!("{}", sum(q.path));
    println!("{}", q.n);
    let t1 = text_of(&r, 4, 10, &mut buf);
    println!("{}", t1.len());
    println!("{}", sum(t1));
    let t2 = text_of(&r, 0, 3, &mut buf);
    println!("{}", t2.len());
    println!("{}", sum(t2));
}
