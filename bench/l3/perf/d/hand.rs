// Leg D hand-written Rust: the same algorithm and flat layouts as kernel.pruv (plain safe indexing).

pub fn bound_int(sess: &[i64], k: usize, items: &[i64]) -> Vec<i64> {
    let w = 1 + 2 * k;
    items
        .chunks_exact(2)
        .map(|it| {
            let (e, t) = (it[0], it[1]);
            for (s, row) in sess.chunks_exact(w).enumerate() {
                if row[0] != 0 {
                    continue;
                }
                for c in 0..k {
                    if row[1 + 2 * c] == e && row[2 + 2 * c] == t {
                        return s as i64;
                    }
                }
            }
            -1
        })
        .collect()
}

pub fn bound_str(sess: &[i64], k: usize, items: &[i64], pool: &[i64]) -> Vec<i64> {
    let w = 1 + 3 * k;
    let range = |o: i64, l: i64| -> Option<&[i64]> { pool.get(usize::try_from(o).ok()?..usize::try_from(o).ok()?.checked_add(usize::try_from(l).ok()?)?) };
    items
        .chunks_exact(3)
        .map(|it| {
            let (e, needle) = (it[0], range(it[1], it[2]));
            for (s, row) in sess.chunks_exact(w).enumerate() {
                if row[0] != 0 {
                    continue;
                }
                for c in 0..k {
                    let p = 1 + 3 * c;
                    if row[p] == e && row[p + 2] == it[2] && needle.is_some() && range(row[p + 1], row[p + 2]) == needle {
                        return s as i64;
                    }
                }
            }
            -1
        })
        .collect()
}
