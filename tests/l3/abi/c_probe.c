/* L3 P10: C-side probe of the pruv_c handle API (manifest raw_entries = "c"), driven by check_c_handle.py.
 *
 * Program (see check_c_handle.py SRC): state = 4 words.
 *   setup(inout st)                         -> ()    st = [10, 20, 0, 0]
 *   op(inout st, a: [i64], inout out: [i64]) -> i64   st[2] += a[0]; out[0] = st[2]; returns st[2] + st[0]
 *   big(st, t: i64) -> bool                 st[2] > t
 *   flagset(st) -> bool                     st[3] == 1
 *   setflag(inout st, f: bool) -> ()        st[3] = f
 *   flag0(st, b: [bool]) -> i64             b[0] ? 1 : 0
 *   grow(prev: [i64], inout next: [i64]) -> i64   rebuild: 1 if new buffer too small (state unchanged), else copy, 0
 *   clash(state, st: [i64], h, ret: i64, ro: [i64], ro_len: i64) -> i64
 *                                           st[0]*10000 + h*1000 + ret*100 + ro[0]*10 + ro_len + state[0]*100000
 *   bsum(state, b: [u8]) -> i64             sum of the bytes
 *   slen(state, s: str) -> i64              len(s)
 *   bflip(state, inout b: [bool]) -> ()     b[i] = !b[i] for all i (needs len >= 1)
 *   stlen(state) -> i64                     len(state)
 *
 * Every global symbol of the generated pruv_c module carries the manifest package name as a prefix ("app__", see
 * check_c_handle.py). The object-like macros below map the short names used in this file to the real symbols.
 *
 * Batch entry (only for entries whose state is inout [i64] and whose return is i64, here: op):
 *   app__pruv_hb_op(h, n, a, a_len, out, out_len, ret, done)  runs items 0..n in order inside one store access and
 *   stops after the first item whose result is nonzero; *done = number of items that ran (status 0 or 2). An
 *   up-front status 2 writes *done = 0 when done is itself usable (non-null, aligned, clear of the handle object and
 *   of the ret table), else leaves it alone; a NULL or poisoned handle never writes it.
 *
 * Not tested on purpose: double free and use after free. Without a registry they cannot be detected (undefined
 * behaviour), so the API only documents them.
 */
#include <stdint.h>
#include <stdio.h>
#include <stddef.h>

#define pruv_handle_new app__pruv_handle_new
#define pruv_handle_free app__pruv_handle_free
#define pruv_handle_poisoned app__pruv_handle_poisoned
#define pruv_handle_probe_panic app__pruv_handle_probe_panic
#define pruv_h_setup app__pruv_h_setup
#define pruv_h_op app__pruv_h_op
#define pruv_h_big app__pruv_h_big
#define pruv_h_flagset app__pruv_h_flagset
#define pruv_h_setflag app__pruv_h_setflag
#define pruv_h_flag0 app__pruv_h_flag0
#define pruv_hr_grow app__pruv_hr_grow
#define pruv_h_clash app__pruv_h_clash
#define pruv_h_bsum app__pruv_h_bsum
#define pruv_h_slen app__pruv_h_slen
#define pruv_h_bflip app__pruv_h_bflip
#define pruv_h_stlen app__pruv_h_stlen
#define pruv_hb_op app__pruv_hb_op

typedef struct PruvHandle PruvHandle;

PruvHandle *pruv_handle_new(int64_t len);
void pruv_handle_free(PruvHandle *h);
int32_t pruv_handle_poisoned(const PruvHandle *h);
int32_t pruv_handle_probe_panic(PruvHandle *h);

int32_t pruv_h_setup(PruvHandle *h);
int32_t pruv_h_op(PruvHandle *h, const int64_t *a, int64_t a_len, int64_t *out, int64_t out_len, int64_t *ret);
int32_t pruv_h_big(PruvHandle *h, int64_t t, int64_t *ret);
int32_t pruv_h_flagset(PruvHandle *h, int64_t *ret);
int32_t pruv_h_setflag(PruvHandle *h, int64_t f);
int32_t pruv_h_flag0(PruvHandle *h, const uint8_t *b, int64_t b_len, int64_t *ret);
int32_t pruv_hr_grow(PruvHandle *h, int64_t new_len, int64_t *ret);
/* clash(state, st: [i64], h: i64, ret: i64, ro: [i64], ro_len: i64): positional C names, state is the resident array */
int32_t pruv_h_clash(PruvHandle *h, const int64_t *st, int64_t st_len, int64_t h_arg, int64_t ret_arg, const int64_t *ro,
                     int64_t ro_ptr_len, int64_t ro_len_arg, int64_t *ret);
int32_t pruv_h_bsum(PruvHandle *h, const uint8_t *b, int64_t b_len, int64_t *ret);
int32_t pruv_h_slen(PruvHandle *h, const uint8_t *s, int64_t s_len, int64_t *ret);
int32_t pruv_h_bflip(PruvHandle *h, uint8_t *b, int64_t b_len);
int32_t pruv_h_stlen(PruvHandle *h, int64_t *ret);
/* batch of op: a is a table of n ro pointers (a_len: n lengths), out a table of n inout pointers (out_len: n lengths),
 * ret n results, *done = items that ran */
int32_t pruv_hb_op(PruvHandle *h, int64_t n, const int64_t *const *a, const int64_t *a_len, int64_t *const *out,
                   const int64_t *out_len, int64_t *ret, int64_t *done);

enum { PRUV_OK = 0, PRUV_POISONED = 1, PRUV_BAD_ARGS = 2, PRUV_PANICKED = 3 };

