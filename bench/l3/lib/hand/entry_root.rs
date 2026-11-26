// Hand Rust baseline for `bench/l3/entry/root.pruv` (Rust-shape reset step 3.8).
// The entry crate root: it declares every entry module once (one `ent_state`, so one `Ctx` type runs every item;
// the item modules use it under cfg `shared_ctx`) and exports the four host functions. The host functions are thin
// forwarders to `entry_api`; the root adds the wire boundary (DB8): section 0 crosses it as the packed WTF-8 pool,
// so begin and step check the wire first (`env_check_wire`), and index_step reads its kept input through `poison`
// when the input's pool is not a valid wire pool.
// Check: rustc --edition 2021 --cfg shared_ctx --test entry_root.rs -o /tmp/entry_root_hand && /tmp/entry_root_hand

#![allow(dead_code)]

#[path = "ent_state.rs"]
pub mod ent_state;
#[path = "ent_path.rs"]
pub mod ent_path;
#[path = "ent_u16.rs"]
pub mod ent_u16;
#[path = "ent_dispatch.rs"]
pub mod ent_dispatch;
#[path = "ent_bound.rs"]
pub mod ent_bound;
#[path = "ent_canon.rs"]
pub mod ent_canon;
#[path = "ent_sources.rs"]
pub mod ent_sources;
#[path = "ent_accounts.rs"]
pub mod ent_accounts;
#[path = "entry_api.rs"]
pub mod entry_api;

use ent_state::ent_packet::ent_wire::{poison, wire_in};
use ent_state::ent_packet::env_check_wire;

pub fn entry_begin(input: &[i64]) -> Vec<i64> {
    entry_api::entry_begin_with_env(input, env_check_wire(input))
}

pub fn entry_step(prev: &[i64], completion: &[i64]) -> Vec<i64> {
    entry_api::entry_step_at(prev, completion, true)
}

pub fn index_begin(progress: &mut [i64], input: &[i64]) -> Vec<i64> {
    entry_api::index_begin(progress, input)
}

pub fn index_step(progress: &mut [i64], input: &[i64], prev: &[i64], emitted: &[i64], completion: &[i64]) -> Vec<i64> {
    if !wire_in(input) {
        return entry_api::index_step(progress, &poison(input), prev, emitted, completion);
    }
    entry_api::index_step(progress, input, prev, emitted, completion)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ent_state::ent_packet::boundary;
    use ent_state::ent_value::encode_pool;

    fn pool(s: &str) -> Vec<i64> {
        encode_pool(&s.encode_utf16().collect::<Vec<u16>>()).view_words()
    }

    fn pkt(kind: i64, id: i64, op: i64, secs: &[&[i64]]) -> Vec<i64> {
        let body: usize = secs.iter().map(|s| s.len()).sum();
        let mut w = vec![1, kind, id, 0, (2 + secs.len() + body) as i64, op, secs.len() as i64];
        w.extend(secs.iter().map(|s| s.len() as i64));
        secs.iter().for_each(|s| w.extend_from_slice(s));
        w
    }

    /// op 5 on a wire pool: the root answers exactly as entry_api.
    #[test]
    fn begin_forwards() {
        let id = "acc_0123456789abcdef";
        let w = pkt(0, 4, 5, &[&pool(&format!("/dclaude{id}")), &[6, 0, 2, 6, 2, 6, 6, 8, 20]]);
        assert!(wire_in(&w));
        assert_eq!(entry_begin(&w), entry_api::entry_begin(&w));
    }

    /// A pool byte 0xff is no WTF-8: the wire check rejects the packet (InvalidPacket, boundary code 0).
    #[test]
    fn begin_rejects_bad_wire() {
        let w = pkt(0, 4, 5, &[&[1, 1, 0xff << 48], &[6, 0, 1, 6, 0, 0, 6, 0, 0]]);
        assert!(!wire_in(&w));
        assert_eq!(entry_begin(&w), boundary(5, 4, 0));
    }

    /// index_step reads a kept input with a bad wire pool through its poison.
    #[test]
    fn index_step_poisons() {
        let w = pkt(0, 4, 9, &[&[1, 1, 0xff << 48], &[0; 3]]);
        let (mut p1, mut p2) = (vec![0i64; 4], vec![0i64; 4]);
        let e = boundary(9, 4, 0);
        let a = index_step(&mut p1, &w, &e, &e, &e);
        let b = entry_api::index_step(&mut p2, &poison(&w), &e, &e, &e);
        assert_eq!((a, p1), (b, p2));
    }
}
