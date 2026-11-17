"""Independent oracle and case generator for the packed-u16 key codec (bud_keys.pruv), keys lane of run.py.

make_cases(seed, count) -> list of (command line, expected output line, category). Python 3, standard library only.

Line protocol (identical to driver_keys.rs; all numbers decimal i64, `|` separates buffers):
  rec P W...                  buffer = W...                      -> rec_end(buffer, P)
  ok KS N W...                buffer = W...                      -> 1 / 0
  unit KS N J W...            buffer = W...                      -> unit
  hash SEED KS N W...         buffer = W...                      -> hash
  eq SA SB N | A... | B...                                       -> 1 / 0
  cmp SA NA SB NB | A... | B...                                  -> -1 / 0 / 1
  pack D S N DLEN | SRC...    dst = DLEN words of 7, src = SRC   -> `STATUS dst...`
  unpack D KS N DLEN | K...   dst = DLEN words of 7, k = K       -> `STATUS dst...`

The oracle models the total semantics of bud_keys.pruv: n clamped into 0..2^30, slice starts into 0..2^40 (hash, eq,
cmp), words outside the buffer read as 0. Hash / eq / cmp are never given a clamped n above a few thousand (the
oracle loops in Python); the n > 2^30 clamp is exercised only by slice_ok / rec_end / unit / pack / unpack.
"""
import random

M = (1 << 64) - 1
MAXU = 1 << 30
MIX_C = -7046029254386353131 & M
FIN_C = -49064778989728563 & M
FILLER = 7


def sg(x):
    x &= M
    return x - (1 << 64) if x >> 63 else x


# ---- oracle ---------------------------------------------------------------------------------------------------------

def words_of(n):
    return (n + 3) >> 2


def enc(units):
    """Units -> canonical packed words (signed i64), 4 per word, big-endian lanes, zero tail."""
    out = []
    for i in range(0, len(units), 4):
        w = 0
        for t in range(4):
            u = units[i + t] if i + t < len(units) else 0
            assert 0 <= u <= 65535
            w |= u << (48 - 16 * t)
        out.append(sg(w))
    return out


def rd(a, i):
    return a[i] if 0 <= i < len(a) else 0


def c30(x):
    return max(0, min(MAXU, x))


def c40(x):
    return max(0, min(1 << 40, x))


def mix(h, v):
    return ((h ^ v) & M) * MIX_C & M


def fin(h):
    h &= M
    a = h ^ (h >> 33)
    b = a * FIN_C & M
    return sg(b ^ (b >> 33))


def o_slice_ok(k, ks, n):
    if n < 0 or n > MAXU or ks < 0:
        return False
    m = words_of(n)
    if m > len(k) - ks:
        return False
    r = n & 3
    if r != 0 and m >= 1 and (k[ks + m - 1] & M) & (M >> (16 * r)):
        return False
    return True


def o_rec_end(k, p):
    if p < 0 or p >= len(k):
        return -1
    n = k[p]
    if not o_slice_ok(k, p + 1, n):
        return -1
    return p + 1 + words_of(n)


def o_unit(k, ks, n, j):
    if n < 0 or n > MAXU or ks < 0 or ks > (1 << 40):
        return 0
    if j < 0 or j >= n:
        return 0
    w = rd(k, ks + (j >> 2)) & M
    return (w >> (48 - 16 * (j & 3))) & 65535


def o_hash(seed, k, ks, n):
    nc = c30(n)
    s0 = c40(ks)
    h = seed & M
    for i in range(words_of(nc)):
        h = mix(h, rd(k, s0 + i) & M)
    return fin(h ^ nc)


def o_eq(a, sa, b, sb, n):
    nc, a0, b0 = c30(n), c40(sa), c40(sb)
    return all(rd(a, a0 + i) == rd(b, b0 + i) for i in range(words_of(nc)))


def o_cmp(a, sa, na, b, sb, nb):
    nac, nbc, a0, b0 = c30(na), c30(nb), c40(sa), c40(sb)
    for i in range(min(words_of(nac), words_of(nbc))):
        x, y = rd(a, a0 + i) & M, rd(b, b0 + i) & M
        if x != y:
            return -1 if x < y else 1
    return -1 if nac < nbc else (1 if nac > nbc else 0)


