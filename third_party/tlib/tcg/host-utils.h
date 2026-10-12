/*
 * Utility compute operations used by translated code.
 *
 * Copyright (c) 2007 Thiemo Seufer
 * Copyright (c) 2007 Jocelyn Mayer
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in
 * all copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL
 * THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
 * THE SOFTWARE.
 */

#pragma once

#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "bswap.h"

#ifndef glue
#define xglue(x, y)  x##y
#define glue(x, y)   xglue(x, y)
#define stringify(s) tostring(s)
#define tostring(s)  #s
#endif

#ifndef ARRAY_SIZE
#define ARRAY_SIZE(x) (sizeof(x) / sizeof((x)[0]))
#endif

#if defined(TCG_TARGET_I386) && HOST_LONG_BITS == 32
#define REGPARM __attribute((regparm(3)))
#else
#define REGPARM
#endif

//  GCCs older than v10.1 don't support the '__has_builtin' macro but support some builtins.
#ifndef __has_builtin
#ifdef __GNUC__
#define __has_builtin(x) _has##x
#define _gcc_version     (__GNUC__ * 100 + __GNUC_MINOR__)

//  Supported since GCC v3.4 (commit SHA: 2928cd7a).
#define _has__builtin_clz        (_gcc_version >= 304)
#define _has__builtin_clzll      (_gcc_version >= 304)
#define _has__builtin_ctz        (_gcc_version >= 304)
#define _has__builtin_ctzll      (_gcc_version >= 304)
#define _has__builtin_popcountll (_gcc_version >= 304)

//  Supported since GCC v4.7 (commit SHA: 3801c801).
#define _has__builtin_clrsb   (_gcc_version >= 407)
#define _has__builtin_clrsbll (_gcc_version >= 407)

//  Supported since GCC v5.1 (commit SHA: 1304953e).
#define _has__builtin_add_overflow (_gcc_version >= 501)
#define _has__builtin_sub_overflow (_gcc_version >= 501)

//  Currently not supported by GCC.
#define _has__builtin_bitreverse8  0
#define _has__builtin_bitreverse16 0
#define _has__builtin_bitreverse32 0
#define _has__builtin_bitreverse64 0
#else
#define __has_builtin(x) 0
#endif
#endif

#if defined(TCG_TARGET_I386) && HOST_LONG_BITS == 64
#define __HAVE_FAST_MULU64__
static inline void mulu64(uint64_t *plow, uint64_t *phigh, uint64_t a, uint64_t b)
{
    __asm__("mul %0\n\t" : "=d"(*phigh), "=a"(*plow) : "a"(a), "0"(b));
}
#define __HAVE_FAST_MULS64__
static inline void muls64(uint64_t *plow, uint64_t *phigh, int64_t a, int64_t b)
{
    __asm__("imul %0\n\t" : "=d"(*phigh), "=a"(*plow) : "a"(a), "0"(b));
}
#else
void muls64(uint64_t *plow, uint64_t *phigh, int64_t a, int64_t b);
void mulu64(uint64_t *plow, uint64_t *phigh, uint64_t a, uint64_t b);
#endif

/* Binary search for leading zeros.  */

static inline int clz32(uint32_t val)
{
    if(!val) {
        return 32;
    }

#if __has_builtin(__builtin_clz)
    return __builtin_clz(val);
#else
    int cnt = 0;

    if(!(val & 0xFFFF0000U)) {
        cnt += 16;
        val <<= 16;
    }
    if(!(val & 0xFF000000U)) {
        cnt += 8;
        val <<= 8;
    }
    if(!(val & 0xF0000000U)) {
        cnt += 4;
        val <<= 4;
    }
    if(!(val & 0xC0000000U)) {
        cnt += 2;
        val <<= 2;
    }
    if(!(val & 0x80000000U)) {
        cnt++;
        val <<= 1;
    }
    if(!(val & 0x80000000U)) {
        cnt++;
    }
    return cnt;
#endif
}

static inline int clz64(uint64_t val)
{
    if(!val) {
        return 64;
    }

#if __has_builtin(__builtin_clzll)
    return __builtin_clzll(val);
#else
    int cnt = 0;

    if(!(val >> 32)) {
        cnt += 32;
    } else {
        val >>= 32;
    }

    return cnt + clz32(val);
#endif
}

static inline int ctz32(uint32_t val)
{
    if(!val) {
        return 32;
    }

#if __has_builtin(__builtin_ctz)
    return __builtin_ctz(val);
#else
    int cnt;

    cnt = 0;
    if(!(val & 0x0000FFFFUL)) {
        cnt += 16;
        val >>= 16;
    }
    if(!(val & 0x000000FFUL)) {
        cnt += 8;
        val >>= 8;
    }
    if(!(val & 0x0000000FUL)) {
        cnt += 4;
        val >>= 4;
    }
    if(!(val & 0x00000003UL)) {
        cnt += 2;
        val >>= 2;
    }
    if(!(val & 0x00000001UL)) {
        cnt++;
        val >>= 1;
    }
    if(!(val & 0x00000001UL)) {
        cnt++;
    }

    return cnt;
#endif
}

