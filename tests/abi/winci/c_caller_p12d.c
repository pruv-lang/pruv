#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

typedef struct { uint16_t *data; int64_t len; } PruvArrayU16;
extern void pruv_abi_units(uint8_t, int16_t, uint64_t, const uint16_t *, int64_t, PruvArrayU16 *);
extern int64_t pruv_abi_checked_length(uint8_t *, int64_t, const uint16_t *, int64_t);
extern int64_t pruv_abi_length(const uint16_t *, int64_t);
extern void pruv_abi_round_trip(const int64_t *, int64_t *);
extern uint64_t pruv_abi_positive(uint64_t);
extern void pruv_free_v2(void *, int64_t, int64_t, int64_t);
extern uint8_t pruv_abi_id_u8(uint8_t);
extern uint16_t pruv_abi_id_u16(uint16_t);
extern uint32_t pruv_abi_id_u32(uint32_t);
extern uint64_t pruv_abi_id_u64(uint64_t);
extern uint64_t pruv_abi_id_usize(uint64_t);
extern int8_t pruv_abi_id_i8(int8_t);
extern int16_t pruv_abi_id_i16(int16_t);
extern int32_t pruv_abi_id_i32(int32_t);
extern int64_t pruv_abi_id_isize(int64_t);

int main(int argc, char **argv) {
    if (argc == 3) {
        /* argv[1] picks the entry: "two" (two buffers, slow path) or "one" (single buffer). */
        int64_t n = strtoll(argv[2], NULL, 10);
        if (argv[1][0] == 't')
            printf("%lld\n", (long long)pruv_abi_checked_length(NULL, 0, NULL, n));
        else
            printf("%lld\n", (long long)pruv_abi_length(NULL, n));
        return 0;
    }
    assert(pruv_abi_id_u8(0) == 0);
    assert(pruv_abi_id_u8(UINT8_MAX) == UINT8_MAX);
    assert(pruv_abi_id_u16(0) == 0);
    assert(pruv_abi_id_u16(UINT16_MAX) == UINT16_MAX);
    assert(pruv_abi_id_u32(0) == 0);
    assert(pruv_abi_id_u32(UINT32_MAX) == UINT32_MAX);
    assert(pruv_abi_id_u64(0) == 0);
    assert(pruv_abi_id_u64(UINT64_MAX) == UINT64_MAX);
    assert(pruv_abi_id_usize(0) == 0);
    assert(pruv_abi_id_usize(UINT64_MAX) == UINT64_MAX);
    assert(pruv_abi_id_i8(INT8_MIN) == INT8_MIN);
    assert(pruv_abi_id_i8(INT8_MAX) == INT8_MAX);
    assert(pruv_abi_id_i16(INT16_MIN) == INT16_MIN);
    assert(pruv_abi_id_i16(INT16_MAX) == INT16_MAX);
    assert(pruv_abi_id_i32(INT32_MIN) == INT32_MIN);
    assert(pruv_abi_id_i32(INT32_MAX) == INT32_MAX);
    assert(pruv_abi_id_isize(INT64_MIN) == INT64_MIN);
    assert(pruv_abi_id_isize(INT64_MAX) == INT64_MAX);
    assert(pruv_abi_positive(UINT64_MAX) == UINT64_MAX);
    int64_t words[] = {-1, 0x18000, -1}, out[3];
    pruv_abi_round_trip(words, out);
    assert(out[0] == 255 && out[1] == INT16_MIN && out[2] == -1);
    uint16_t sid[] = {0xD800, 0xFFFF};
    assert(pruv_abi_length(sid, 2) == 2);
    PruvArrayU16 result;
    pruv_abi_units(255, -1, UINT64_MAX, NULL, 0, &result);
    assert(result.len == 0);
    pruv_abi_units(255, -1, UINT64_MAX, sid, 2, &result);
    assert(result.len == 2 && result.data[0] == sid[0] && result.data[1] == sid[1]);
    pruv_free_v2(result.data, result.len, 2, 2);
    return 0;
}
