// Hand Rust baseline for `bench/l3/lib/ent_accounts.pruv` (Rust-shape reset step 3.8).
// Account ids, accountHome and op 5 account_home (reference/backend-accounts.ts), and the input check for ops 1, 5
// and 6, which `ent_packet::input_check` does not cover. Op 5 never awaits: its begin builds the done packet.
// Used by `ent_canon` (op 6), `ent_sources` (op 1) and `entry_api` (op 5).
// Check: rustc --edition 2021 --test ent_accounts.rs -o /tmp/ent_accounts_hand && /tmp/ent_accounts_hand

#![allow(dead_code)]

#[cfg(not(shared_ctx))]
#[path = "ent_state.rs"]
pub mod ent_state;
#[cfg(shared_ctx)]
pub use crate::ent_state;
#[cfg(not(shared_ctx))]
#[path = "ent_path.rs"]
pub mod ent_path;
#[cfg(shared_ctx)]
pub use crate::ent_path;
#[cfg(not(shared_ctx))]
#[path = "ent_u16.rs"]
pub mod ent_u16;
#[cfg(shared_ctx)]
pub use crate::ent_u16;

use ent_state::ent_packet::{boundary, mk_packet, put_vs, sec_len, sec_start, Check, ANY, STR};
use ent_state::ent_value::{decode_span, encode_pool, units_in, Pool, Value};
use ent_u16::lit;

/// tag mask of an env slot: undefined, null, string, absent
const ENV: i64 = 1091;

fn rd(w: &[i64], i: usize) -> i64 {
    w.get(i).copied().unwrap_or(0)
}

/// /^acc_[a-f0-9]{16}$/ : exactly 20 units, so a trailing line terminator is rejected by the length
pub fn valid_account_id(w: &[i64], p: usize) -> bool {
    let Value::Str(s) = Value::parse([rd(w, p), rd(w, p + 1), rd(w, p + 2)]) else { return false };
    let pool = Pool::at(w, sec_start(w, 0));
    if units_in(pool, s.off, s.len) != 20 {
        return false;
    }
    let id = decode_span(pool, s.off, s.len);
    id[..4] == [97, 99, 99, 95] && id[4..].iter().all(|&u| matches!(u, 48..=57 | 97..=102))
}

/// join(dataDir, 'engine-homes', engine, id) for a valid id
pub fn account_home(data_dir: &[u16], engine: &[u16], id: &[u16]) -> Vec<u16> {
    ent_path::join(&[data_dir, &lit("engine-homes"), engine, id])
}

/// Input check for ops 1, 5, 6 (kind 0, envelope already ok): the pool prefix length the record references.
pub fn io_check(w: &[i64]) -> Option<i64> {
    let mut c = Check::new(w);
    let r0 = c.start(1);
    match (rd(w, 5), sec_len(w, 1)) {
        (1, 24) => {
            c.slot(r0, STR, 0);
            for k in 1..8 {
                c.slot(r0 + 3 * k, ENV, 0);
            }
        }
        (5, 9) => {
            c.slot(r0, STR, 0);
            c.slot(r0 + 3, STR, 0);
            c.slot(r0 + 6, ANY, 0);
        }
        (6, 9) => {
            c.slot(r0, STR, 0);
            c.slot(r0 + 3, ANY, 0);
            c.slot(r0 + 6, ANY, 0);
        }
        _ => return None,
    }
    c.finish()
}

/// units of the string slot at word p of packet w (the slot is known to be a string)
fn str_units(w: &[i64], p: usize) -> Vec<u16> {
    match Value::parse([rd(w, p), rd(w, p + 1), rd(w, p + 2)]) {
        Value::Str(s) => decode_span(Pool::at(w, sec_start(w, 0)), s.off, s.len),
        _ => Vec::new(),
    }
}

