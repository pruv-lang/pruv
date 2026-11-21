use std::collections::HashSet;

fn bytes(text: &[u8], start: i64, len: i64) -> Option<&[u8]> {
    let start = usize::try_from(start).ok()?;
    let len = usize::try_from(len).ok()?;
    text.get(start..start.checked_add(len)?)
}

pub fn begin(cfg: &[i64], text: &[u8]) -> bool {
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
    for reference in cfg[6..18].chunks_exact(2).chain(cfg[19..21].chunks_exact(2)) {
        let (start, len) = (reference[0], reference[1]);
        if (start, len) != (-1, 0) && bytes(text, start, len).is_none() {
            return false;
        }
    }
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

pub fn step(cfg_words_len: usize, ctl: &[i64], event: &[i64]) -> u64 {
    if !(32..=i32::MAX as usize).contains(&cfg_words_len)
        || ctl.len() != 64 || event.len() < 16 || event[0] != 1
    {
        return 28;
    }
    let empty = event == [1, 0, 0, 0, 0, 0, 0, 0, -1, 0, -1, 0, 0, 0, 0, 0];
    match ctl[6] {
        0 if empty => 0,
        1 if !empty && ctl[4] > 0 && event[1] == ctl[4] && event[2] == ctl[5] => 0,
        _ => 28,
    }
}
