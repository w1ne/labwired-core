/*
 * Copyright (c) Antmicro
 *
 * SPDX-License-Identifier: Apache-2.0
 */

#pragma once

#include "stdint.h"
#include "stdbool.h"

static inline __int128_t int128_add(const __int128_t a, const __int128_t b)
{
    return a + b;
}

static inline bool int128_eq(const __int128_t a, const __int128_t b)
{
    return a == b;
};

static inline __int128_t int128_exts64(const int64_t a)
{
    return (__int128_t)a;
}

static inline int64_t int128_gethi(const __int128_t a)
{
    return (int64_t)(a >> 64);
}

static inline uint64_t int128_getlo(const __int128_t a)
{
    return (uint64_t)a;
}

static inline __int128_t int128_lshift(const __int128_t a, const uint8_t shift)
{
    return a << shift;
}

static inline __int128_t int128_make128(const uint64_t low, const uint64_t high)
{
    return (__int128_t)(((__int128_t)(high) << 64) | low);
}

static inline __int128_t int128_make64(const uint64_t a)
{
    return (__int128_t)a;
}

static inline __int128_t int128_neg(const __int128_t a)
{
    return -a;
}

static inline __int128_t int128_rshift(const __int128_t a, const uint8_t shift)
{
    return a >> shift;
}
