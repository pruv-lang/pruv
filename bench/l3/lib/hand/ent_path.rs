// Hand Rust baseline for `bench/l3/lib/ent_path.pruv` (Rust-shape reset step 3.1): Node v26.4.0 `path.posix`
// (normalize, join, dirname, basename, one-argument resolve split around `process.cwd()`) over UTF-16 units,
// written the way a Rust programmer ports it: `&[u16]` in, `Vec<u16>` out, one growing buffer per result.
// The Pruv module keeps one unit per `i64` word and `i64` indexes (gap U, bench/l3/lib/RESHAPE.md).
// Check: rustc --edition 2021 --test ent_path.rs -o /tmp/ent_path_hand && /tmp/ent_path_hand

const SLASH: u16 = b'/' as u16;
const DOT: u16 = b'.' as u16;

pub fn is_abs(p: &[u16]) -> bool {
    p.first() == Some(&SLASH)
}

/// Node internal `normalizeString(path, allowAboveRoot, '/', isPosixPathSeparator)`.
pub fn normalize_string(p: &[u16], allow_above_root: bool) -> Vec<u16> {
    let mut res: Vec<u16> = Vec::with_capacity(p.len());
    let mut last_segment_length = 0usize;
    let mut last_slash: isize = -1;
    let mut dots: i32 = 0;
    let mut code = 0u16;
    for i in 0..=p.len() {
        if i < p.len() {
            code = p[i];
        } else if code == SLASH {
            break;
        } else {
            code = SLASH;
        }
        if code == SLASH {
            if last_slash == i as isize - 1 || dots == 1 {
                // NOOP
            } else if dots == 2 {
                let n = res.len();
                if n < 2 || last_segment_length != 2 || res[n - 1] != DOT || res[n - 2] != DOT {
                    if n > 2 {
                        match res.iter().rposition(|&c| c == SLASH) {
                            None => {
                                res.clear();
                                last_segment_length = 0;
                            }
                            Some(li) => {
                                res.truncate(li);
                                last_segment_length = match res.iter().rposition(|&c| c == SLASH) {
                                    None => li,
                                    Some(x) => li - 1 - x,
                                };
                            }
                        }
                        last_slash = i as isize;
                        dots = 0;
                        continue;
                    } else if n != 0 {
                        res.clear();
                        last_segment_length = 0;
                        last_slash = i as isize;
                        dots = 0;
                        continue;
                    }
                }
                if allow_above_root {
                    if !res.is_empty() {
                        res.push(SLASH);
                    }
                    res.extend_from_slice(&[DOT, DOT]);
                    last_segment_length = 2;
                }
            } else {
                if !res.is_empty() {
                    res.push(SLASH);
                }
                let from = (last_slash + 1) as usize;
                res.extend_from_slice(&p[from..i]);
                last_segment_length = i - from;
            }
            last_slash = i as isize;
            dots = 0;
        } else if code == DOT && dots != -1 {
            dots += 1;
        } else {
            dots = -1;
        }
    }
    res
}

pub fn normalize(p: &[u16]) -> Vec<u16> {
    if p.is_empty() {
        return vec![DOT];
    }
    let absolute = is_abs(p);
    let trailing = p[p.len() - 1] == SLASH;
    let r = normalize_string(p, !absolute);
    if r.is_empty() {
        return match (absolute, trailing) {
            (true, _) => vec![SLASH],
            (false, true) => vec![DOT, SLASH],
            (false, false) => vec![DOT],
        };
    }
    let mut out = Vec::with_capacity(r.len() + 2);
    if absolute {
        out.push(SLASH);
    }
    out.extend_from_slice(&r);
    if trailing {
        out.push(SLASH);
    }
    out
}

/// Node `join`: the non-empty arguments joined with '/', then normalized.
fn push_part(joined: &mut Vec<u16>, part: &[u16]) {
    if part.is_empty() {
        return;
    }
    if !joined.is_empty() {
        joined.push(SLASH);
    }
    joined.extend_from_slice(part);
}

pub fn join(parts: &[&[u16]]) -> Vec<u16> {
    let mut joined = Vec::with_capacity(parts.iter().map(|p| p.len() + 1).sum());
    for part in parts {
        push_part(&mut joined, part);
    }
    normalize(&joined)
}

