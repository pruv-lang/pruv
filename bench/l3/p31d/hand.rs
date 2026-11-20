// Hand Rust for bench/l3/p31d/p31d.pruv.
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub struct Budget {
    pub limit_ms: i64,
    pub active_ms: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub since_ms: i64,
    pub armed: bool,
}

#[derive(Clone, Debug)]
pub struct St {
    pub sessions: HashMap<String, Budget>,
    pub clocks: HashMap<String, Clock>,
}

#[derive(Clone, Copy, Debug)]
pub struct Effect {
    pub tag: i64,
    pub argument: i64,
}

#[derive(Clone, Debug)]
pub struct Reply {
    pub status: i64,
    pub effects: Vec<Effect>,
}

pub fn refresh(st: &mut St, sid: &str, bid: &str, now: i64) -> i64 {
    match st.sessions.get_mut(sid) {
        Some(budget) => match st.clocks.get_mut(bid) {
            Some(clock) => {
                if 0 <= clock.since_ms && clock.since_ms <= now && now <= 1_000_000
                    && 0 <= budget.active_ms && budget.active_ms <= 1_000_000_000_000
                {
                    budget.active_ms += now - clock.since_ms;
                }
                clock.since_ms = now;
                clock.armed = budget.active_ms < budget.limit_ms;
                0
            }
            None => 1,
        },
        None => 1,
    }
}

pub fn poll(st: &mut St, sid: &str) -> Reply {
    let mut effects = Vec::new();
    let status = match st.sessions.get_mut(sid) {
        Some(budget) => {
            if budget.active_ms >= budget.limit_ms {
                effects.push(Effect { tag: 1, argument: budget.active_ms });
            }
            0
        }
        None => 1,
    };
    Reply { status, effects }
}
