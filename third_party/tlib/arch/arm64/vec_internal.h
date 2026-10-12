/*
 * ARM AdvSIMD / SVE Vector Helpers
 *
 * Copyright (c) 2020 Linaro
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, see <http://www.gnu.org/licenses/>.
 */

#pragma once

static inline void clear_tail(void *vd, uintptr_t opr_sz, uintptr_t max_sz)
{
    uint64_t *d = vd + opr_sz;
    uintptr_t i;

    for(i = opr_sz; i < max_sz; i += 8) {
        *d++ = 0;
    }
}

int8_t do_sqrdmlah_b(int8_t, int8_t, int8_t, bool, bool);
int16_t do_sqrdmlah_h(int16_t, int16_t, int16_t, bool, bool, uint32_t *);
int32_t do_sqrdmlah_s(int32_t, int32_t, int32_t, bool, bool, uint32_t *);
int64_t do_sqrdmlah_d(int64_t, int64_t, int64_t, bool, bool);

/*
 * 8 x 8 -> 16 vector polynomial multiply where the inputs are
 * in the low 8 bits of each 16-bit element
 */
uint64_t pmull_h(uint64_t op1, uint64_t op2);
/*
 * 16 x 16 -> 32 vector polynomial multiply where the inputs are
 * in the low 16 bits of each 32-bit element
 */
uint64_t pmull_w(uint64_t op1, uint64_t op2);

/**
 * bfdotadd:
 * @sum: addend
 * @e1, @e2: multiplicand vectors
 *
 * BFloat16 2-way dot product of @e1 & @e2, accumulating with @sum.
 * The @e1 and @e2 operands correspond to the 32-bit source vector
 * slots and contain two Bfloat16 values each.
 *
 * Corresponds to the ARM pseudocode function BFDotAdd.
 */
float32 bfdotadd(float32 sum, uint32_t e1, uint32_t e2);