pub fn dirname(p: &[u16]) -> Vec<u16> {
    if p.is_empty() {
        return vec![DOT];
    }
    let root = p[0] == SLASH;
    let mut end = None;
    let mut matched_slash = true;
    for i in (1..p.len()).rev() {
        if p[i] == SLASH {
            if !matched_slash {
                end = Some(i);
                break;
            }
        } else {
            matched_slash = false;
        }
    }
    match end {
        None if root => vec![SLASH],
        None => vec![DOT],
        Some(1) if root => vec![SLASH, SLASH],
        Some(e) => p[..e].to_vec(),
    }
}

pub fn basename(p: &[u16]) -> Vec<u16> {
    let mut start = 0;
    let mut end = None;
    let mut matched_slash = true;
    for i in (0..p.len()).rev() {
        if p[i] == SLASH {
            if !matched_slash {
                start = i + 1;
                break;
            }
        } else if end.is_none() {
            matched_slash = false;
            end = Some(i + 1);
        }
    }
    match end {
        None => Vec::new(),
        Some(e) => p[start..e].to_vec(),
    }
}

// ---- resolve(p) with one argument, split around process.cwd() ----

fn is_fast(p: &[u16]) -> bool {
    p.is_empty() || p == [DOT]
}

pub fn resolve_need(p: &[u16], ncwd: i64, cwd1: &[u16]) -> bool {
    if is_abs(p) {
        return false;
    }
    match ncwd {
        n if n <= 0 => true,
        1 => is_fast(p) && !is_abs(cwd1),
        _ => false,
    }
}

pub fn resolve_finish(p: &[u16], ncwd: i64, cwd1: &[u16], cwd2: &[u16]) -> Vec<u16> {
    let mut resolved = Vec::new();
    let absolute;
    if is_abs(p) {
        resolved.reserve(p.len() + 1);
        resolved.extend_from_slice(p);
        resolved.push(SLASH);
        absolute = true;
    } else {
        if ncwd == 1 && is_fast(p) && is_abs(cwd1) {
            return cwd1.to_vec();
        }
        let c = if ncwd >= 2 { cwd2 } else { cwd1 };
        resolved.reserve(c.len() + p.len() + 2);
        resolved.extend_from_slice(c);
        resolved.push(SLASH);
        if !p.is_empty() {
            resolved.extend_from_slice(p);
            resolved.push(SLASH);
        }
        absolute = is_abs(c);
    }
    let r = normalize_string(&resolved, !absolute);
    if absolute {
        let mut out = Vec::with_capacity(r.len() + 1);
        out.push(SLASH);
        out.extend_from_slice(&r);
        return out;
    }
    if r.is_empty() {
        return vec![DOT];
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn node_cases() {
        for (i, o) in [("", "."), ("/", "/"), ("a//b/../c/", "a/c/"), ("../../x", "../../x"), ("/../a/./b", "/a/b"),
                       ("a/..", "."), ("./", "./"), ("a/b/../../..", ".."), ("//a", "/a"), ("x/../../y/..", "..")] {
            assert_eq!(normalize(&u(i)), u(o), "normalize {i}");
        }
        assert_eq!(join(&[&u("/h"), &u(""), &u(".local"), &u("share")]), u("/h/.local/share"));
        assert_eq!(join(&[&u(""), &u("")]), u("."));
        for (i, o) in [("", "."), ("/", "/"), ("//", "/"), ("/a", "/"), ("a/b/", "a"), ("//a", "//"), ("a", ".")] {
            assert_eq!(dirname(&u(i)), u(o), "dirname {i}");
        }
        for (i, o) in [("", ""), ("/", ""), ("/a/b/", "b"), ("a", "a"), ("x//", "x")] {
            assert_eq!(basename(&u(i)), u(o), "basename {i}");
        }
        assert_eq!(resolve_finish(&u("/a/../b"), 0, &[], &[]), u("/b"));
        assert_eq!(resolve_finish(&u("."), 1, &u("/w"), &[]), u("/w"));
        assert_eq!(resolve_finish(&u("x"), 1, &u("/w/v"), &[]), u("/w/v/x"));
        assert_eq!(resolve_finish(&u(""), 2, &u("rel"), &u("r2")), u("r2"));
        assert_eq!(resolve_finish(&u("x/.."), 1, &u(""), &[]), u("."));
    }
}
