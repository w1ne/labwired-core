/*
 * Copyright (c) Antmicro
 *
 * SPDX-License-Identifier: Apache-2.0
 */

#pragma once

#include <stdint.h>

#include "infrastructure.h"

#define MIN(a, b) (((a) < (b)) ? (a) : (b))
#define MAX(a, b) (((a) > (b)) ? (a) : (b))

//  Used by deposit32.
static inline uint64_t deposit64(uint64_t dst_val, uint8_t start, uint8_t length, uint64_t val);

/* Deposits 'length' bits from 'val' into 'dst_val' at 'start' bit. */
static inline uint32_t deposit32(uint32_t dst_val, uint8_t start, uint8_t length, uint32_t val)
{
    tlib_assert(start + length <= 32);
    return deposit64(dst_val, start, length, val);
}

/* Deposits 'length' bits from 'val' into 'dst_val' at 'start' bit. */
static inline uint64_t deposit64(uint64_t dst_val, uint8_t start, uint8_t length, uint64_t val)
{
    tlib_assert(start + length <= 64);

    //  Number with only relevant bits ('start' to 'start+length-1') set.
    uint64_t relevant_bits = (UINT64_MAX >> (64 - length)) << start;

    //  Shift value into the start bit and reset the irrelevant bits.
    val = (val << start) & relevant_bits;

    //  Reset the relevant bits in the destination value.
    dst_val &= ~relevant_bits;

    //  Return the above values merged.
    return dst_val | val;
}

/* Extracts 'length' bits of 'value' at 'start' bit. */
static inline uint32_t extract16(uint16_t value, uint8_t start, uint8_t length)
{
    tlib_assert(start + length <= 16);
    return (value >> start) & (UINT16_MAX >> (16 - length));
}

/* Extracts 'length' bits of 'value' at 'start' bit. */
static inline uint32_t extract32(uint32_t value, uint8_t start, uint8_t length)
{
    tlib_assert(start + length <= 32);
    return (value >> start) & (UINT32_MAX >> (32 - length));
}

/* Extracts 'length' bits of 'value' at 'start' bit. */
static inline uint64_t extract64(uint64_t value, uint8_t start, uint8_t length)
{
    tlib_assert(start + length <= 64);
    return (value >> start) & (UINT64_MAX >> (64 - length));
}

static inline int32_t sextract32(uint32_t value, uint8_t start, uint8_t length)
{
    int32_t result = (int32_t)extract32(value, start, length);
    if(result >> (length - 1)) {
        //  Set all the bits from 'length' to bit 31.
        result |= UINT32_MAX << length;
    }
    return result;
}

static inline int64_t sextract64(uint64_t value, uint8_t start, uint8_t length)
{
    int64_t result = (int64_t)extract64(value, start, length);
    if(result >> (length - 1)) {
        //  Set all the bits from 'length' to bit 63.
        result |= UINT64_MAX << length;
    }
    return result;
}

static inline uint8_t rol8(uint8_t word, unsigned int shift)
{
    return (word << shift) | (word >> ((8 - shift) & 7));
}

static inline uint16_t rol16(uint16_t word, unsigned int shift)
{
    return (word << shift) | (word >> ((16 - shift) & 15));
}

static inline uint32_t rol32(uint32_t word, unsigned int shift)
{
    return (word << shift) | (word >> ((32 - shift) & 31));
}

static inline uint64_t rol64(uint64_t word, unsigned int shift)
{
    return (word << shift) | (word >> ((64 - shift) & 63));
}

static inline uint8_t ror8(uint8_t word, unsigned int shift)
{
    return (word >> shift) | (word << ((8 - shift) & 7));
}

static inline uint16_t ror16(uint16_t word, unsigned int shift)
{
    return (word >> shift) | (word << ((16 - shift) & 15));
}

static inline uint32_t ror32(uint32_t word, unsigned int shift)
{
    return (word >> shift) | (word << ((32 - shift) & 31));
}

