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
 * Not tested on purpose: double free and use after free. Without a registry they cannot be detected (undefined
 * behaviour), so the API only documents them.
 */
#include <stdint.h>
#include <stdio.h>
#include <stddef.h>

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