static inline int ctz64(uint64_t val)
{
    if(!val) {
        return 64;
    }

#if __has_builtin(__builtin_ctzll)
    return __builtin_ctzll(val);
#else
    int cnt;

    cnt = 0;
    if(!((uint32_t)val)) {
        cnt += 32;
        val >>= 32;
    }

    return cnt + ctz32(val);
#endif
}

#if TARGET_LONG_BITS == 64
#define ctz_tl ctz64
#elif TARGET_LONG_BITS == 32
#define ctz_tl ctz32
#endif

static inline int ctpop64(uint64_t val)
{
#if __has_builtin(__builtin_popcountll)
    return __builtin_popcountll(val);
#else
    val = (val & 0x5555555555555555ULL) + ((val >> 1) & 0x5555555555555555ULL);
    val = (val & 0x3333333333333333ULL) + ((val >> 2) & 0x3333333333333333ULL);
    val = (val & 0x0f0f0f0f0f0f0f0fULL) + ((val >> 4) & 0x0f0f0f0f0f0f0f0fULL);
    val = (val & 0x00ff00ff00ff00ffULL) + ((val >> 8) & 0x00ff00ff00ff00ffULL);
    val = (val & 0x0000ffff0000ffffULL) + ((val >> 16) & 0x0000ffff0000ffffULL);
    val = (val & 0x00000000ffffffffULL) + ((val >> 32) & 0x00000000ffffffffULL);

    return val;
#endif
}

/**
 * clrsb32 - count leading redundant sign bits in a 32-bit value.
 * @val: The value to search
 *
 * Returns the number of bits following the sign bit that are equal to it.
 * No special cases; output range is [0-31].
 */
static inline int clrsb32(uint32_t val)
{
#if __has_builtin(__builtin_clrsb)
    return __builtin_clrsb(val);
#else
    return clz32(val ^ ((int32_t)val >> 1)) - 1;
#endif
}

/**
 * clrsb64 - count leading redundant sign bits in a 64-bit value.
 * @val: The value to search
 *
 * Returns the number of bits following the sign bit that are equal to it.
 * No special cases; output range is [0-63].
 */
static inline int clrsb64(uint64_t val)
{
#if __has_builtin(__builtin_clrsbll)
    return __builtin_clrsbll(val);
#else
    return clz64(val ^ ((int64_t)val >> 1)) - 1;
#endif
}

/**
 * revbit8 - reverse the bits in an 8-bit value.
 * @x: The value to modify.
 */
static inline uint8_t revbit8(uint8_t x)
{
#if __has_builtin(__builtin_bitreverse8)
    return __builtin_bitreverse8(x);
#else
    /* Assign the correct nibble position.  */
    x = ((x & 0xf0) >> 4) | ((x & 0x0f) << 4);
    /* Assign the correct bit position.  */
    x = ((x & 0x88) >> 3) | ((x & 0x44) >> 1) | ((x & 0x22) << 1) | ((x & 0x11) << 3);
    return x;
#endif
}

/**
 * revbit16 - reverse the bits in a 16-bit value.
 * @x: The value to modify.
 */
static inline uint16_t revbit16(uint16_t x)
{
#if __has_builtin(__builtin_bitreverse16)
    return __builtin_bitreverse16(x);
#else
    /* Assign the correct byte position.  */
    x = bswap16(x);
    /* Assign the correct nibble position.  */
    x = ((x & 0xf0f0) >> 4) | ((x & 0x0f0f) << 4);
    /* Assign the correct bit position.  */
    x = ((x & 0x8888) >> 3) | ((x & 0x4444) >> 1) | ((x & 0x2222) << 1) | ((x & 0x1111) << 3);
    return x;
#endif
}

/**
 * revbit32 - reverse the bits in a 32-bit value.
 * @x: The value to modify.
 */
static inline uint32_t revbit32(uint32_t x)
{
#if __has_builtin(__builtin_bitreverse32)
    return __builtin_bitreverse32(x);
#else
    /* Assign the correct byte position.  */
    x = bswap32(x);
    /* Assign the correct nibble position.  */
    x = ((x & 0xf0f0f0f0u) >> 4) | ((x & 0x0f0f0f0fu) << 4);
    /* Assign the correct bit position.  */
    x = ((x & 0x88888888u) >> 3) | ((x & 0x44444444u) >> 1) | ((x & 0x22222222u) << 1) | ((x & 0x11111111u) << 3);
    return x;
#endif
}

/**
 * revbit64 - reverse the bits in a 64-bit value.
 * @x: The value to modify.
 */