static inline uint64_t ror64(uint64_t word, unsigned int shift)
{
    return (word >> shift) | (word << ((64 - shift) & 63));
}

static inline uint32_t ctpop(uint32_t val)
{
    int cnt = 0;
    while(val > 0) {
        cnt += val & 1;
        val >>= 1;
    }
    return cnt;
}

static inline uint8_t ctpop8(uint8_t val)
{
    return (uint8_t)ctpop(val);
}

static inline uint16_t ctpop16(uint16_t val)
{
    return (uint16_t)ctpop(val);
}

static inline uint32_t ctpop32(uint32_t val)
{
    return (uint32_t)ctpop(val);
}

static inline uint32_t brev32(uint32_t val)
{
    return ((val >> 24) & 0x000000FF) | ((val >> 8) & 0x0000FF00) | ((val << 8) & 0x00FF0000) | ((val << 24) & 0xFF000000);
}

//  Source: https://stackoverflow.com/a/21507710
static inline uint64_t brev64(uint64_t val)
{
    val = (val & 0x00000000FFFFFFFF) << 32 | (val & 0xFFFFFFFF00000000) >> 32;
    val = (val & 0x0000FFFF0000FFFF) << 16 | (val & 0xFFFF0000FFFF0000) >> 16;
    val = (val & 0x00FF00FF00FF00FF) << 8 | (val & 0xFF00FF00FF00FF00) >> 8;
    return val;
}

static inline uint32_t clmul32(uint32_t val1, uint32_t val2)
{
    uint32_t result = 0u;
    int i;
    for(i = 0; i < 32; ++i) {
        if((val2 >> i) & 1) {
            result ^= (uint32_t)val1 << i;
        }
    }
    return result;
}

static inline uint64_t clmul64(uint64_t val1, uint64_t val2)
{
    uint64_t result = 0u;
    int i;
    for(i = 0; i < 64; ++i) {
        if((val2 >> i) & 1) {
            result ^= (uint64_t)val1 << i;
        }
    }
    return result;
}

static inline uint32_t clmulh32(uint32_t val1, uint32_t val2)
{
    uint32_t result = 0u;
    int i;
    for(i = 1; i < 32; ++i) {
        if((val2 >> i) & 1) {
            result ^= (uint32_t)val1 >> (32 - i);
        }
    }
    return result;
}

static inline uint64_t clmulh64(uint64_t val1, uint64_t val2)
{
    uint64_t result = 0u;
    int i;
    for(i = 1; i < 64; ++i) {
        if((val2 >> i) & 1) {
            result ^= (uint64_t)val1 >> (64 - i);
        }
    }
    return result;
}

static inline uint32_t clmulr32(uint32_t val1, uint32_t val2)
{
    uint32_t result = 0u;
    int i;
    for(i = 0; i < 32; ++i) {
        if((val2 >> i) & 1) {
            result ^= (uint32_t)val1 >> (32 - i - 1);
        }
    }
    return result;
}

static inline uint64_t clmulr64(uint64_t val1, uint64_t val2)
{
    uint64_t result = 0u;
    int i;
    for(i = 0; i < 64; ++i) {
        if((val2 >> i) & 1) {
            result ^= (uint64_t)val1 >> (64 - i - 1);
        }
    }
    return result;
}

static inline uint32_t orcb32(uint32_t val)
{
    return ((uint8_t)(val >> 24) > 0 ? 0xFF000000 : 0x00000000) | ((uint8_t)(val >> 16) > 0 ? 0x00FF0000 : 0x00000000) |
           ((uint8_t)(val >> 8) > 0 ? 0x0000FF00 : 0x00000000) | ((uint8_t)(val) > 0 ? 0x000000FF : 0x00000000);
}

static inline uint64_t orcb64(uint64_t val)
{
    return (uint64_t)orcb32((uint32_t)(val >> 32)) << 32 | orcb32((uint32_t)val);
}

static inline int is_power_of_2(uint64_t val)
{
    return ((val - 1) & val) == 0;
}
