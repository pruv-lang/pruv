use std::collections::HashSet;

fn bytes(text: &[u8], start: i64, len: i64) -> Option<&[u8]> {
    let start = usize::try_from(start).ok()?;
    let len = usize::try_from(len).ok()?;
    text.get(start..start.checked_add(len)?)
}

// O(1) header part of begin; the checked step adds the refs-only row loop, like root.pruv cfg_ok(uniq=false)
fn head_ok(cfg: &[i64], text: &[u8]) -> bool {
    let max = i32::MAX as usize;
    if !(32..=max).contains(&cfg.len()) || text.len() > max {
        return false;
    }
    let env = &cfg[32..];
    if env.len() % 4 != 0 || cfg[18] != (env.len() / 4) as i64
        || cfg[0] != 1 || !(0..=5).contains(&cfg[1])
        || !cfg[2..5].iter().all(|x| (1..=i64::from(i32::MAX)).contains(x))
        || !(0..=1).contains(&cfg[5])
        || (cfg[1] == 5 && cfg[19] < 0)
        || (cfg[1] != 5 && cfg[19] != -1)
        || cfg[21..32].iter().any(|&word| word != 0)
    {
        return false;
    }
    cfg[6..18].chunks_exact(2).chain(cfg[19..21].chunks_exact(2))
        .all(|r| (r[0], r[1]) == (-1, 0) || bytes(text, r[0], r[1]).is_some())
}

pub fn begin(cfg: &[i64], text: &[u8]) -> bool {
    if !head_ok(cfg, text) {
        return false;
    }
    let env = &cfg[32..];
    let mut keys = HashSet::with_capacity(env.len() / 4);
    for row in env.chunks_exact(4) {
        let Some(key) = bytes(text, row[0], row[1]) else { return false };
        let Some(value) = bytes(text, row[2], row[3]) else { return false };
        if key.is_empty() || key.contains(&b'=') || key.contains(&0)
            || value.contains(&0) || !keys.insert(key)
        {
            return false;
        }
    }
    true
}

fn ref_ok(s: i64, n: i64, total: i64) -> bool {
    if s == -1 { n == 0 } else { 0 <= s && s <= total && 0 <= n && n <= total - s }
}

// the same 64-word control consistency check as root.pruv ctl_ok_at
fn ctl_ok(a: &[i64], mode: i64, tl: i64) -> bool {
    let (ph, nid, pend, opc, ex) = (a[2], a[3], a[4], a[5], a[6]);
    a[0] == 1 && a[1] == mode
        && (0..=999).contains(&ph) && (1..=1_000_000_000).contains(&nid)
        && (0..nid).contains(&pend) && (0..=15).contains(&opc) && (0..=2).contains(&ex)
        && (pend == 0) == (opc == 0) && (ex == 1) == (pend != 0) && (ex == 2) == (ph == 999)
        && (0..=999).contains(&a[7]) && a[8..13].iter().all(|&w| w >= -1)
        && a[13..16].iter().all(|&w| (0..=1).contains(&w))
        && a[16..20].iter().all(|&w| w >= 0) && a[19] <= 2
        && a[20] >= 0 && a[21] >= 0 && a[20] <= a[21]
        && (0..=1).contains(&a[22]) && (0..=5).contains(&a[23])
        && (0..=1).contains(&a[38]) && a[39] >= 0
        && a[24..38].chunks_exact(2).all(|r| ref_ok(r[0], r[1], tl))
        && a[40..64].iter().all(|&w| w == 0)
}

// done packet with the InternalInvariant result at `stage`: the same 116 words as the Pruv reject packet
fn reject(ctl: &[i64], mode: i64, stage: i64) -> Vec<i64> {
    let mut p = vec![0i64; 116];
    p[..4].copy_from_slice(&[1329812273, 1, 1, 64]);
    p[8] = 40;
    if ctl.len() == 64 {
        p[12..76].copy_from_slice(ctl);
    } else {
        p[13] = mode;
        p[15] = 1;
        for k in [8, 9, 10, 11, 12, 24, 26, 28, 30, 32, 34, 36] {
            p[12 + k] = -1;
        }
    }
    p[12] = 1;
    p[14] = 999;
    p[16] = 0;
    p[17] = 0;
    p[18] = 2;
    let m = if (0..=5).contains(&p[13]) { p[13] } else { 0 };
    p[76..80].copy_from_slice(&[1329812017, 1, 1, m]);
    for k in [10, 12, 14, 16, 23, 25, 27] {
        p[76 + k] = -1;
    }
    p[95..98].copy_from_slice(&[stage, 3, 6]);
    p
}

// a step on the reentry/rejection paths: the same checks as root.pruv step_body before routing and the same reject
// packet; an accepted event returns an empty Vec (routing is not part of this lane)
pub fn step(checked: bool, cfg: &[i64], text: &[u8], ctl: &[i64], event: &[i64]) -> Vec<i64> {
    if checked && !(head_ok(cfg, text) && cfg[32..].chunks_exact(4).all(|r| {
        r[0] >= 0 && r[2] >= 0 && r[1] >= 1 && ref_ok(r[0], r[1], text.len() as i64) && ref_ok(r[2], r[3], text.len() as i64)
    })) {
        return reject(&[], if (0..=5).contains(&cfg.get(1).copied().unwrap_or(0)) { cfg[1] } else { 0 }, 0);
    }
    let mode = cfg[1];
    if ctl.len() != 64 || !ctl_ok(ctl, mode, 0) {
        return reject(&[], mode, 28);
    }
    let empty = event == [1, 0, 0, 0, 0, 0, 0, 0, -1, 0, -1, 0, 0, 0, 0, 0];
    let shape = event.len() >= 16 && event[0] == 1 && event[1] >= 0 && (0..=15).contains(&event[2])
        && (0..=1).contains(&event[3]) && (0..=5).contains(&event[4]) && event[5] >= 0
        && event[6] >= 0 && event.len() as i64 - 16 == event[6] && event[7] == 0
        && event[12..16].iter().all(|&w| w == 0)
        && ref_ok(event[8], event[9], 0) && ref_ok(event[10], event[11], 0)
        && !(event[3] == 1 && (event[4] == 0 || event[6] != 0));
    if !shape {
        return reject(ctl, mode, 28);
    }
    if ctl[6] == 2 || ctl[2] == 999 {
        return reject(&[], mode, 28);
    }
    let ok = match ctl[6] {
        0 => empty,
        _ => !empty && ctl[4] > 0 && event[1] == ctl[4] && event[2] == ctl[5],
    };
    if ok { Vec::new() } else { reject(ctl, mode, 28) }
}