#define SENT ((int64_t)0x5EA15EA15EA15EA1LL)
static int fails = 0;
#define CHECK(c, msg)                                                                  \
    do {                                                                               \
        if (!(c)) {                                                                    \
            printf("FAIL line %d: %s\n", __LINE__, msg);                               \
            fails++;                                                                   \
        }                                                                              \
    } while (0)

/* a new handle with the setup state [10, 20, 0, 0] */
static PruvHandle *fresh(void) {
    PruvHandle *p = pruv_handle_new(4);
    if (p && pruv_h_setup(p) != PRUV_OK) {
        pruv_handle_free(p);
        return NULL;
    }
    return p;
}

/* one op call on a fresh sentinel-guarded output buffer: ob = {SENT, SENT, SENT}, out = &ob[1] (1 word) */
int main(void) {
    int64_t ret = SENT;

    /* ---- handle basics ---- */
    CHECK(pruv_handle_new(-1) == NULL, "new(-1) is NULL");
    CHECK(pruv_handle_new(INT64_MAX) == NULL, "new(INT64_MAX) is NULL");
    pruv_handle_free(NULL); /* ignored */
    CHECK(pruv_handle_poisoned(NULL) == 2, "poisoned(NULL) == 2");
    PruvHandle *h = pruv_handle_new(4);
    CHECK(h != NULL, "new(4)");
    if (!h) return 1;
    CHECK(pruv_handle_poisoned(h) == 0, "fresh handle not poisoned");

    /* ---- ABI + lifetime: state persists across calls ---- */
    ret = SENT;
    CHECK(pruv_h_big(h, -1, &ret) == PRUV_OK && ret == 1, "zero state: 0 > -1");
    CHECK(pruv_h_setup(h) == PRUV_OK, "setup");
    int64_t a[1] = {5};
    int64_t ob[3] = {SENT, SENT, SENT};
    ret = SENT;
    CHECK(pruv_h_op(h, a, 1, &ob[1], 1, &ret) == PRUV_OK, "op #1 status");
    CHECK(ret == 15, "op #1 result 5+10");
    CHECK(ob[1] == 5, "op #1 out[0]");
    /* provenance: ro buffer unchanged, guard words around the inout buffer unchanged */
    CHECK(a[0] == 5, "ro buffer unchanged");
    CHECK(ob[0] == SENT && ob[2] == SENT, "inout guards unchanged");
    a[0] = 7;
    CHECK(pruv_h_op(h, a, 1, &ob[1], 1, &ret) == PRUV_OK && ret == 22 && ob[1] == 12, "op #2 persisted state");
    CHECK(pruv_h_big(h, 11, &ret) == PRUV_OK && ret == 1, "big(11): 12 > 11");
    CHECK(pruv_h_big(h, 12, &ret) == PRUV_OK && ret == 0, "big(12): 12 > 12 is false");
    CHECK(pruv_h_flagset(h, &ret) == PRUV_OK && ret == 0, "flag clear");
    CHECK(pruv_h_setflag(h, 1) == PRUV_OK, "setflag(1)");
    CHECK(pruv_h_flagset(h, &ret) == PRUV_OK && ret == 1, "flag set");
    CHECK(pruv_h_setflag(h, 0) == PRUV_OK && pruv_h_flagset(h, &ret) == PRUV_OK && ret == 0, "setflag(0)");
    {
        uint8_t b1[1] = {1}, b0[1] = {0};
        CHECK(pruv_h_flag0(h, b1, 1, &ret) == PRUV_OK && ret == 1, "flag0 true");
        CHECK(pruv_h_flag0(h, b0, 1, &ret) == PRUV_OK && ret == 0, "flag0 false");
    }

    /* ---- ret slots between sentinels: only the middle word is written, and only on success ---- */
    {
        int64_t rb[3] = {SENT, SENT, SENT};
        int64_t z[1] = {0}, ob2[3] = {SENT, SENT, SENT};
        /* state is [10, 20, 12, 0] here; op with a[0] = 0 leaves st[2] = 12 and returns 12 + 10 */
        CHECK(pruv_h_op(h, z, 1, &ob2[1], 1, &rb[1]) == PRUV_OK && rb[1] == 22 && ob2[1] == 12, "op sentinel ret");
        CHECK(rb[0] == SENT && rb[2] == SENT && ob2[0] == SENT && ob2[2] == SENT, "op sentinel guards intact");
        rb[1] = SENT;
        CHECK(pruv_h_big(h, 11, &rb[1]) == PRUV_OK && rb[1] == 1, "big sentinel ret");
        CHECK(rb[0] == SENT && rb[2] == SENT, "big sentinel guards intact");
        /* clash: every argument and the resident state distinguishable. st[0] is the OUTER array's word, not the state's */
        {
            int64_t st[1] = {3}, ro[1] = {6};
            rb[1] = SENT;
            CHECK(pruv_h_clash(h, st, 1, 4, 5, ro, 1, 7, &rb[1]) == PRUV_OK, "clash status");
            /* 3*10000 + 4*1000 + 5*100 + 6*10 + 7 + 10*100000 */
            CHECK(rb[1] == 1034567, "clash exact result");
            CHECK(rb[0] == SENT && rb[2] == SENT && st[0] == 3 && ro[0] == 6, "clash guards and inputs intact");
            /* the same call with the state-sized arrays swapped must give a different number (st[0] is not the state) */
            int64_t st2[1] = {9};
            rb[1] = SENT;
            CHECK(pruv_h_clash(h, st2, 1, 4, 5, ro, 1, 7, &rb[1]) == PRUV_OK && rb[1] == 1094567, "clash uses outer st[0]");
            /* a bad clash call leaves ret alone */
            rb[1] = SENT;
            CHECK(pruv_h_clash(h, NULL, 1, 4, 5, ro, 1, 7, &rb[1]) == PRUV_BAD_ARGS && rb[1] == SENT, "clash null st");
            CHECK(rb[0] == SENT && rb[2] == SENT, "clash bad call guards intact");
        }
        CHECK(pruv_h_stlen(h, &rb[1]) == PRUV_OK && rb[1] == 4, "stlen is 4 before any rebuild");
    }

    /* ---- [u8], str, [bool] arrays ---- */
    {
        int64_t rb[3] = {SENT, SENT, SENT};
        uint8_t by[3] = {1, 2, 250};
        CHECK(pruv_h_bsum(h, by, 3, &rb[1]) == PRUV_OK && rb[1] == 253, "[u8] sum");
        rb[1] = SENT;
        CHECK(pruv_h_bsum(h, NULL, 0, &rb[1]) == PRUV_OK && rb[1] == 0, "[u8] length 0 with NULL pointer");
        rb[1] = SENT;
        CHECK(pruv_h_slen(h, (const uint8_t *)"hello", 5, &rb[1]) == PRUV_OK && rb[1] == 5, "str ok");
        rb[1] = SENT;
        CHECK(pruv_h_slen(h, NULL, 0, &rb[1]) == PRUV_OK && rb[1] == 0, "str length 0 with NULL pointer");
        CHECK(rb[0] == SENT && rb[2] == SENT, "u8/str ret guards intact");
        {
            uint8_t bad8[1] = {0xff};
            rb[1] = SENT;
            CHECK(pruv_h_slen(h, bad8, 1, &rb[1]) == PRUV_BAD_ARGS && rb[1] == SENT, "invalid UTF-8 {0xff}");
            uint8_t trunc[2] = {'a', 0xc3}; /* truncated two-byte sequence */
            CHECK(pruv_h_slen(h, trunc, 2, &rb[1]) == PRUV_BAD_ARGS && rb[1] == SENT, "truncated UTF-8");
        }
        {
            uint8_t fl[3] = {1, 0, 1};
            uint8_t gd[5] = {0xAA, 1, 0, 1, 0xAA};
            uint8_t badfl[3] = {0, 2, 1};
            CHECK(pruv_h_bflip(h, badfl, 3) == PRUV_BAD_ARGS, "inout [bool] byte 2");
            CHECK(badfl[0] == 0 && badfl[1] == 2 && badfl[2] == 1, "inout [bool] buffer unchanged after refusal");
            CHECK(pruv_h_bflip(h, gd + 1, 3) == PRUV_OK, "bflip status");
            CHECK(gd[0] == 0xAA && gd[1] == 0 && gd[2] == 1 && gd[3] == 0 && gd[4] == 0xAA, "bflip flips, guards intact");
            CHECK(pruv_h_bflip(h, fl, 1) == PRUV_OK && fl[0] == 0 && fl[1] == 0 && fl[2] == 1, "bflip length 1");
            CHECK(pruv_h_bflip(h, NULL, 1) == PRUV_BAD_ARGS, "inout [bool] NULL with length 1");
        }
    }

    /* ---- BAD_ARGS: refused, nothing ran (state and every out buffer unchanged) ---- */
    {
        int64_t buf[4] = {1, 2, 3, 4};
        int64_t o[3] = {SENT, SENT, SENT};
        int64_t r = SENT;
        int64_t *out = &o[1];
        int64_t *misal = (int64_t *)((char *)buf + 1);
        /* each case: status must be BAD_ARGS, ret and out untouched */
#define BAD(call, msg)                                                                 \
    do {                                                                               \
        o[1] = SENT;                                                                   \
        r = SENT;                                                                      \
        CHECK((call) == PRUV_BAD_ARGS, msg " status");                                 \
        CHECK(r == SENT && o[0] == SENT && o[1] == SENT && o[2] == SENT, msg " untouched"); \
    } while (0)
        BAD(pruv_h_op(NULL, a, 1, out, 1, &r), "h NULL");
        BAD(pruv_h_op(h, a, 1, out, 1, NULL), "ret NULL");
        BAD(pruv_h_op(h, a, 1, out, 1, (int64_t *)((char *)&r + 1)), "ret misaligned");
        BAD(pruv_h_op(h, a, 1, out, 1, (int64_t *)(UINTPTR_MAX - 7)), "ret end address overflows");
        BAD(pruv_h_op(h, a, -1, out, 1, &r), "negative ro length");
        BAD(pruv_h_op(h, a, 1, out, -1, &r), "negative inout length");
        BAD(pruv_h_op(h, NULL, 1, out, 1, &r), "null ro with len>0");
        BAD(pruv_h_op(h, a, 1, NULL, 1, &r), "null inout with len>0");
        BAD(pruv_h_op(h, misal, 1, out, 1, &r), "misaligned ro");
        BAD(pruv_h_op(h, a, 1, (int64_t *)((char *)buf + 1), 1, &r), "misaligned inout");
        /* overlaps: inout over ro, inout over inout-ret, ro over ret, partial overlap */
        BAD(pruv_h_op(h, a, 1, a, 1, &r), "out == a");
        BAD(pruv_h_op(h, buf, 2, buf + 1, 2, &r), "partial overlap a/out");
        BAD(pruv_h_op(h, a, 1, &r, 1, &r), "out == ret");
        BAD(pruv_h_op(h, &r, 1, out, 1, &r), "a == ret");
        /* (pointer, len) whose end address overflows the address space */
        {
            const int64_t *top = (const int64_t *)(UINTPTR_MAX & ~(uintptr_t)7);
            BAD(pruv_h_op(h, top, 2, out, 1, &r), "ro end address overflows");
            BAD(pruv_h_op(h, a, 1, (int64_t *)(UINTPTR_MAX & ~(uintptr_t)7), 2, &r), "inout end address overflows");
            const uint8_t *top8 = (const uint8_t *)UINTPTR_MAX;
            BAD(pruv_h_slen(h, top8, 2, &r), "str end address overflows");
            BAD(pruv_h_bsum(h, top8, 2, &r), "[u8] end address overflows");
        }
        /* i64 array length above INT64_MAX/8 */
        BAD(pruv_h_op(h, a, INT64_MAX / 8 + 1, out, 1, &r), "ro length > INT64_MAX/8");
        BAD(pruv_h_op(h, a, 1, out, INT64_MAX / 8 + 1, &r), "inout length > INT64_MAX/8");
        BAD(pruv_h_op(h, a, INT64_MAX, out, 1, &r), "ro length INT64_MAX");
        /* any buffer or ret overlapping the handle object itself */
        BAD(pruv_h_op(h, (const int64_t *)h, 1, out, 1, &r), "ro overlaps the handle");
        BAD(pruv_h_op(h, a, 1, (int64_t *)h, 1, &r), "inout overlaps the handle");
        BAD(pruv_h_op(h, a, 1, out, 1, (int64_t *)h), "ret overlaps the handle");
        BAD(pruv_h_big(h, 0, (int64_t *)h), "big: ret overlaps the handle");
        BAD(pruv_h_bsum(h, (const uint8_t *)h, 1, &r), "[u8] overlaps the handle");
        BAD(pruv_h_slen(h, (const uint8_t *)h, 1, &r), "str overlaps the handle");
        BAD(pruv_h_bflip(h, (uint8_t *)h, 1), "inout [bool] overlaps the handle");
        /* nothing above may have touched the state or the ro/guard buffers */
        CHECK(buf[0] == 1 && buf[1] == 2 && buf[2] == 3 && buf[3] == 4, "overlap cases left buf unchanged");
        CHECK(a[0] == 7, "a unchanged");
        /* adjacent (touching) regions do not overlap: a = buf[1], out = buf[0], ret = buf[2] */
        {
            int64_t two[3] = {SENT, 9, SENT};
            /* st[2] was 12: becomes 21, out = 21, ret = 21 + st[0] (10) = 31 */
            CHECK(pruv_h_op(h, &two[1], 1, &two[0], 1, &two[2]) == PRUV_OK, "adjacent regions are fine");
            CHECK(two[1] == 9, "adjacent: ro unchanged");
            CHECK(two[0] == 21 && two[2] == 31, "adjacent: out and ret written");
        }
        /* bool scalar 2 */
        CHECK(pruv_h_setflag(h, 2) == PRUV_BAD_ARGS, "bool scalar 2");
        CHECK(pruv_h_setflag(h, -1) == PRUV_BAD_ARGS, "bool scalar -1");
        r = SENT;
        CHECK(pruv_h_flagset(h, &r) == PRUV_OK && r == 0, "flag still clear after refused setflag");
        /* [bool] byte 2 */
        {
            uint8_t bad[2] = {1, 2};
            r = SENT;
            CHECK(pruv_h_flag0(h, bad, 2, &r) == PRUV_BAD_ARGS && r == SENT, "[bool] byte 2");
            CHECK(pruv_h_flag0(h, bad, 1, &r) == PRUV_OK && r == 1, "[bool] prefix of valid bytes ok");
        }
        CHECK(pruv_handle_poisoned(h) == 0, "BAD_ARGS never poisons");
    }

    /* ---- rebuild: commit on 0, keep the state on nonzero ---- */
    {
        int64_t r = SENT;
        CHECK(pruv_hr_grow(h, 2, &r) == PRUV_OK && r == 1, "rebuild too small: entry returns 1");
        CHECK(pruv_h_big(h, 20, &r) == PRUV_OK && r == 1, "state unchanged after nonzero rebuild");
        CHECK(pruv_h_stlen(h, &r) == PRUV_OK && r == 4, "stlen still 4 after nonzero rebuild");
        r = SENT;
        CHECK(pruv_hr_grow(h, 8, &r) == PRUV_OK && r == 0, "rebuild to 8 words commits");
        CHECK(pruv_h_stlen(h, &r) == PRUV_OK && r == 8, "stlen is 8 after pruv_hr_grow(h, 8)");
        CHECK(pruv_h_big(h, 20, &r) == PRUV_OK && r == 1, "state carried into rebuilt buffer");
        CHECK(pruv_h_big(h, 21, &r) == PRUV_OK && r == 0, "state values preserved (st[2] == 21)");
        CHECK(pruv_hr_grow(NULL, 8, &r) == PRUV_BAD_ARGS, "rebuild h NULL");
        r = SENT;
        CHECK(pruv_hr_grow(h, -1, &r) == PRUV_BAD_ARGS && r == SENT, "rebuild negative length");
        CHECK(pruv_hr_grow(h, 8, NULL) == PRUV_BAD_ARGS, "rebuild ret NULL");
        CHECK(pruv_hr_grow(h, 8, (int64_t *)((char *)&r + 1)) == PRUV_BAD_ARGS, "rebuild ret misaligned");
        CHECK(pruv_hr_grow(h, 8, (int64_t *)(UINTPTR_MAX - 7)) == PRUV_BAD_ARGS, "rebuild ret end address overflows");
        CHECK(pruv_h_stlen(h, &r) == PRUV_OK && r == 8, "stlen unchanged (8) after rebuild with overflowing ret");
        /* oversized / negative new_len and ret over the handle: refused, state (stlen 8) and ret unchanged */
        r = SENT;
        CHECK(pruv_hr_grow(h, INT64_MAX, &r) == PRUV_BAD_ARGS && r == SENT, "rebuild new_len INT64_MAX");
        CHECK(pruv_hr_grow(h, INT64_MAX / 8 + 1, &r) == PRUV_BAD_ARGS && r == SENT, "rebuild new_len > INT64_MAX/8");
        CHECK(pruv_hr_grow(h, INT64_MIN, &r) == PRUV_BAD_ARGS && r == SENT, "rebuild new_len INT64_MIN");
        CHECK(pruv_hr_grow(h, 16, (int64_t *)h) == PRUV_BAD_ARGS, "rebuild ret overlaps the handle");
        CHECK(pruv_h_stlen(h, &r) == PRUV_OK && r == 8, "stlen unchanged (8) after refused rebuilds");
        CHECK(pruv_h_big(h, 20, &r) == PRUV_OK && r == 1, "state intact after refused rebuilds");
    }

    /* ---- batch entry: items run in order in one store access, stop after the first nonzero result ---- */
    {
        /* op returns st[2] + st[0] and st[0] is 10 after setup, so an item returns 0 exactly while the running st[2]
         * is -10: av = {-10, 0, 0} keeps every result 0 (the batch runs to the end), a later nonzero stops it */
        int64_t av[3] = {-10, 0, 0};
        const int64_t *at[3] = {&av[0], &av[1], &av[2]};
        int64_t alen[3] = {1, 1, 1}, olen[3] = {1, 1, 1};
        int64_t ov[3] = {SENT, SENT, SENT}, rt[3] = {SENT, SENT, SENT};
        int64_t *ot[3] = {&ov[0], &ov[1], &ov[2]};
        int64_t dn = SENT, r = SENT;
        PruvHandle *hb = fresh(), *tw = fresh();
        CHECK(hb != NULL && tw != NULL, "batch: fresh handles");
        if (hb && tw) {
            /* (i) n = 3 all-zero results == three single calls on a twin handle, done = 3 */
            CHECK(pruv_hb_op(hb, 3, at, alen, ot, olen, rt, &dn) == PRUV_OK, "batch n=3 status");
            CHECK(dn == 3, "batch n=3 done == 3");
            {
                int64_t ov2[3] = {SENT, SENT, SENT}, rt2[3] = {SENT, SENT, SENT};
                int i;
                for (i = 0; i < 3; i++)
                    CHECK(pruv_h_op(tw, &av[i], 1, &ov2[i], 1, &rt2[i]) == PRUV_OK, "twin single op status");
                for (i = 0; i < 3; i++) {
                    CHECK(rt[i] == rt2[i] && rt[i] == 0, "batch ret equals single-call ret");
                    CHECK(ov[i] == ov2[i] && ov[i] == -10, "batch out equals single-call out");
                }
                CHECK(pruv_h_big(hb, -11, &r) == PRUV_OK && r == 1 && pruv_h_big(tw, -11, &r) == PRUV_OK && r == 1,
                      "batch and twin state agree (st[2] == -10)");
            }
            CHECK(av[0] == -10 && av[1] == 0 && av[2] == 0, "batch ro items unchanged");
        }
        pruv_handle_free(hb);
        pruv_handle_free(tw);

        /* (ii) stop at the first nonzero result: done = index + 1, later ret/out slots untouched, later items not run */
        {
            int64_t av2[3] = {-10, 5, 100};
            const int64_t *at2[3] = {&av2[0], &av2[1], &av2[2]};
            int64_t ov3[3] = {SENT, SENT, SENT}, rt3[3] = {SENT, SENT, SENT};
            int64_t *ot3[3] = {&ov3[0], &ov3[1], &ov3[2]};
            PruvHandle *g = fresh();
            dn = SENT;
            CHECK(g != NULL, "batch stop: fresh handle");
            if (g) {
                CHECK(pruv_hb_op(g, 3, at2, alen, ot3, olen, rt3, &dn) == PRUV_OK, "batch stop status");
                CHECK(dn == 2, "batch stop done == 2");
                CHECK(rt3[0] == 0 && rt3[1] == 5 && rt3[2] == SENT, "batch stop ret: item 2 slot untouched");
                CHECK(ov3[0] == -10 && ov3[1] == -5 && ov3[2] == SENT, "batch stop out: item 2 slot untouched");
                CHECK(pruv_h_big(g, 50, &r) == PRUV_OK && r == 0, "batch stop: item 2 did not run (st[2] == -5)");
                CHECK(pruv_h_big(g, -6, &r) == PRUV_OK && r == 1, "batch stop: items 0 and 1 committed");
                pruv_handle_free(g);
            }
            /* the very first item already nonzero: done = 1 */
            g = fresh();
            if (g) {
                int64_t av4[2] = {7, 1};
                const int64_t *at4[2] = {&av4[0], &av4[1]};
                int64_t ov4[2] = {SENT, SENT}, rt4[2] = {SENT, SENT};
                int64_t *ot4[2] = {&ov4[0], &ov4[1]};
                dn = SENT;
                CHECK(pruv_hb_op(g, 2, at4, alen, ot4, olen, rt4, &dn) == PRUV_OK && dn == 1, "batch first item stops: done 1");
                CHECK(rt4[0] == 17 && rt4[1] == SENT && ov4[0] == 7 && ov4[1] == SENT, "batch first item stops: slots");
                pruv_handle_free(g);
            }
        }

        /* (iii) n = 0: status 0, done = 0, nothing else touched (tables may be NULL) */
        {
            PruvHandle *g = fresh();
            int64_t rz = SENT;
            dn = SENT;
            CHECK(g != NULL, "batch n=0: fresh handle");
            if (g) {
                CHECK(pruv_hb_op(g, 0, at, alen, ot, olen, &rz, &dn) == PRUV_OK, "batch n=0 status");
                CHECK(dn == 0 && rz == SENT, "batch n=0: done 0, ret untouched");
                dn = SENT;
                CHECK(pruv_hb_op(g, 0, NULL, NULL, NULL, NULL, &rz, &dn) == PRUV_OK && dn == 0, "batch n=0 with NULL tables");
                CHECK(pruv_h_big(g, -1, &r) == PRUV_OK && r == 1, "batch n=0 left the state alone (st[2] == 0)");
                pruv_handle_free(g);
            }
        }

        /* (iv) bad tables up front: NULL with n > 0, n < 0: status 2, nothing ran */
        {
            PruvHandle *g = fresh();
            int64_t o5[3] = {SENT, SENT, SENT}, r5[3] = {SENT, SENT, SENT};
            int64_t *ot5[2] = {&o5[0], &o5[1]};
            int64_t av5[2] = {-10, 0};
            const int64_t *at5[2] = {&av5[0], &av5[1]};
            CHECK(g != NULL, "batch bad tables: fresh handle");
            if (g) {
                dn = SENT;
                CHECK(pruv_hb_op(g, 2, NULL, alen, ot5, olen, r5, &dn) == PRUV_BAD_ARGS, "batch NULL ro table");
                CHECK(pruv_hb_op(g, 2, at5, NULL, ot5, olen, r5, &dn) == PRUV_BAD_ARGS, "batch NULL ro length table");
                CHECK(pruv_hb_op(g, 2, at5, alen, NULL, olen, r5, &dn) == PRUV_BAD_ARGS, "batch NULL inout table");
                CHECK(pruv_hb_op(g, 2, at5, alen, ot5, NULL, r5, &dn) == PRUV_BAD_ARGS, "batch NULL inout length table");
                CHECK(pruv_hb_op(g, 2, at5, alen, ot5, olen, NULL, &dn) == PRUV_BAD_ARGS, "batch NULL ret table");
                CHECK(pruv_hb_op(g, 2, at5, alen, ot5, olen, r5, NULL) == PRUV_BAD_ARGS, "batch NULL done");
                CHECK(pruv_hb_op(NULL, 2, at5, alen, ot5, olen, r5, &dn) == PRUV_BAD_ARGS, "batch NULL handle");
                CHECK(pruv_hb_op(g, -1, at5, alen, ot5, olen, r5, &dn) == PRUV_BAD_ARGS, "batch n = -1");
                CHECK(pruv_hb_op(g, INT64_MIN, at5, alen, ot5, olen, r5, &dn) == PRUV_BAD_ARGS, "batch n = INT64_MIN");
                CHECK(pruv_hb_op(g, 2, (const int64_t *const *)((char *)at5 + 1), alen, ot5, olen, r5, &dn) == PRUV_BAD_ARGS,
                      "batch misaligned ro table");
                CHECK(pruv_hb_op(g, 2, at5, alen, ot5, olen, (int64_t *)((char *)r5 + 1), &dn) == PRUV_BAD_ARGS,
                      "batch misaligned ret table");
                CHECK(pruv_hb_op(g, 2, at5, alen, ot5, olen, r5, (int64_t *)((char *)&dn + 1)) == PRUV_BAD_ARGS,
                      "batch misaligned done");
                CHECK(r5[0] == SENT && r5[1] == SENT && o5[0] == SENT && o5[1] == SENT, "batch bad tables: slots untouched");
                CHECK(av5[0] == -10 && av5[1] == 0, "batch bad tables: ro untouched");
                CHECK(pruv_h_big(g, -1, &r) == PRUV_OK && r == 1, "batch bad tables: state untouched (st[2] == 0)");
                CHECK(pruv_handle_poisoned(g) == 0, "batch BAD_ARGS never poisons");
                pruv_handle_free(g);
            }
        }

        /* (v) ret table overlapping done / the handle: status 2 up front, nothing ran */
        {
            PruvHandle *g = fresh();
            int64_t rd[2] = {SENT, SENT};
            int64_t o6[2] = {SENT, SENT};
            int64_t av6[2] = {-10, 0};
            const int64_t *at6[2] = {&av6[0], &av6[1]};
            int64_t *ot6[2] = {&o6[0], &o6[1]};
            CHECK(g != NULL, "batch overlap: fresh handle");
            if (g) {
                CHECK(pruv_hb_op(g, 2, at6, alen, ot6, olen, rd, &rd[1]) == PRUV_BAD_ARGS, "batch ret table overlaps done");
                CHECK(pruv_hb_op(g, 1, at6, alen, ot6, olen, rd, &rd[0]) == PRUV_BAD_ARGS, "batch ret == done");
                CHECK(pruv_hb_op(g, 2, at6, alen, ot6, olen, (int64_t *)g, &dn) == PRUV_BAD_ARGS, "batch ret table overlaps the handle");
                CHECK(pruv_hb_op(g, 2, at6, alen, ot6, olen, rd, (int64_t *)g) == PRUV_BAD_ARGS, "batch done overlaps the handle");
                CHECK(rd[0] == SENT && rd[1] == SENT && o6[0] == SENT && o6[1] == SENT, "batch overlap: slots untouched");
                CHECK(pruv_h_big(g, -1, &r) == PRUV_OK && r == 1, "batch overlap: state untouched (st[2] == 0)");
                pruv_handle_free(g);
            }
        }

        /* (v-b) an up-front refusal writes *done = 0 when done is itself a usable slot (non-null, aligned, clear of the
         * handle object and of the ret table); it is left alone when done is the bad argument; NULL handle and poisoned
         * handle never write it */
        {
            PruvHandle *g = fresh();
            int64_t o9[3] = {SENT, SENT, SENT}, r9[3] = {SENT, SENT, SENT};
            int64_t *ot9[2] = {&o9[0], &o9[1]};
            int64_t av9[2] = {-10, 0};
            const int64_t *at9[2] = {&av9[0], &av9[1]};
            int64_t d9 = 77;
            CHECK(g != NULL, "batch done on refusal: fresh handle");
            if (g) {
                /* n = -1 with done preset to 77 -> status 2 and done == 0 */
                d9 = 77;
                CHECK(pruv_hb_op(g, -1, at9, alen, ot9, olen, r9, &d9) == PRUV_BAD_ARGS, "batch n = -1 status (done)");
                CHECK(d9 == 0, "batch n = -1: done == 0");
                /* a NULL ro table with n = 1 -> status 2 and done == 0 */
                d9 = 77;
                CHECK(pruv_hb_op(g, 1, NULL, alen, ot9, olen, r9, &d9) == PRUV_BAD_ARGS, "batch NULL items status (done)");
                CHECK(d9 == 0, "batch NULL items with n = 1: done == 0");
                /* a NULL ret table (n > 0) is a refusal too; done is a different, valid slot */
                d9 = 77;
                CHECK(pruv_hb_op(g, 2, at9, alen, ot9, olen, NULL, &d9) == PRUV_BAD_ARGS, "batch NULL ret status (done)");
                CHECK(d9 == 0, "batch NULL ret: done == 0");
                /* a misaligned ret table: done is still a valid slot clear of it */
                d9 = 77;
                CHECK(pruv_hb_op(g, 2, at9, alen, ot9, olen, (int64_t *)((char *)r9 + 1), &d9) == PRUV_BAD_ARGS,
                      "batch misaligned ret status (done)");
                CHECK(d9 == 0, "batch misaligned ret: done == 0");
                /* INT64_MIN: n < 0 */
                d9 = 77;
                CHECK(pruv_hb_op(g, INT64_MIN, at9, alen, ot9, olen, r9, &d9) == PRUV_BAD_ARGS, "batch n = INT64_MIN status (done)");
                CHECK(d9 == 0, "batch n = INT64_MIN: done == 0");
                /* done is the bad argument: NULL (nothing to write), misaligned (the bytes are left alone) */
                {
                    int64_t two[2] = {77, 77};
                    CHECK(pruv_hb_op(g, 1, at9, alen, ot9, olen, r9, (int64_t *)((char *)&two[0] + 1)) == PRUV_BAD_ARGS,
                          "batch misaligned done status");
                    CHECK(two[0] == 77 && two[1] == 77, "batch misaligned done: not written");
                    CHECK(pruv_hb_op(g, 1, at9, alen, ot9, olen, r9, NULL) == PRUV_BAD_ARGS, "batch NULL done status");
                }
                /* done inside the ret table: not written (the slot stays as it was) */
                {
                    int64_t rr[2] = {SENT, 77};
                    CHECK(pruv_hb_op(g, 2, at9, alen, ot9, olen, rr, &rr[1]) == PRUV_BAD_ARGS, "batch done inside ret status");
                    CHECK(rr[0] == SENT && rr[1] == 77, "batch done inside ret: not written");
                }
                /* done on the handle object: not written (that would corrupt the handle), the handle still works */
                {
                    CHECK(pruv_hb_op(g, 1, at9, alen, ot9, olen, r9, (int64_t *)g) == PRUV_BAD_ARGS, "batch done on handle status");
                    CHECK(pruv_h_big(g, -1, &r) == PRUV_OK && r == 1, "batch done on handle: the handle is intact");
                }
                /* a NULL handle never writes done */
                d9 = 77;
                CHECK(pruv_hb_op(NULL, 1, at9, alen, ot9, olen, r9, &d9) == PRUV_BAD_ARGS, "batch NULL handle status (done)");
                CHECK(d9 == 77, "batch NULL handle: done not written");
                CHECK(r9[0] == SENT && r9[1] == SENT && o9[0] == SENT && o9[1] == SENT, "batch done on refusal: slots untouched");
                CHECK(av9[0] == -10 && av9[1] == 0, "batch done on refusal: ro untouched");
                pruv_handle_free(g);
            }
        }

        /* (vi) a bad item i answers 2 with done = i; the effects of items 0..i are visible, item i did nothing */
        {
            int64_t av7[2] = {-10, 0};
            int64_t al7[2] = {1, 1};
            int64_t o7[2] = {SENT, SENT}, r7[2] = {SENT, SENT};
            int c;
            /* item 1 broken in five ways: inout over the ret table, inout over done, misaligned ro, negative inout
             * length, inout over the handle */
            for (c = 0; c < 5; c++) {
                PruvHandle *g = fresh();
                const int64_t *a7[2] = {&av7[0], &av7[1]};
                int64_t *o7t[2] = {&o7[0], &o7[1]};
                int64_t ol[2] = {1, 1};
                int64_t d7 = SENT;
                if (!g) {
                    CHECK(0, "batch bad item: fresh handle");
                    continue;
                }
                o7[0] = o7[1] = SENT;
                r7[0] = r7[1] = SENT;
                if (c == 0) o7t[1] = &r7[1];
                if (c == 1) o7t[1] = &d7;
                if (c == 2) a7[1] = (const int64_t *)((const char *)&av7[1] + 1);
                if (c == 3) ol[1] = -1;
                if (c == 4) o7t[1] = (int64_t *)g;
                CHECK(pruv_hb_op(g, 2, a7, al7, o7t, ol, r7, &d7) == PRUV_BAD_ARGS, "batch bad item status");
                CHECK(d7 == 1, "batch bad item: done == 1");
                CHECK(r7[0] == 0 && o7[0] == -10, "batch bad item: item 0 effects visible");
                CHECK(r7[1] == SENT && o7[1] == SENT, "batch bad item: item 1 wrote nothing");
                CHECK(pruv_h_big(g, -11, &r) == PRUV_OK && r == 1, "batch bad item: item 0 committed (st[2] == -10)");
                CHECK(pruv_handle_poisoned(g) == 0, "batch bad item never poisons");
                pruv_handle_free(g);
            }
        }
    }

    /* ---- poison: a caught panic poisons the handle; later calls refuse and touch nothing ---- */
    {
        int64_t o[3] = {SENT, SENT, SENT};
        int64_t r = SENT;
        CHECK(pruv_handle_probe_panic(h) == PRUV_PANICKED, "probe_panic returns 3");
        CHECK(pruv_handle_poisoned(h) == 1, "poisoned() is 1");
        CHECK(pruv_h_op(h, a, 1, &o[1], 1, &r) == PRUV_POISONED, "op on poisoned handle returns 1");
        CHECK(r == SENT && o[0] == SENT && o[1] == SENT && o[2] == SENT, "poisoned op left out buffers untouched");
        CHECK(a[0] == 7, "poisoned op left ro buffer unchanged");
        CHECK(pruv_h_big(h, 0, &r) == PRUV_POISONED && r == SENT, "query on poisoned handle returns 1");
        /* (vii) batch entry on a poisoned handle: status 1, ret and out slots untouched, nothing ran */
        {
            int64_t av8[2] = {-10, 0}, al8[2] = {1, 1}, ol8[2] = {1, 1};
            const int64_t *at8[2] = {&av8[0], &av8[1]};
            int64_t o8[2] = {SENT, SENT}, r8[2] = {SENT, SENT};
            int64_t *ot8[2] = {&o8[0], &o8[1]};
            int64_t d8 = SENT;
            CHECK(pruv_hb_op(h, 2, at8, al8, ot8, ol8, r8, &d8) == PRUV_POISONED, "batch on poisoned handle returns 1");
            CHECK(r8[0] == SENT && r8[1] == SENT && o8[0] == SENT && o8[1] == SENT, "poisoned batch left slots untouched");
            CHECK(av8[0] == -10 && av8[1] == 0, "poisoned batch left ro items unchanged");
        }
        CHECK(pruv_h_setflag(h, 1) == PRUV_POISONED, "unit entry on poisoned handle returns 1");
        CHECK(pruv_hr_grow(h, 8, &r) == PRUV_POISONED && r == SENT, "rebuild on poisoned handle returns 1");
        /* poisoned refusal comes BEFORE any argument check: invalid arguments still give 1 and touch nothing */
        {
            uint8_t bad2[1] = {2}, badutf[1] = {0xff};
            CHECK(pruv_h_setflag(h, 2) == PRUV_POISONED, "poisoned: setflag(2) -> 1");
            CHECK(pruv_h_flag0(h, bad2, 1, &r) == PRUV_POISONED && r == SENT, "poisoned: flag0 byte 2 -> 1");
            CHECK(pruv_h_slen(h, badutf, 1, &r) == PRUV_POISONED && r == SENT, "poisoned: slen invalid UTF-8 -> 1");
            CHECK(pruv_h_bflip(h, bad2, 1) == PRUV_POISONED && bad2[0] == 2, "poisoned: bflip byte 2 -> 1, untouched");
            CHECK(pruv_h_op(h, a, 1, &o[1], 1, NULL) == PRUV_POISONED, "poisoned: op NULL ret -> 1");
            CHECK(pruv_h_op(h, a, 1, (int64_t *)h, 1, (int64_t *)h) == PRUV_POISONED, "poisoned: handle overlap -> 1");
            CHECK(pruv_hr_grow(h, -1, &r) == PRUV_POISONED && r == SENT, "poisoned: hr_grow(-1) -> 1");
            CHECK(pruv_hr_grow(h, INT64_MAX, &r) == PRUV_POISONED && r == SENT, "poisoned: hr_grow(INT64_MAX) -> 1");
            CHECK(pruv_hr_grow(h, 8, NULL) == PRUV_POISONED, "poisoned: hr_grow ret NULL -> 1");
            CHECK(o[0] == SENT && o[1] == SENT && o[2] == SENT && a[0] == 7, "poisoned: nothing touched");
        }
        CHECK(pruv_handle_poisoned(h) == 1, "still poisoned");
        pruv_handle_free(h); /* a poisoned handle is still freed */
    }

    pruv_handle_free(NULL);
    /* a second, independent handle is unaffected by the first one's poison */
    {
        PruvHandle *g = pruv_handle_new(4);
        int64_t r = SENT;
        CHECK(g != NULL && pruv_handle_poisoned(g) == 0, "second handle clean");
        CHECK(pruv_h_setup(g) == PRUV_OK && pruv_h_big(g, 0, &r) == PRUV_OK && r == 0, "second handle works");
        pruv_handle_free(g);
    }

    if (fails) {
        printf("%d check(s) failed\n", fails);
        return 1;
    }
    printf("OK\n");
    return 0;
}