def o_pack(dst_len, d, src, s, n):
    """-> (status, dst words)."""
    dst = [FILLER] * dst_len
    if n < 0 or n > MAXU or s < 0 or d < 0:
        return 2, dst
    m = words_of(n)
    if s > len(src) - n or d > len(dst) - m:
        return 2, dst
    if any(u < 0 or u > 65535 for u in src[s:s + n]):
        return 1, dst
    dst[d:d + m] = enc(src[s:s + n])
    return 0, dst


def o_unpack(dst_len, d, k, ks, n):
    dst = [FILLER] * dst_len
    if n < 0 or n > MAXU or ks < 0 or d < 0:
        return 2, dst
    m = words_of(n)
    if ks > len(k) - m or d > len(dst) - n:
        return 2, dst
    for j in range(n):
        dst[d + j] = (k[ks + (j >> 2)] & M) >> (48 - 16 * (j & 3)) & 65535
    return 0, dst


def unit_cmp(ua, ub):
    """Reference order by the unit lists (Python list comparison, then length is implied)."""
    return -1 if ua < ub else (1 if ua > ub else 0)


# ---- command builders (line, expected, category) --------------------------------------------------------------------

def ws(a):
    return " ".join(str(x) for x in a)


def c_rec(buf, p, cat):
    return ("rec %d %s" % (p, ws(buf)), str(o_rec_end(buf, p)), cat)


def c_ok(k, ks, n, cat):
    return ("ok %d %d %s" % (ks, n, ws(k)), "1" if o_slice_ok(k, ks, n) else "0", cat)


def c_unit(k, ks, n, j, cat):
    return ("unit %d %d %d %s" % (ks, n, j, ws(k)), str(o_unit(k, ks, n, j)), cat)


def c_hash(seed, k, ks, n, cat):
    return ("hash %d %d %d %s" % (seed, ks, n, ws(k)), str(o_hash(seed, k, ks, n)), cat)


def c_eq(a, sa, b, sb, n, cat):
    return ("eq %d %d %d | %s | %s" % (sa, sb, n, ws(a), ws(b)), "1" if o_eq(a, sa, b, sb, n) else "0", cat)


def c_cmp(a, sa, na, b, sb, nb, cat):
    return ("cmp %d %d %d %d | %s | %s" % (sa, na, sb, nb, ws(a), ws(b)), str(o_cmp(a, sa, na, b, sb, nb)), cat)


def c_pack(dlen, d, src, s, n, cat):
    st, dst = o_pack(dlen, d, src, s, n)
    return ("pack %d %d %d %d | %s" % (d, s, n, dlen, ws(src)), ws([st] + dst), cat)


def c_unpack(dlen, d, k, ks, n, cat):
    st, dst = o_unpack(dlen, d, k, ks, n)
    return ("unpack %d %d %d %d | %s" % (d, ks, n, dlen, ws(k)), ws([st] + dst), cat)


def U(s):
    """Text -> UTF-16 code units (surrogate pairs for astral characters)."""
    b = s.encode("utf-16-be", "surrogatepass")
    return [(b[i] << 8) | b[i + 1] for i in range(0, len(b), 2)]


# ---- fixed controls -------------------------------------------------------------------------------------------------

def fixed_keys():
    ks = [[]]
    for n in range(1, 10):
        ks.append(U("abcdefghi"[:n]))                      # every length 0..9 (tails 0..3 twice, beyond 8)
    ks += [
        U("a\0"), [0], [0, 0x61], U("a\0b"), U("ab\0"), [0, 0, 0, 0, 0], [0x61, 0, 0, 0, 0],   # NUL start/middle/end
        U("é"), [0xFFFF], U("\U0001F600"), U("a\U0001F600b"),                              # BMP, astral pair
        [0xD800], [0xDC00], [0x61, 0xD800], [0xD800, 0x61], [0xDC00, 0xD800],                   # lone surrogates
        [0x8000], [0x41], [0x8000, 0x41], [0x7FFF], [0xFFFF] * 5,                               # unit vs signed order
        U("hello, world 0123"), list(range(0x30, 0x30 + 20)), U("abc"), U("ab"),
    ]
    return ks


