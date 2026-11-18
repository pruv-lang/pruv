/* L3 P10H: one C binary that links two Pruv programs (packages twoa and twob), driven by check_two_programs.py.
 *
 * Both programs define the same entry name `calc(st: [i64], x: i64) -> i64`, build a Vec<i64> through the G6 builtins
 * and return from it:
 *   twoa: vec_get(v, 0) * 1000 + vec_get(v, 1) = x * 1000 + 7
 *   twob: vec_get(v, 0) * 100  + vec_get(v, 1) = x * 100 + 9
 * Every global symbol of each program carries its package prefix, so each call must come back with its own result. */
#include <stdint.h>
#include <stdio.h>

typedef struct PruvHandle PruvHandle;

PruvHandle *twoa__pruv_handle_new(int64_t len);
void twoa__pruv_handle_free(PruvHandle *h);
int32_t twoa__pruv_h_calc(PruvHandle *h, int64_t a0, int64_t *ret);
PruvHandle *twob__pruv_handle_new(int64_t len);
void twob__pruv_handle_free(PruvHandle *h);
int32_t twob__pruv_h_calc(PruvHandle *h, int64_t a0, int64_t *ret);

#define SENT ((int64_t)0x5EA15EA15EA15EA1LL)
static int fails = 0;
#define CHECK(c, msg)                                                                  \
    do {                                                                               \
        if (!(c)) {                                                                    \
            printf("FAIL line %d: %s\n", __LINE__, msg);                               \
            fails++;                                                                   \
        }                                                                              \
    } while (0)

int main(void) {
    PruvHandle *a = twoa__pruv_handle_new(1);
    PruvHandle *b = twob__pruv_handle_new(1);
    int64_t ra = SENT, rb = SENT;
    CHECK(a != NULL && b != NULL, "handles");
    if (a && b) {
        /* interleaved calls: each program keeps its own G6 bridge, its own Vec adapters and its own result */
        CHECK(twoa__pruv_h_calc(a, 5, &ra) == 0, "twoa calc status");
        CHECK(ra == 5007, "twoa calc(5) == 5007");
        CHECK(twob__pruv_h_calc(b, 5, &rb) == 0, "twob calc status");
        CHECK(rb == 509, "twob calc(5) == 509");
        CHECK(twoa__pruv_h_calc(a, 6, &ra) == 0 && ra == 6007, "twoa calc(6) == 6007");
        CHECK(twob__pruv_h_calc(b, 6, &rb) == 0 && rb == 609, "twob calc(6) == 609");
        CHECK(twoa__pruv_h_calc(a, 0, &ra) == 0 && ra == 7, "twoa calc(0) == 7");
        CHECK(twob__pruv_h_calc(b, 0, &rb) == 0 && rb == 9, "twob calc(0) == 9");
    }
    if (a) twoa__pruv_handle_free(a);
    if (b) twob__pruv_handle_free(b);
    if (fails) {
        printf("%d check(s) failed\n", fails);
        return 1;
    }
    printf("OK\n");
    return 0;
}
