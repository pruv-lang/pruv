// Host facts for bench/l3/p31n: the running ids of a session, as owned strings.
// Deterministic: session text "clk-NNNNNNNN" yields 4 ids run-((N+j) % 16), j = 0..4.
pub fn running_ids(sid: &str) -> Vec<String> {
    let n: usize = sid.as_bytes()[4..].iter().fold(0usize, |a, b| a * 10 + (b - b'0') as usize);
    (0..4).map(|j| format!("run-{:02}", (n + j) % 16)).collect()
}