def embed(words, rng=None):
    """words inside a larger buffer: returns (buf, start). Fixed padding (arbitrary-looking words, tail garbage)."""
    pre = [-1, 0x1234, 5]
    post = [sg(0xFFFF000000000001), 9]
    return pre + words + post, len(pre)


def fixed_cases():
    out = []
    keys = fixed_keys()
    for u in keys:
        n = len(u)
        w = enc(u)
        buf, st = embed(w)
        # rec: record at 0, embedded, and followed by trailing words
        out.append(c_rec([n] + w, 0, "fixed-rec"))
        out.append(c_rec([8, 9] + [n] + w + [3], 2, "fixed-rec"))
        out.append(c_rec([n] + w, 1, "fixed-rec"))
        out.append(c_ok(w, 0, n, "fixed-ok"))
        out.append(c_ok(buf, st, n, "fixed-ok"))
        out.append(c_ok(buf, st, n + 1 if n % 4 else n + 4, "fixed-ok"))     # one more unit than the key
        for j in range(-1, n + 2):
            out.append(c_unit(buf, st, n, j, "fixed-unit"))
        for seed in (0, 1469598103934665603, -1):
            out.append(c_hash(seed, buf, st, n, "fixed-hash"))
            out.append(c_hash(seed, w, 0, n, "fixed-hash"))
        out.append(c_hash(0, w, 0, n + 8, "fixed-hash"))                      # reads past the buffer as 0
        out.append(c_hash(0, buf, -3, n, "fixed-hash"))                        # negative start clamps to 0
        out.append(c_hash(0, buf, st, -5, "fixed-hash"))                       # negative n clamps to 0
        src = [99] + u + [98]
        out.append(c_pack(len(w) + 2, 1, src, 1, n, "fixed-pack"))
        out.append(c_pack(len(w), 0, src, 1, n, "fixed-pack"))
        out.append(c_unpack(n + 2, 1, buf, st, n, "fixed-unpack"))
        out.append(c_unpack(n, 0, w, 0, n, "fixed-unpack"))
    # every pair: eq with n = min and n = max length, cmp with each key's own length
    for ua in keys:
        for ub in keys:
            wa, wb = enc(ua), enc(ub)
            ba, sa = embed(wa)
            bb, sb = embed(wb)
            for n in sorted({min(len(ua), len(ub)), max(len(ua), len(ub))}):
                out.append(c_eq(ba, sa, bb, sb, n, "fixed-eq"))
            out.append(c_cmp(ba, sa, len(ua), bb, sb, len(ub), "fixed-cmp"))
            r = o_cmp(ba, sa, len(ua), bb, sb, len(ub))
            if o_slice_ok(ba, sa, len(ua)) and o_slice_ok(bb, sb, len(ub)):
                assert r == unit_cmp(ua, ub), (ua, ub)          # word order == code-unit order on canonical keys
    return out


