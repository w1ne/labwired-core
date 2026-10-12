/*
 * Tiny Code Generator for QEMU
 *
 * Copyright (c) 2008 Fabrice Bellard
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

#include <stdint.h>
#include "host-utils.h"

/* 32-bit helpers */

uint32_t tcg_helper_clrsb_i32(uint32_t arg)
{
    return clrsb32(arg);
}

uint32_t tcg_helper_clz_i32(uint32_t arg, uint32_t zero_val)
{
    return arg ? clz32(arg) : zero_val;
}

int32_t tcg_helper_div_i32(int32_t arg1, int32_t arg2)
{
    return arg1 / arg2;
}

int32_t tcg_helper_rem_i32(int32_t arg1, int32_t arg2)
{
    return arg1 % arg2;
}

uint32_t tcg_helper_divu_i32(uint32_t arg1, uint32_t arg2)
{
    return arg1 / arg2;
}

uint32_t tcg_helper_remu_i32(uint32_t arg1, uint32_t arg2)
{
    return arg1 % arg2;
}

/* 64-bit helpers */

uint64_t tcg_helper_shl_i64(uint64_t arg1, uint64_t arg2)
{
    return arg1 << arg2;
}

uint64_t tcg_helper_shr_i64(uint64_t arg1, uint64_t arg2)
{
    return arg1 >> arg2;
}

int64_t tcg_helper_sar_i64(int64_t arg1, int64_t arg2)
{
    return arg1 >> arg2;
}

int64_t tcg_helper_div_i64(int64_t arg1, int64_t arg2)
{
    return arg1 / arg2;
}

int64_t tcg_helper_rem_i64(int64_t arg1, int64_t arg2)
{
    return arg1 % arg2;
}

int64_t tcg_helper_mulsh_i64(int64_t arg1, int64_t arg2)
{
    uint64_t l, h;
    muls64(&l, &h, arg1, arg2);
    return h;
}

uint64_t tcg_helper_divu_i64(uint64_t arg1, uint64_t arg2)
{
    return arg1 / arg2;
}

uint64_t tcg_helper_remu_i64(uint64_t arg1, uint64_t arg2)
{
    return arg1 % arg2;
}

uint64_t tcg_helper_muluh_i64(uint64_t arg1, uint64_t arg2)
{
    uint64_t l, h;
    mulu64(&l, &h, arg1, arg2);
    return h;
}

uint64_t tcg_helper_clrsb_i64(uint64_t arg)
{
    return clrsb64(arg);
}

uint64_t tcg_helper_clz_i64(uint64_t arg, uint64_t zero_val)
{
    return arg ? clz64(arg) : zero_val;
}
