#pragma once

#include <stdint.h>
#include <stdbool.h>
#include "bit_helper.h"
#include "cpu_common.h"

/*
 * Note that vector data is stored in host-endian 64-bit chunks,
 * so addressing units smaller than that needs a host-endian fixup.
 *
 * The H<N> macros are used when indexing an array of elements of size N.
 *
 * The H1_<N> macros are used when performing byte arithmetic and then
 * casting the final pointer to a type of size N.
 */
#if HOST_WORDS_BIGENDIAN
#define H1(x)   ((x) ^ 7)
#define H1_2(x) ((x) ^ 6)
#define H1_4(x) ((x) ^ 4)
#define H2(x)   ((x) ^ 3)
#define H4(x)   ((x) ^ 1)
#else
#define H1(x)   (x)
#define H1_2(x) (x)
#define H1_4(x) (x)
#define H2(x)   (x)
#define H4(x)   (x)
#endif

/*
 * Access to 64-bit elements isn't host-endian dependent; we provide H8
 * and H1_8 so that when a function is being generated from a macro we
 * can pass these rather than an empty macro argument, for clarity.
 */
#define H8(x)   (x)
#define H1_8(x) (x)

/*
 * Expand active predicate bits to bytes, for byte elements.
 */
extern const uint64_t expand_pred_b_data[256];
static inline uint64_t expand_pred_b(uint8_t byte)
{
    return expand_pred_b_data[byte];
}

/* Similarly for half-word elements. */
extern const uint64_t expand_pred_h_data[0x55 + 1];
static inline uint64_t expand_pred_h(uint8_t byte)
{
    return expand_pred_h_data[byte & 0x55];
}

/* Similarly for single word elements.  */
static inline uint64_t expand_pred_s(uint8_t byte)
{
    static const uint64_t word[] = {
        [0x01] = 0x00000000ffffffffull,
        [0x10] = 0xffffffff00000000ull,
        [0x11] = 0xffffffffffffffffull,
    };
    return word[byte & 0x11];
}

static inline uint64_t do_urshr(uint64_t x, unsigned sh)
{
    if(likely(sh < 64)) {
        return (x >> sh) + ((x >> (sh - 1)) & 1);
    } else if(sh == 64) {
        return x >> 63;
    } else {
        return 0;
    }
}

static inline int64_t do_srshr(int64_t x, unsigned sh)
{
    if(likely(sh < 64)) {
        return (x >> sh) + ((x >> (sh - 1)) & 1);
    } else {
        /* Rounding the sign bit always produces 0. */
        return 0;
    }
}

int32_t do_sqrshl_bhs(int32_t src, int32_t shift, int bits, bool round, uint32_t *sat);
int64_t do_sqrshl_d(int64_t src, int64_t shift, bool round, uint32_t *sat);

uint32_t do_uqrshl_bhs(uint32_t src, int32_t shift, int bits, bool round, uint32_t *sat);
uint64_t do_uqrshl_d(uint64_t src, int64_t shift, bool round, uint32_t *sat);

int32_t do_suqrshl_bhs(int32_t src, int32_t shift, int bits, bool round, uint32_t *sat);
int64_t do_suqrshl_d(int64_t src, int64_t shift, bool round, uint32_t *sat);

uint64_t pmull_w(uint64_t op1, uint64_t op2);
uint64_t pmull_h(uint64_t op1, uint64_t op2);