def fixed_named():
    """Named controls from the task list (also covered by the loops above; kept explicit for readers)."""
    out = []
    a, ab0 = enc(U("a")), enc(U("a\0"))
    assert a == ab0                                              # same words, so n decides
    out.append(c_eq(a, 0, ab0, 0, 1, "fixed-eq"))
    out.append(c_cmp(a, 0, 1, ab0, 0, 2, "fixed-cmp"))           # "a" < "a\0"
    out.append(c_hash(0, a, 0, 1, "fixed-hash"))
    out.append(c_hash(0, ab0, 0, 2, "fixed-hash"))               # differs from the line above through n
    out.append(c_cmp(enc([0x8000]), 0, 1, enc([0x41]), 0, 1, "fixed-cmp"))    # unit order > despite signed word order <
    out.append(c_cmp(enc(U("ab")), 0, 2, enc(U("abc")), 0, 3, "fixed-cmp"))
    out.append(c_cmp(enc(U("ab\0")), 0, 3, enc(U("ab")), 0, 2, "fixed-cmp"))
    out.append(c_cmp([], 0, 0, [], 0, 0, "fixed-cmp"))
    out.append(c_eq([], 0, [], 0, 0, "fixed-eq"))
    out.append(c_hash(0, [], 0, 0, "fixed-hash"))
    # canonicality negatives: a nonzero lane past n in the last word, for r = 1, 2, 3 (and a valid lane stays valid)
    for n in (1, 2, 3, 5, 6, 7, 9, 10, 11):
        r = n & 3
        good = enc([0x41] * n)
        for lane in range(r, 4):
            bad = list(good)
            bad[-1] = sg(bad[-1] | (1 << (48 - 16 * lane)))
            out.append(c_ok(bad, 0, n, "fixed-neg"))
            out.append(c_rec([n] + bad, 0, "fixed-neg"))
            out.append(c_ok([7] + bad, 1, n, "fixed-neg"))
        top = list(good)
        top[-1] = sg(top[-1] | (0xFFFF << 48))                 # lane 0 of the last word is a real unit: still valid
        out.append(c_ok(top, 0, n, "fixed-neg"))
    # lengths and ranges
    w4 = enc(U("abcd"))
    for n in (-1, -(1 << 40), 1 << 30, (1 << 30) + 1, (1 << 62), 5, 9, 1000):
        out.append(c_ok(w4, 0, n, "fixed-neg"))
        out.append(c_rec([n] + w4, 0, "fixed-neg"))
        out.append(c_unit(w4, 0, n, 0, "fixed-neg"))
        out.append(c_unit(w4, 0, n, 3, "fixed-neg"))
    for p in (-1, -(1 << 40), 4, 5, 100, 1 << 62):
        out.append(c_rec([4] + w4, p, "fixed-neg"))
    out.append(c_rec([], 0, "fixed-neg"))
    out.append(c_rec([], -1, "fixed-neg"))
    out.append(c_rec([4], 0, "fixed-neg"))                      # n = 4 but no data word
    out.append(c_rec([0], 0, "fixed-neg"))                      # empty key, nothing after the count
    for ks in (-1, 1, 2, 1 << 40, (1 << 62)):
        out.append(c_ok(w4, ks, 4, "fixed-neg"))
        out.append(c_ok(w4, ks, 0, "fixed-neg"))
        out.append(c_unit(w4, ks, 4, 0, "fixed-neg"))
    out.append(c_ok(w4, 1, 0, "fixed-neg"))                     # empty key at the end of the buffer: fine
    out.append(c_ok(w4, 2, 0, "fixed-neg"))                     # one past the end: not
    # pack negatives
    good = [0x41, 0x42, 0x43, 0x44, 0x45]
    for bad_u in (-1, 65536, 1 << 40, -(1 << 40), (1 << 62)):
        for pos in (0, 2, 4):
            src = list(good)
            src[pos] = bad_u
            out.append(c_pack(4, 0, src, 0, 5, "fixed-pack-neg"))
    out.append(c_pack(1, 0, good, 0, 5, "fixed-pack-neg"))      # dst too small
    out.append(c_pack(2, 1, good, 0, 5, "fixed-pack-neg"))      # dst too small by the offset
    out.append(c_pack(2, 0, good, 0, 5, "fixed-pack-neg"))      # exactly 2 words
    out.append(c_pack(2, 0, good, 1, 5, "fixed-pack-neg"))      # n beyond src
    out.append(c_pack(4, 0, good, 0, 6, "fixed-pack-neg"))
    out.append(c_pack(4, 0, good, -1, 3, "fixed-pack-neg"))
    out.append(c_pack(4, -1, good, 0, 3, "fixed-pack-neg"))
    out.append(c_pack(4, 0, good, 0, -1, "fixed-pack-neg"))
    out.append(c_pack(4, 0, good, 0, (1 << 30) + 1, "fixed-pack-neg"))
    out.append(c_pack(4, 0, good, 0, 1 << 62, "fixed-pack-neg"))
    out.append(c_pack(0, 0, [], 0, 0, "fixed-pack-neg"))        # empty into empty
    out.append(c_pack(3, 3, good, 5, 0, "fixed-pack-neg"))      # empty at the ends
    out.append(c_pack(3, 4, good, 0, 0, "fixed-pack-neg"))      # d past dst
    out.append(c_pack(3, 0, good, 6, 0, "fixed-pack-neg"))      # s past src
    out.append(c_pack(4, 0, [0xFFFF, 0, 0xFFFF, 0, 0xFFFF], 0, 5, "fixed-pack-neg"))
    # unpack negatives
    k = enc(good)
    out.append(c_unpack(5, 0, k, 0, 5, "fixed-unpack-neg"))
    out.append(c_unpack(4, 0, k, 0, 5, "fixed-unpack-neg"))     # dst too small
    out.append(c_unpack(5, 1, k, 0, 5, "fixed-unpack-neg"))
    out.append(c_unpack(5, 0, k, 1, 5, "fixed-unpack-neg"))     # key words out of range
    out.append(c_unpack(5, 0, k, 0, 9, "fixed-unpack-neg"))
    out.append(c_unpack(5, 0, k, -1, 3, "fixed-unpack-neg"))
    out.append(c_unpack(5, -1, k, 0, 3, "fixed-unpack-neg"))
    out.append(c_unpack(5, 0, k, 0, -1, "fixed-unpack-neg"))
    out.append(c_unpack(5, 0, k, 0, (1 << 30) + 1, "fixed-unpack-neg"))
    out.append(c_unpack(0, 0, [], 0, 0, "fixed-unpack-neg"))
    out.append(c_unpack(2, 2, k, 2, 0, "fixed-unpack-neg"))
    out.append(c_unpack(2, 3, k, 0, 0, "fixed-unpack-neg"))
    return out


