// Hand Rust for bench/l3/p31n/p31n.pruv (shape of bench/l3/budget/m10b_test/hand_m10b.rs).
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct Clock {
    pub owner: String,
    pub since_ms: i64,
    pub runs: HashMap<String, i64>,
}

#[derive(Clone, Debug)]
pub struct St {
    pub clocks: HashMap<String, Clock>,
    pub out: Vec<i64>,
}

pub fn tick(st: &mut St, sid: &str, name: &str, now: i64) -> i64 {
    match st.clocks.get_mut(sid) {
        Some(c) => {
            c.since_ms = now;
            match c.runs.get_mut(name) {
                Some(v) => {
                    if *v < 1_000_000_000_000 {
                        *v += 1;
                    }
                    0
                }
                None => {
                    if c.runs.len() < 1_000_000 {
                        c.runs.insert(name.to_string(), 1);
                        2
                    } else {
                        4
                    }
                }
            }
        }
        None => 1,
    }
}

pub fn halt(st: &mut St, sid: &str, who: &str) -> i64 {
    let ids = ::p31n_facts::running_ids(sid);
    st.out.clear();
    let mut removed = 0;
    if let Some(c) = st.clocks.get_mut(sid) {
        if c.owner.as_bytes() == who.as_bytes() {
            for (i, id) in ids.iter().enumerate() {
                if c.runs.remove(id.as_str()).is_some() {
                    st.out.push(i as i64);
                    removed += 1;
                }
            }
        }
    }
    removed
}
