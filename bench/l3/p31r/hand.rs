// Hand Rust for bench/l3/p31r/p31r.pruv.
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct Clock {
    pub armed: bool,
    pub used: i64,
    pub note: String,
}

#[derive(Clone, Debug)]
pub struct St {
    pub clocks: HashMap<String, Clock>,
}

pub fn end(st: &mut St, bid: &str) -> i64 {
    match st.clocks.remove(bid) {
        Some(c) => {
            if c.armed {
                c.used
            } else {
                0
            }
        }
        None => -1,
    }
}

pub fn rekey(st: &mut St, from: &str, to: &str) -> i64 {
    match st.clocks.remove(from) {
        Some(mut c) => {
            if c.used < 1_000_000_000_000 {
                c.used += 1;
            }
            if st.clocks.len() < 1_000_000 {
                st.clocks.insert(to.to_string(), c);
                0
            } else {
                4
            }
        }
        None => 1,
    }
}