# ---- random cases ---------------------------------------------------------------------------------------------------

def rand_units(rng, lo=0, hi=40):
    n = rng.randint(lo, hi)
    out = []
    for _ in range(n):
        t = rng.random()
        if t < 0.35:
            out.append(rng.randint(32, 126))
        elif t < 0.55:
            out.append(rng.randint(0, 65535))
        elif t < 0.75:
            out.append(rng.randint(0xD800, 0xDFFF))
        elif t < 0.85:
            out.append(0)
        elif t < 0.9:
            out.append(0xFFFF)
        else:
            out.append(rng.choice([0x8000, 0x7FFF, 0x80, 0xFF, 0x100, 0xD83D, 0xDE00]))
    return out


def junk(rng, k):
    return [sg(rng.getrandbits(64)) if rng.random() < 0.7 else rng.randint(-3, 3) for _ in range(k)]


def rand_embed(rng, words):
    pre = junk(rng, rng.randint(0, 5))
    post = junk(rng, rng.randint(0, 5))
    return pre + words + post, len(pre)


def corrupt_tail(rng, units, w):
    """Canonical words w for `units` with garbage ORed into the lanes past n (when there are any)."""
    r = len(units) & 3
    if r == 0 or not w:
        return list(w)
    lane = rng.randint(r, 3)
    bad = list(w)
    bad[-1] = sg(bad[-1] | (rng.randint(1, 65535) << (48 - 16 * lane)))
    return bad


def mutate(rng, u):
    v = list(u)
    if not v:
        return v
    t = rng.random()
    i = rng.randrange(len(v)) if t < 0.7 else len(v) - 1
    v[i] = rng.choice([v[i] ^ 1, rng.randint(0, 65535), 0, 0xFFFF, 0x8000])
    return v