static inline uint64_t revbit64(uint64_t x)
{
#if __has_builtin(__builtin_bitreverse64)
    return __builtin_bitreverse64(x);
#else
    /* Assign the correct byte position.  */
    x = bswap64(x);
    /* Assign the correct nibble position.  */
    x = ((x & 0xf0f0f0f0f0f0f0f0ull) >> 4) | ((x & 0x0f0f0f0f0f0f0f0full) << 4);
    /* Assign the correct bit position.  */
    x = ((x & 0x8888888888888888ull) >> 3) | ((x & 0x4444444444444444ull) >> 1) | ((x & 0x2222222222222222ull) << 1) |
        ((x & 0x1111111111111111ull) << 3);
    return x;
#endif
}

/**
 * sadd32_overflow - addition with overflow indication
 * @x, @y: addends
 * @ret: Output for sum
 *
 * Computes *@ret = @x + @y, and returns true if and only if that
 * value has been truncated.
 */
static inline bool sadd32_overflow(int32_t x, int32_t y, int32_t *ret)
{
#if __has_builtin(__builtin_add_overflow)
    return __builtin_add_overflow(x, y, ret);
#else
    *ret = x + y;
    return ((*ret ^ x) & ~(x ^ y)) < 0;
#endif
}

/**
 * sadd64_overflow - addition with overflow indication
 * @x, @y: addends
 * @ret: Output for sum
 *
 * Computes *@ret = @x + @y, and returns true if and only if that
 * value has been truncated.
 */
static inline bool sadd64_overflow(int64_t x, int64_t y, int64_t *ret)
{
#if __has_builtin(__builtin_add_overflow)
    return __builtin_add_overflow(x, y, ret);
#else
    *ret = x + y;
    return ((*ret ^ x) & ~(x ^ y)) < 0;
#endif
}

/**
 * uadd32_overflow - addition with overflow indication
 * @x, @y: addends
 * @ret: Output for sum
 *
 * Computes *@ret = @x + @y, and returns true if and only if that
 * value has been truncated.
 */
static inline bool uadd32_overflow(uint32_t x, uint32_t y, uint32_t *ret)
{
#if __has_builtin(__builtin_add_overflow)
    return __builtin_add_overflow(x, y, ret);
#else
    *ret = x + y;
    return *ret < x;
#endif
}

/**
 * uadd64_overflow - addition with overflow indication
 * @x, @y: addends
 * @ret: Output for sum
 *
 * Computes *@ret = @x + @y, and returns true if and only if that
 * value has been truncated.
 */
static inline bool uadd64_overflow(uint64_t x, uint64_t y, uint64_t *ret)
{
#if __has_builtin(__builtin_add_overflow)
    return __builtin_add_overflow(x, y, ret);
#else
    *ret = x + y;
    return *ret < x;
#endif
}

/**
 * ssub32_overflow - subtraction with overflow indication
 * @x: Minuend
 * @y: Subtrahend
 * @ret: Output for difference
 *
 * Computes *@ret = @x - @y, and returns true if and only if that
 * value has been truncated.
 */
static inline bool ssub32_overflow(int32_t x, int32_t y, int32_t *ret)
{
#if __has_builtin(__builtin_sub_overflow)
    return __builtin_sub_overflow(x, y, ret);
#else
    *ret = x - y;
    return ((*ret ^ x) & (x ^ y)) < 0;
#endif
}

/**
 * ssub64_overflow - subtraction with overflow indication
 * @x: Minuend
 * @y: Subtrahend
 * @ret: Output for sum
 *
 * Computes *@ret = @x - @y, and returns true if and only if that
 * value has been truncated.
 */
static inline bool ssub64_overflow(int64_t x, int64_t y, int64_t *ret)
{
#if __has_builtin(__builtin_sub_overflow)
    return __builtin_sub_overflow(x, y, ret);
#else
    *ret = x - y;
    return ((*ret ^ x) & (x ^ y)) < 0;
#endif
}

/**
 * usub32_overflow - subtraction with overflow indication
 * @x: Minuend
 * @y: Subtrahend
 * @ret: Output for sum
 *
 * Computes *@ret = @x - @y, and returns true if and only if that
 * value has been truncated.
 */
static inline bool usub32_overflow(uint32_t x, uint32_t y, uint32_t *ret)
{
#if __has_builtin(__builtin_sub_overflow)
    return __builtin_sub_overflow(x, y, ret);
#else
    *ret = x - y;
    return x < y;
#endif
}

/**
 * usub64_overflow - subtraction with overflow indication
 * @x: Minuend
 * @y: Subtrahend
 * @ret: Output for sum
 *
 * Computes *@ret = @x - @y, and returns true if and only if that
 * value has been truncated.
 */
static inline bool usub64_overflow(uint64_t x, uint64_t y, uint64_t *ret)
{
#if __has_builtin(__builtin_sub_overflow)
    return __builtin_sub_overflow(x, y, ret);
#else
    *ret = x - y;
    return x < y;
#endif
}
