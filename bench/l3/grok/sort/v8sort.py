"""Port of V8 Array.prototype.sort (third_party/v8/builtins/array-sort.tq, TimSort).
cmp(a, b) returns a number; only its sign (< 0) matters."""
MIN_GALLOP = 7

def sort(a, cmp):
    w = list(a)
    n = len(w)
    if n < 2:
        return w
    st = {'mg': MIN_GALLOP}
    runs = []

    def minrun(n):
        r = 0
        while n >= 64:
            r |= n & 1
            n >>= 1
        return n + r

    def count_run(lo, hi):
        low = lo + 1
        if low == hi:
            return 1
        rl = 2
        prev = w[low]
        desc = cmp(w[low], w[low - 1]) < 0
        for i in range(low + 1, hi):
            o = cmp(w[i], prev)
            if desc:
                if o >= 0: break
            else:
                if o < 0: break
            prev = w[i]
            rl += 1
        if desc:
            w[lo:lo + rl] = w[lo:lo + rl][::-1]
        return rl

    def bis(low, start, high):
        if low == start:
            start += 1
        while start < high:
            left, right = low, start
            p = w[start]
            while left < right:
                mid = left + ((right - left) >> 1)
                if cmp(p, w[mid]) < 0: right = mid
                else: left = mid + 1
            w[left + 1:start + 1] = w[left:start]
            w[left] = p
            start += 1

    def gallop_left(arr, key, base, length, hint):
        last, off = 0, 1
        if cmp(arr[base + hint], key) < 0:
            maxo = length - hint
            while off < maxo:
                if cmp(arr[base + hint + off], key) >= 0: break
                last = off; off = (off << 1) + 1
            off = min(off, maxo)
            last += hint; off += hint
        else:
            maxo = hint + 1
            while off < maxo:
                if cmp(arr[base + hint - off], key) < 0: break
                last = off; off = (off << 1) + 1
            off = min(off, maxo)
            last, off = hint - off, hint - last
        last += 1
        while last < off:
            m = last + ((off - last) >> 1)
            if cmp(arr[base + m], key) < 0: last = m + 1
            else: off = m
        return off

    def gallop_right(arr, key, base, length, hint):
        last, off = 0, 1
        if cmp(key, arr[base + hint]) < 0:
            maxo = hint + 1
            while off < maxo:
                if cmp(key, arr[base + hint - off]) >= 0: break
                last = off; off = (off << 1) + 1
            off = min(off, maxo)
            last, off = hint - off, hint - last
        else:
            maxo = length - hint
            while off < maxo:
                if cmp(key, arr[base + hint + off]) < 0: break
                last = off; off = (off << 1) + 1
            off = min(off, maxo)
            last += hint; off += hint
        last += 1
        while last < off:
            m = last + ((off - last) >> 1)
            if cmp(key, arr[base + m]) < 0: off = m
            else: last = m + 1
        return off

    def merge_low(ba, la, bb, lb):
        t = w[ba:ba + la]
        dest, ct, cb = ba, 0, bb
        w[dest] = w[cb]; dest += 1; cb += 1
        lb -= 1
        mode = None
        if lb == 0: mode = 'ok'
        elif la == 1: mode = 'copyb'
        else:
            mg = st['mg']
            while mode is None:
                wa = wb = 0
                while True:
                    if cmp(w[cb], t[ct]) < 0:
                        w[dest] = w[cb]; dest += 1; cb += 1
                        wb += 1; lb -= 1; wa = 0
                        if lb == 0: mode = 'ok'; break
                        if wb >= mg: break
                    else:
                        w[dest] = t[ct]; dest += 1; ct += 1
                        wa += 1; la -= 1; wb = 0
                        if la == 1: mode = 'copyb'; break
                        if wa >= mg: break
                if mode: break
                mg += 1
                first = True
                while wa >= MIN_GALLOP or wb >= MIN_GALLOP or first:
                    first = False
                    mg = max(1, mg - 1); st['mg'] = mg
                    k = gallop_right(t, w[cb], ct, la, 0)
                    wa = k
                    if wa > 0:
                        w[dest:dest + wa] = t[ct:ct + wa]
                        dest += wa; ct += wa; la -= wa
                        if la == 1: mode = 'copyb'; break
                        if la == 0: mode = 'ok'; break
                    w[dest] = w[cb]; dest += 1; cb += 1
                    lb -= 1
                    if lb == 0: mode = 'ok'; break
                    wb = gallop_left(w, t[ct], cb, lb, 0)
                    if wb > 0:
                        w[dest:dest + wb] = w[cb:cb + wb]
                        dest += wb; cb += wb; lb -= wb
                        if lb == 0: mode = 'ok'; break
                    w[dest] = t[ct]; dest += 1; ct += 1
                    la -= 1
                    if la == 1: mode = 'copyb'; break
                if mode: break
                mg += 1; st['mg'] = mg
        if mode == 'ok':
            if la > 0: w[dest:dest + la] = t[ct:ct + la]
        else:
            w[dest:dest + lb] = w[cb:cb + lb]
            w[dest + lb] = t[ct]

    def merge_high(ba, la, bb, lb):
        t = w[bb:bb + lb]
        dest, ct, ca = bb + lb - 1, lb - 1, ba + la - 1
        w[dest] = w[ca]; dest -= 1; ca -= 1
        la -= 1
        mode = None
        if la == 0: mode = 'ok'
        elif lb == 1: mode = 'copya'
        else:
            mg = st['mg']
            while mode is None:
                wa = wb = 0
                while True:
                    if cmp(t[ct], w[ca]) < 0:
                        w[dest] = w[ca]; dest -= 1; ca -= 1
                        wa += 1; la -= 1; wb = 0
                        if la == 0: mode = 'ok'; break
                        if wa >= mg: break
                    else:
                        w[dest] = t[ct]; dest -= 1; ct -= 1
                        wb += 1; lb -= 1; wa = 0
                        if lb == 1: mode = 'copya'; break
                        if wb >= mg: break
                if mode: break
                mg += 1
                first = True
                while wa >= MIN_GALLOP or wb >= MIN_GALLOP or first:
                    first = False
                    mg = max(1, mg - 1); st['mg'] = mg
                    k = gallop_right(w, t[ct], ba, la, la - 1)
                    wa = la - k
                    if wa > 0:
                        dest -= wa; ca -= wa
                        w[dest + 1:dest + 1 + wa] = w[ca + 1:ca + 1 + wa]
                        la -= wa
                        if la == 0: mode = 'ok'; break
                    w[dest] = t[ct]; dest -= 1; ct -= 1
                    lb -= 1
                    if lb == 1: mode = 'copya'; break
                    k = gallop_left(t, w[ca], 0, lb, lb - 1)
                    wb = lb - k
                    if wb > 0:
                        dest -= wb; ct -= wb
                        w[dest + 1:dest + 1 + wb] = t[ct + 1:ct + 1 + wb]
                        lb -= wb
                        if lb == 1: mode = 'copya'; break
                        if lb == 0: mode = 'ok'; break
                    w[dest] = w[ca]; dest -= 1; ca -= 1
                    la -= 1
                    if la == 0: mode = 'ok'; break
                if mode: break
                mg += 1; st['mg'] = mg
        if mode == 'ok':
            if lb > 0: w[dest - (lb - 1):dest + 1] = t[0:lb]
        else:
            dest -= la; ca -= la
            w[dest + 1:dest + 1 + la] = w[ca + 1:ca + 1 + la]
            w[dest] = t[ct]

    def merge_at(i):
        ba, la = runs[i]; bb, lb = runs[i + 1]
        runs[i] = (ba, la + lb)
        del runs[i + 1]
        k = gallop_right(w, w[bb], ba, la, 0)
        ba += k; la -= k
        if la == 0: return
        lb = gallop_left(w, w[ba + la - 1], bb, lb, lb - 1)
        if lb == 0: return
        if la <= lb: merge_low(ba, la, bb, lb)
        else: merge_high(ba, la, bb, lb)

    def inv(n):
        if n < 2: return True
        return runs[n - 2][1] > runs[n - 1][1] + runs[n][1]

    def collapse():
        while len(runs) > 1:
            n = len(runs) - 2
            if not inv(n + 1) or not inv(n):
                if runs[n - 1][1] < runs[n + 1][1]: n -= 1
                merge_at(n)
            elif runs[n][1] <= runs[n + 1][1]:
                merge_at(n)
            else:
                break

    def force():
        while len(runs) > 1:
            n = len(runs) - 2
            if n > 0 and runs[n - 1][1] < runs[n + 1][1]: n -= 1
            merge_at(n)

    if n <= 7:
        # observed in Node 26.4.0 / V8 14.6: short arrays skip run detection
        bis(0, 0, n)
        return w
    lo, rem = 0, n
    mr = minrun(n)
    while rem:
        cur = count_run(lo, lo + rem)
        if cur < mr:
            f = min(mr, rem)
            bis(lo, lo + cur, lo + f)
            cur = f
        runs.append((lo, cur))
        collapse()
        lo += cur; rem -= cur
    force()
    return w