def rand_case(rng, kind):
    u = rand_units(rng)
    n = len(u)
    w = enc(u)
    if rng.random() < 0.2:
        w = corrupt_tail(rng, u, w)                 # tail garbage: canonicality / word-equality differ from content
        tag = "tail"
    else:
        tag = "rand"
    buf, st = rand_embed(rng, w)
    cat = "%s-%s" % (tag, kind)
    if kind == "rec":
        pre = junk(rng, rng.randint(0, 4))
        p = len(pre)
        n_rec = n if rng.random() < 0.8 else n + rng.choice([-1, 1, 2, 5, -n, 100])
        r = pre + [n_rec] + w + junk(rng, rng.randint(0, 4))
        return c_rec(r, p if rng.random() < 0.9 else p + rng.choice([-1, 1, 3, 50, -50]), cat)
    if kind == "ok":
        nn = n if rng.random() < 0.8 else n + rng.choice([-1, 1, 3, 4, 8, -n])
        return c_ok(buf, st if rng.random() < 0.9 else st + rng.choice([-1, 1, 2, 7]), nn, cat)
    if kind == "unit":
        return c_unit(buf, st, n, rng.randint(-2, n + 3), cat)
    if kind == "hash":
        nn = n if rng.random() < 0.8 else rng.choice([n + 1, n + 9, 0, -1])
        return c_hash(sg(rng.getrandbits(64)) if rng.random() < 0.5 else rng.randint(-2, 2), buf,
                      st if rng.random() < 0.9 else rng.choice([-2, st + 1, 0]), nn, cat)
    if kind == "eq":
        v = u if rng.random() < 0.4 else mutate(rng, u)
        wv = enc(v)
        if rng.random() < 0.2:
            wv = corrupt_tail(rng, v, wv)
        b2, s2 = rand_embed(rng, wv)
        nn = n if rng.random() < 0.85 else n + rng.choice([-1, 1, 4, 9])
        return c_eq(buf, st, b2, s2, nn, cat)
    if kind == "cmp":
        t = rng.random()
        if t < 0.3:
            v = mutate(rng, u)
        elif t < 0.5:
            v = u[:rng.randint(0, n)]
        elif t < 0.7:
            v = u + rand_units(rng, 0, 4)
        elif t < 0.8:
            v = list(u)
        else:
            v = rand_units(rng)
        wv = enc(v)
        b2, s2 = rand_embed(rng, wv)
        ba, sa = rand_embed(rng, enc(u))
        line = c_cmp(ba, sa, n, b2, s2, len(v), "rand-cmp")
        assert o_cmp(ba, sa, n, b2, s2, len(v)) == unit_cmp(u, v)
        return line
    if kind == "pack":
        src_u = list(u)
        if src_u and rng.random() < 0.25:
            src_u[rng.randrange(len(src_u))] = rng.choice([-1, 65536, 1 << 40, -(1 << 33), 1 << 62])
        pre = junk(rng, rng.randint(0, 3))
        src = pre + src_u + junk(rng, rng.randint(0, 3))
        s = len(pre) if rng.random() < 0.9 else len(pre) + rng.choice([-1, 1, 2, 30])
        m = words_of(n)
        d = rng.randint(0, 4)
        dlen = d + m + rng.choice([0, 0, 1, 3, -1])
        nn = n if rng.random() < 0.9 else n + rng.choice([-1, 1, 5])
        return c_pack(max(dlen, 0), d, src, s, nn, "rand-pack")
    if kind == "unpack":
        d = rng.randint(0, 4)
        dlen = d + n + rng.choice([0, 0, 1, 3, -1])
        nn = n if rng.random() < 0.9 else n + rng.choice([-1, 1, 5])
        return c_unpack(max(dlen, 0), d, buf, st if rng.random() < 0.9 else st + rng.choice([-1, 1, 3]), nn,
                        "%s-unpack" % tag)
    raise AssertionError(kind)


KINDS = ["rec", "ok", "unit", "hash", "eq", "cmp", "pack", "unpack"]


def make_cases(seed, count):
    rng = random.Random(seed)
    cases = fixed_cases() + fixed_named()
    for i in range(count):
        cases.append(rand_case(rng, KINDS[i % len(KINDS)]))
    return cases


if __name__ == "__main__":
    import sys
    sd = int(sys.argv[1]) if len(sys.argv) > 1 else 1
    ct = int(sys.argv[2]) if len(sys.argv) > 2 else 300
    for line, exp, cat in make_cases(sd, ct):
        print("%s\t%s\t%s" % (line, exp, cat))