/// op 5: record dataDir {6}, engine {6}, id any. Done: [0, home] or the wire error invalidId [3, 0]. Never awaits.
pub fn account_home_begin(input: &[i64]) -> Vec<i64> {
    let id = rd(input, 2);
    if io_check(input).is_none() {
        return boundary(5, id, 0);
    }
    let r0 = sec_start(input, 1);
    if !valid_account_id(input, r0 + 6) {
        return mk_packet(3, id, 0, 5, &[], &[3, 0], &[], &[], &[], 0);
    }
    let home = account_home(&str_units(input, r0), &str_units(input, r0 + 3), &str_units(input, r0 + 6));
    let mut rec = vec![0; 4];
    let pool = encode_pool(&home);
    put_vs(&mut rec, 1, 6, 0, pool.nbytes as i64);
    mk_packet(3, id, 0, 5, &pool.view_words(), &rec, &[], &[], &[], 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Vec<u16> {
        lit(s)
    }

    fn pool(s: &str) -> Vec<i64> {
        encode_pool(&u(s)).view_words()
    }

    /// A packet with exactly these sections (pool, record): every later section must be claimed by a slot.
    fn pkt(kind: i64, id: i64, op: i64, secs: &[&[i64]]) -> Vec<i64> {
        let body: usize = secs.iter().map(|s| s.len()).sum();
        let mut w = vec![1, kind, id, 0, (2 + secs.len() + body) as i64, op, secs.len() as i64];
        w.extend(secs.iter().map(|s| s.len() as i64));
        secs.iter().for_each(|s| w.extend_from_slice(s));
        w
    }

    fn rec(w: &[i64]) -> &[i64] {
        let s = sec_start(w, 1);
        &w[s..s + sec_len(w, 1)]
    }

    #[test]
    fn account_ids() {
        let w = pkt(0, 7, 6, &[&pool("acc_0123456789abcdef"), &[6, 0, 20]]);
        assert!(valid_account_id(&w, sec_start(&w, 1)));
        let w = pkt(0, 7, 6, &[&pool("acc_0123456789abcdeF"), &[6, 0, 20]]);
        assert!(!valid_account_id(&w, sec_start(&w, 1)));
        let w = pkt(0, 7, 6, &[&pool("acc_0123456789abcde\n"), &[6, 0, 20]]);
        assert!(!valid_account_id(&w, sec_start(&w, 1)));
        let w = pkt(0, 7, 6, &[&pool("acc_0123456789abcdef0"), &[6, 0, 21]]);
        assert!(!valid_account_id(&w, sec_start(&w, 1)));
        let w = pkt(0, 7, 6, &[&pool("x"), &[5, 0, 0]]);
        assert!(!valid_account_id(&w, sec_start(&w, 1)));
    }

    #[test]
    fn home_join() {
        assert_eq!(account_home(&u("/d"), &u("codex"), &u("acc_1")), u("/d/engine-homes/codex/acc_1"));
        assert_eq!(account_home(&u("/d/"), &u("claude"), &u("acc_1")), u("/d/engine-homes/claude/acc_1"));
    }

    #[test]
    fn io_checks() {
        let p = pool("/dclaude");
        assert_eq!(io_check(&pkt(0, 4, 5, &[&p, &[6, 0, 2, 6, 2, 6, 0, 0, 0]])), Some(8));
        assert_eq!(io_check(&pkt(0, 4, 5, &[&p, &[6, 0, 2, 6, 2, 6]])), None);
        assert_eq!(io_check(&pkt(0, 4, 6, &[&p, &[6, 0, 2, 0, 0, 0, 0, 0, 0]])), Some(2));
        assert_eq!(io_check(&pkt(0, 4, 5, &[&p, &[6, 0, 2, 0, 0, 0, 0, 0, 0]])), None);
        assert_eq!(io_check(&pkt(0, 4, 2, &[&p, &[6, 0, 2, 0, 0, 0, 0, 0, 0]])), None);
        let mut r1 = vec![6, 0, 2];
        r1.extend([0; 21]);
        assert_eq!(io_check(&pkt(0, 4, 1, &[&p, &r1])), Some(2));
    }

    #[test]
    fn account_home_done() {
        let id = "acc_0123456789abcdef";
        let p = pool(&format!("/dclaude{id}"));
        let out = account_home_begin(&pkt(0, 4, 5, &[&p, &[6, 0, 2, 6, 2, 6, 6, 8, 20]]));
        assert_eq!(rd(&out, 1), 3);
        let r = rec(&out);
        assert_eq!(&r[..2], &[0, 6]);
        let home = decode_span(Pool::at(&out, sec_start(&out, 0)), r[2] as usize, r[3] as usize);
        assert_eq!(String::from_utf16(&home).unwrap(), format!("/d/engine-homes/claude/{id}"));
        let bad = account_home_begin(&pkt(0, 4, 5, &[&p, &[6, 0, 2, 6, 2, 6, 6, 8, 19]]));
        assert_eq!(rec(&bad), &[3, 0]);
        let short = pkt(0, 4, 5, &[&p, &[6, 0, 2, 6, 2, 6]]);
        assert_eq!(account_home_begin(&short), boundary(5, 4, 0));
    }
}
