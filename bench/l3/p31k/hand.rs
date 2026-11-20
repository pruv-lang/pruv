// Hand Rust for bench/l3/p31k/p31k.pruv (shape of bench/l3/budget/m10b_test/hand_m10b.rs:115-185).
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct Session {
    pub used: i64,
    pub stopped: Vec<String>,
    pub runs: HashMap<String, bool>,
}

#[derive(Clone, Debug)]
pub struct St {
    pub sessions: HashMap<String, Session>,
}

pub fn charge(st: &mut St, sid: &str, x: i64) -> i64 {
    match st.sessions.get_mut(sid) {
        Some(r) => {
            if r.used <= 1_000_000_000_000 {
                r.used += x;
                0
            } else {
                2
            }
        }
        None => 1,
    }
}

pub fn halt(st: &mut St, sid: &str, name: &str) -> i64 {
    match st.sessions.get_mut(sid) {
        Some(r) => {
            if (r.stopped.len() as i64) < 1_000_000 {
                let gone = r.runs.remove(name).is_some();
                r.stopped.push(name.to_string());
                if gone { 0 } else { 3 }
            } else {
                2
            }
        }
        None => 1,
    }
}
