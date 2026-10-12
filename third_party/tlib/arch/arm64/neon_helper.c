/*
 * Neon instructions implementation for ARM.
 *
 *  Copyright (c) Antmicro
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2 of the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, see <http://www.gnu.org/licenses/>.
 */
#include "cpu.h"
#include "helper.h"
#include <math.h>

#define S32_1(x) ((int32_t)((x >> 32) & 0xffffffffu))
#define S32_0(x) ((int32_t)(x & 0xffffffffu))

#define U32_1(x) ((uint32_t)((x >> 32) & 0xffffffffu))
#define U32_0(x) ((uint32_t)(x & 0xffffffffu))

#define S16_3(x) ((int16_t)((x >> 48) & 0xffffu))
#define S16_2(x) ((int16_t)((x >> 32) & 0xffffu))
#define S16_1(x) ((int16_t)((x >> 16) & 0xffffu))
#define S16_0(x) ((int16_t)(x & 0xffffu))

#define U16_3(x) ((uint16_t)((x >> 48) & 0xffffu))
#define U16_2(x) ((uint16_t)((x >> 32) & 0xffffu))
#define U16_1(x) ((uint16_t)((x >> 16) & 0xffffu))
#define U16_0(x) ((uint16_t)(x & 0xffffu))

#define U8_7(x) ((uint8_t)((x >> 56) & 0xff))
#define U8_6(x) ((uint8_t)((x >> 48) & 0xff))
#define U8_5(x) ((uint8_t)((x >> 40) & 0xff))
#define U8_4(x) ((uint8_t)((x >> 32) & 0xff))
#define U8_3(x) ((uint8_t)((x >> 24) & 0xff))
#define U8_2(x) ((uint8_t)((x >> 16) & 0xff))
#define U8_1(x) ((uint8_t)((x >> 8) & 0xff))
#define U8_0(x) ((uint8_t)(x & 0xff))

#define S8_3(x) ((int8_t)((x >> 24) & 0xff))
#define S8_2(x) ((int8_t)((x >> 16) & 0xff))
#define S8_1(x) ((int8_t)((x >> 8) & 0xff))
#define S8_0(x) ((int8_t)(x & 0xff))

enum operation { ADD, SUB };

enum flags {
    UNSIGNED = 1,
    SATURATING = 1 << 1,
};

static uint32_t qaddsub_8_common(CPUState *env, uint32_t a, uint32_t b, enum operation op, unsigned flags)
{
    int saturated = 0;

    int16_t a0 = (uint8_t)(a >> 24);
    int16_t a1 = (uint8_t)((a >> 16) & 0xff);
    int16_t a2 = (uint8_t)((a >> 8) & 0xff);
    int16_t a3 = (uint8_t)(a & 0xff);

    int16_t b0 = (uint8_t)(b >> 24);
    int16_t b1 = (uint8_t)((b >> 16) & 0xff);
    int16_t b2 = (uint8_t)((b >> 8) & 0xff);
    int16_t b3 = (uint8_t)(b & 0xff);

    const int isUnsigned = flags & UNSIGNED;

    if(!isUnsigned) {
        a0 = (int8_t)(uint8_t)a0;
        a1 = (int8_t)(uint8_t)a1;
        a2 = (int8_t)(uint8_t)a2;
        a3 = (int8_t)(uint8_t)a3;
        b0 = (int8_t)(uint8_t)b0;
        b1 = (int8_t)(uint8_t)b1;
        b2 = (int8_t)(uint8_t)b2;
        b3 = (int8_t)(uint8_t)b3;
    }

    int16_t out0 = op == SUB ? a0 - b0 : a0 + b0;
    int16_t out1 = op == SUB ? a1 - b1 : a1 + b1;
    int16_t out2 = op == SUB ? a2 - b2 : a2 + b2;
    int16_t out3 = op == SUB ? a3 - b3 : a3 + b3;

    if(flags & SATURATING) {
        const int16_t max = isUnsigned ? UINT8_MAX : INT8_MAX;
        const int16_t min = isUnsigned ? 0 : INT8_MIN;

        if(out0 > max) {
            saturated = 1;
            out0 = max;
        } else if(out0 < min) {
            saturated = 1;
            out0 = min;
        }

        if(out1 > max) {
            saturated = 1;
            out1 = max;
        } else if(out1 < min) {
            saturated = 1;
            out1 = min;
        }

        if(out2 > max) {
            saturated = 1;
            out2 = max;
        } else if(out2 < min) {
            saturated = 1;
            out2 = min;
        }

        if(out3 > max) {
            saturated = 1;
            out3 = max;
        } else if(out3 < min) {
            saturated = 1;
            out3 = min;
        }

        if(saturated) {
            env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        }
    }

    return (uint8_t)out0 << 24 | (uint8_t)out1 << 16 | (uint8_t)out2 << 8 | (uint8_t)out3;
}

static uint32_t qaddsub_16_common(CPUState *env, uint32_t a, uint32_t b, enum operation op, unsigned flags)
{
    int saturated = 0;

    int32_t aHi = (uint16_t)(a >> 16);
    int32_t aLo = (uint16_t)(a & 0xffffu);
    int32_t bHi = (uint16_t)(b >> 16);
    int32_t bLo = (uint16_t)(b & 0xffffu);

    const int isUnsigned = flags & UNSIGNED;

    if(!isUnsigned) {
        aHi = (int16_t)(uint16_t)aHi;
        aLo = (int16_t)(uint16_t)aLo;

        bHi = (int16_t)(uint16_t)bHi;
        bLo = (int16_t)(uint16_t)bLo;
    }

    int32_t outHi = op == SUB ? aHi - bHi : aHi + bHi;
    int32_t outLo = op == SUB ? aLo - bLo : aLo + bLo;

    if(flags & SATURATING) {
        const int32_t max = isUnsigned ? UINT16_MAX : INT16_MAX;
        const int32_t min = isUnsigned ? 0 : INT16_MIN;

        if(outHi > max) {
            saturated = 1;
            outHi = max;
        } else if(outHi < min) {
            saturated = 1;
            outHi = min;
        }

        if(outLo > max) {
            saturated = 1;
            outLo = max;
        } else if(outLo < min) {
            saturated = 1;
            outLo = min;
        }

        if(saturated) {
            env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        }
    }

    if(isUnsigned) {
        const uint32_t ret = (uint16_t)outHi << 16 | (uint16_t)outLo;
        return ret;
    } else {
        const int16_t outHi16 = outHi;
        const int16_t outLo16 = outLo;
        const uint32_t ret = ((uint16_t)outHi16) << 16 | (uint16_t)outLo16;
        return ret;
    }
}

static int8_t abs_s8(int8_t a)
{
    return abs(a);
}

uint32_t HELPER(neon_abs_s8)(uint32_t a)
{
    const uint8_t out0 = abs_s8(S8_3(a));
    const uint8_t out1 = abs_s8(S8_2(a));
    const uint8_t out2 = abs_s8(S8_1(a));
    const uint8_t out3 = abs_s8(S8_0(a));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static int16_t abs_s16(int16_t a)
{
    return abs(a);
}

uint32_t HELPER(neon_abs_s16)(uint32_t a)
{
    const uint16_t hi = abs_s16(S16_1(a));
    const uint16_t lo = abs_s16(S16_0(a));
    return (hi << 16) | lo;
}

static int8_t qabs_s8(CPUState *env, int8_t a)
{
    if(a == INT8_MIN) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return INT8_MAX;
    }

    return abs_s8(a);
}

uint32_t HELPER(neon_qabs_s8)(CPUState *env, uint32_t a)
{
    const uint8_t out0 = qabs_s8(env, S8_3(a));
    const uint8_t out1 = qabs_s8(env, S8_2(a));
    const uint8_t out2 = qabs_s8(env, S8_1(a));
    const uint8_t out3 = qabs_s8(env, S8_0(a));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static int16_t qabs_s16(CPUState *env, int16_t a)
{
    if(a == INT16_MIN) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return INT16_MAX;
    }

    return abs_s16(a);
}

uint32_t HELPER(neon_qabs_s16)(CPUState *env, uint32_t a)
{
    const uint16_t hi = qabs_s16(env, S16_1(a));
    const uint16_t lo = qabs_s16(env, S16_0(a));
    return (hi << 16) | lo;
}

uint32_t HELPER(neon_qabs_s32)(CPUState *env, uint32_t a)
{
    if(a == INT32_MIN) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return INT32_MAX;
    }

    return abs((int32_t)a);
}

static int8_t qneg_s8(CPUState *env, int8_t a)
{
    if(a == INT8_MIN) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return INT8_MAX;
    }

    return -a;
}

uint32_t HELPER(neon_qneg_s8)(CPUState *env, uint32_t a)
{
    const uint8_t out0 = qneg_s8(env, S8_3(a));
    const uint8_t out1 = qneg_s8(env, S8_2(a));
    const uint8_t out2 = qneg_s8(env, S8_1(a));
    const uint8_t out3 = qneg_s8(env, S8_0(a));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static int16_t qneg_s16(CPUState *env, int16_t a)
{
    if(a == INT16_MIN) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return INT16_MAX;
    }

    return -a;
}

uint32_t HELPER(neon_qneg_s16)(CPUState *env, uint32_t a)
{
    const uint16_t hi = qneg_s16(env, S16_1(a));
    const uint16_t lo = qneg_s16(env, S16_0(a));
    return (hi << 16) | lo;
}

uint32_t HELPER(neon_qneg_s32)(CPUState *env, uint32_t a)
{
    if(a == INT32_MIN) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return INT32_MAX;
    }

    return -a;
}

uint64_t HELPER(neon_negl_u16)(uint64_t a)
{
    const uint16_t out0 = -(int16_t)U16_3(a);
    const uint16_t out1 = -(int16_t)U16_2(a);
    const uint16_t out2 = -(int16_t)U16_1(a);
    const uint16_t out3 = -(int16_t)U16_0(a);

    return (uint64_t)out0 << 48 | (uint64_t)out1 << 32 | (uint64_t)out2 << 16 | (uint64_t)out3;
}

uint64_t HELPER(neon_negl_u32)(uint64_t a)
{
    const uint32_t out0 = -S32_1(a);
    const uint32_t out1 = -S32_0(a);

    return (uint64_t)out0 << 32 | (uint64_t)out1;
}

uint32_t HELPER(neon_abd_u8)(uint32_t a, uint32_t b)
{
    const uint8_t a0 = a >> 24;
    const uint8_t a1 = (a >> 16) & 0xff;
    const uint8_t a2 = (a >> 8) & 0xff;
    const uint8_t a3 = a & 0xff;

    const uint8_t b0 = b >> 24;
    const uint8_t b1 = (b >> 16) & 0xff;
    const uint8_t b2 = (b >> 8) & 0xff;
    const uint8_t b3 = b & 0xff;

    const uint8_t out0 = a0 > b0 ? a0 - b0 : b0 - a0;
    const uint8_t out1 = a1 > b1 ? a1 - b1 : b1 - a1;
    const uint8_t out2 = a2 > b2 ? a2 - b2 : b2 - a2;
    const uint8_t out3 = a3 > b3 ? a3 - b3 : b3 - a3;

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static uint8_t abd_s8(int8_t a, int8_t b)
{
    if(a > 0 && b < 0) {
        const int overflow = a > INT8_MAX + b;
        if(overflow) {
            return a - b;
        }
    } else if(a < 0 && b > 0) {
        const int overflow = a < INT8_MIN + b;
        if(overflow) {
            return b - a;
        }
    }

    return a > b ? a - b : b - a;
}

uint32_t HELPER(neon_abd_s8)(uint32_t a, uint32_t b)
{
    //  unsigned
    const uint8_t a0 = a >> 24;
    const uint8_t a1 = (a >> 16) & 0xff;
    const uint8_t a2 = (a >> 8) & 0xff;
    const uint8_t a3 = a & 0xff;

    const uint8_t b0 = b >> 24;
    const uint8_t b1 = (b >> 16) & 0xff;
    const uint8_t b2 = (b >> 8) & 0xff;
    const uint8_t b3 = b & 0xff;

    return abd_s8(a0, b0) << 24 | abd_s8(a1, b1) << 16 | abd_s8(a2, b2) << 8 | abd_s8(a3, b3);
}

uint32_t HELPER(neon_abd_u16)(uint32_t a, uint32_t b)
{
    const uint16_t aHi = a >> 16;
    const uint16_t aLo = a & 0xffffu;
    const uint16_t bHi = b >> 16;
    const uint16_t bLo = b & 0xffffu;

    const uint16_t outHi = aHi > bHi ? aHi - bHi : bHi - aHi;
    const uint16_t outLo = aLo > bLo ? aLo - bLo : bLo - aLo;

    return outHi << 16 | outLo;
}

static uint16_t abd_s16(int16_t a, int16_t b)
{
    if(a > 0 && b < 0) {
        const int overflow = a > INT16_MAX + b;
        if(overflow) {
            return a - b;
        }
    } else if(a < 0 && b > 0) {
        const int overflow = a < INT16_MIN + b;
        if(overflow) {
            return b - a;
        }
    }

    return a > b ? a - b : b - a;
}

uint32_t HELPER(neon_abd_s16)(uint32_t a, uint32_t b)
{
    //  unsigned
    const uint16_t aHi = a >> 16;
    const uint16_t aLo = a & 0xffffu;
    const uint16_t bHi = b >> 16;
    const uint16_t bLo = b & 0xffffu;

    return abd_s16(aHi, bHi) << 16 | abd_s16(aLo, bLo);
}

uint32_t HELPER(neon_abd_u32)(uint32_t a, uint32_t b)
{
    return a > b ? a - b : b - a;
}

uint32_t HELPER(neon_abd_s32)(int32_t a, int32_t b)
{
    if(a > 0 && b < 0) {
        const int overflow = a > INT32_MAX + b;
        if(overflow) {
            return a - b;
        }
    } else if(a < 0 && b > 0) {
        const int overflow = a < INT32_MIN + b;
        if(overflow) {
            return b - a;
        }
    }

    return a > b ? a - b : b - a;
}

uint32_t HELPER(neon_add_u8)(uint32_t a, uint32_t b)
{
    return qaddsub_8_common(NULL, a, b, ADD, UNSIGNED);
}

uint32_t HELPER(neon_add_u16)(uint32_t a, uint32_t b)
{
    return qaddsub_16_common(NULL, a, b, ADD, UNSIGNED);
}

uint32_t HELPER(neon_sub_u8)(uint32_t a, uint32_t b)
{
    return qaddsub_8_common(NULL, a, b, SUB, UNSIGNED);
}

uint32_t HELPER(neon_sub_u16)(uint32_t a, uint32_t b)
{
    return qaddsub_16_common(NULL, a, b, SUB, UNSIGNED);
}

uint32_t HELPER(neon_qadd_s8)(CPUState *env, uint32_t a, uint32_t b)
{
    return qaddsub_8_common(env, a, b, ADD, SATURATING);
}

uint32_t HELPER(neon_qadd_u8)(CPUState *env, uint32_t a, uint32_t b)
{
    return qaddsub_8_common(env, a, b, ADD, SATURATING | UNSIGNED);
}

uint32_t HELPER(neon_qadd_s16)(CPUState *env, uint32_t a, uint32_t b)
{
    return qaddsub_16_common(env, a, b, ADD, SATURATING);
}

uint32_t HELPER(neon_qadd_u16)(CPUState *env, uint32_t a, uint32_t b)
{
    return qaddsub_16_common(env, a, b, ADD, SATURATING | UNSIGNED);
}

uint32_t HELPER(neon_qadd_s32)(CPUState *env, uint32_t a, uint32_t b)
{
    const int32_t as = a;
    const int32_t bs = b;

    if(as > 0 && bs > 0) {
        const int saturated = bs > INT32_MAX - as;
        if(saturated) {
            env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
            return INT32_MAX;
        }
    } else if(as < 0 && bs < 0) {
        const int saturated = bs < INT32_MIN - as;
        if(saturated) {
            env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
            return INT32_MIN;
        }
    }

    return as + bs;
}

uint32_t HELPER(neon_qadd_u32)(CPUState *env, uint32_t a, uint32_t b)
{
    const int saturated = b > UINT32_MAX - a;

    if(saturated) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return UINT32_MAX;
    }

    return a + b;
}

uint64_t HELPER(neon_qadd_u64)(CPUState *env, uint64_t a, uint64_t b)
{
    const int saturated = b > UINT64_MAX - a;

    if(saturated) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return UINT64_MAX;
    }

    return a + b;
}

uint64_t HELPER(neon_qadd_s64)(CPUState *env, uint64_t a, uint64_t b)
{
    const int64_t as = a;
    const int64_t bs = b;

    if(as > 0 && bs > 0) {
        const int saturated = bs > INT64_MAX - as;
        if(saturated) {
            env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
            return INT64_MAX;
        }
    } else if(as < 0 && bs < 0) {
        const int saturated = bs < INT64_MIN - as;
        if(saturated) {
            env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
            return INT64_MIN;
        }
    }

    return as + bs;
}

uint32_t HELPER(neon_qsub_u8)(CPUState *env, uint32_t a, uint32_t b)
{
    return qaddsub_8_common(env, a, b, SUB, SATURATING | UNSIGNED);
}

uint32_t HELPER(neon_qsub_s8)(CPUState *env, uint32_t a, uint32_t b)
{
    return qaddsub_8_common(env, a, b, SUB, SATURATING);
}

uint32_t HELPER(neon_qsub_u16)(CPUState *env, uint32_t a, uint32_t b)
{
    return qaddsub_16_common(env, a, b, SUB, SATURATING | UNSIGNED);
}

uint32_t HELPER(neon_qsub_s16)(CPUState *env, uint32_t a, uint32_t b)
{
    return qaddsub_16_common(env, a, b, SUB, SATURATING);
}

uint32_t HELPER(neon_qsub_u32)(CPUState *env, uint32_t a, uint32_t b)
{
    const int saturated = b > a;

    if(saturated) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return 0;
    }

    return a - b;
}

uint32_t HELPER(neon_qsub_s32)(CPUState *env, uint32_t a, uint32_t b)
{
    const int32_t as = a;
    const int32_t bs = b;

    if(as > 0 && bs < 0) {
        const int saturated = as > INT32_MAX + bs;
        if(saturated) {
            env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
            return INT32_MAX;
        }
    } else if(as < 0 && bs > 0) {
        const int saturated = as < INT32_MIN + bs;
        if(saturated) {
            env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
            return INT32_MIN;
        }
    }

    return as - bs;
}

uint64_t HELPER(neon_qsub_u64)(CPUState *env, uint64_t a, uint64_t b)
{
    const int saturated = b > a;

    if(saturated) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return 0;
    }

    return a - b;
}

uint64_t HELPER(neon_qsub_s64)(CPUState *env, uint64_t a, uint64_t b)
{
    const int64_t as = a;
    const int64_t bs = b;

    if(as > 0 && bs < 0) {
        const int saturated = as > INT64_MAX + bs;
        if(saturated) {
            env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
            return INT64_MAX;
        }
    } else if(as < 0 && bs > 0) {
        const int saturated = as < INT64_MIN + bs;
        if(saturated) {
            env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
            return INT64_MIN;
        }
    }

    return as - bs;
}

static int8_t hadd_s8(int8_t a, int8_t b)
{
    const int16_t sum = (int16_t)a + (int16_t)b;
    return sum >= 0 ? sum / 2 : sum / 2 + sum % 2;
}

uint32_t HELPER(neon_hadd_s8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = hadd_s8(S8_3(a), S8_3(b));
    const uint8_t out1 = hadd_s8(S8_2(a), S8_2(b));
    const uint8_t out2 = hadd_s8(S8_1(a), S8_1(b));
    const uint8_t out3 = hadd_s8(S8_0(a), S8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static uint8_t hadd_u8(uint8_t a, uint8_t b)
{
    return ((uint16_t)a + (uint16_t)b) >> 1;
}

uint32_t HELPER(neon_hadd_u8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = hadd_u8(U8_3(a), U8_3(b));
    const uint8_t out1 = hadd_u8(U8_2(a), U8_2(b));
    const uint8_t out2 = hadd_u8(U8_1(a), U8_1(b));
    const uint8_t out3 = hadd_u8(U8_0(a), U8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static int16_t hadd_s16(int16_t a, int16_t b)
{
    const int32_t sum = (int32_t)a + (int32_t)b;
    return sum >= 0 ? sum / 2 : sum / 2 + sum % 2;
}

uint32_t HELPER(neon_hadd_s16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = hadd_s16(S16_1(a), S16_1(b));
    const uint16_t lo = hadd_s16(S16_0(a), S16_0(b));
    return (hi << 16) | lo;
}

static uint16_t hadd_u16(uint16_t a, uint16_t b)
{
    return ((uint32_t)a + (uint32_t)b) >> 1;
}

uint32_t HELPER(neon_hadd_u16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = hadd_u16(U16_1(a), U16_1(b));
    const uint16_t lo = hadd_u16(U16_0(a), U16_0(b));
    return (hi << 16) | lo;
}

int32_t HELPER(neon_hadd_s32)(int32_t a, int32_t b)
{
    const int64_t sum = (int64_t)a + (int64_t)b;
    return sum >= 0 ? sum / 2 : sum / 2 + sum % 2;
}

uint32_t HELPER(neon_hadd_u32)(uint32_t a, uint32_t b)
{
    return ((uint64_t)a + (uint64_t)b) >> 1;
}

static int8_t rhadd_s8(int8_t a, int8_t b)
{
    const int16_t sum = (int16_t)a + (int16_t)b;
    return sum >= 0 ? (sum + 1) / 2 : sum / 2;
}

uint32_t HELPER(neon_rhadd_s8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = rhadd_s8(S8_3(a), S8_3(b));
    const uint8_t out1 = rhadd_s8(S8_2(a), S8_2(b));
    const uint8_t out2 = rhadd_s8(S8_1(a), S8_1(b));
    const uint8_t out3 = rhadd_s8(S8_0(a), S8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static uint8_t rhadd_u8(uint8_t a, uint8_t b)
{
    return ((uint16_t)a + (uint16_t)b + 1) >> 1;
}

uint32_t HELPER(neon_rhadd_u8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = rhadd_u8(U8_3(a), U8_3(b));
    const uint8_t out1 = rhadd_u8(U8_2(a), U8_2(b));
    const uint8_t out2 = rhadd_u8(U8_1(a), U8_1(b));
    const uint8_t out3 = rhadd_u8(U8_0(a), U8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static int16_t rhadd_s16(int16_t a, int16_t b)
{
    const int32_t sum = (int32_t)a + (int32_t)b;
    return sum >= 0 ? (sum + 1) / 2 : sum / 2;
}

uint32_t HELPER(neon_rhadd_s16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = rhadd_s16(S16_1(a), S16_1(b));
    const uint16_t lo = rhadd_s16(S16_0(a), S16_0(b));
    return (hi << 16) | lo;
}

static uint16_t rhadd_u16(uint16_t a, uint16_t b)
{
    return ((uint32_t)a + (uint32_t)b + 1) >> 1;
}

uint32_t HELPER(neon_rhadd_u16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = rhadd_u16(U16_1(a), U16_1(b));
    const uint16_t lo = rhadd_u16(U16_0(a), U16_0(b));
    return (hi << 16) | lo;
}

int32_t HELPER(neon_rhadd_s32)(int32_t a, int32_t b)
{
    const int64_t sum = (int64_t)a + (int64_t)b;
    return sum >= 0 ? (sum + 1) / 2 : sum / 2;
}

uint32_t HELPER(neon_rhadd_u32)(uint32_t a, uint32_t b)
{
    return ((uint64_t)a + (uint64_t)b + 1) >> 1;
}

static int8_t hsub_s8(int8_t a, int8_t b)
{
    const int16_t diff = (int16_t)a - (int16_t)b;
    return diff >= 0 ? diff / 2 : diff / 2 + diff % 2;
}

uint32_t HELPER(neon_hsub_s8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = hsub_s8(S8_3(a), S8_3(b));
    const uint8_t out1 = hsub_s8(S8_2(a), S8_2(b));
    const uint8_t out2 = hsub_s8(S8_1(a), S8_1(b));
    const uint8_t out3 = hsub_s8(S8_0(a), S8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static uint8_t hsub_u8(uint8_t a, uint8_t b)
{
    const int16_t diff = (int16_t)a - (int16_t)b;
    return diff >= 0 ? diff / 2 : diff / 2 + diff % 2;
}

uint32_t HELPER(neon_hsub_u8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = hsub_u8(U8_3(a), U8_3(b));
    const uint8_t out1 = hsub_u8(U8_2(a), U8_2(b));
    const uint8_t out2 = hsub_u8(U8_1(a), U8_1(b));
    const uint8_t out3 = hsub_u8(U8_0(a), U8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static int16_t hsub_s16(int16_t a, int16_t b)
{
    const int32_t diff = (int32_t)a - (int32_t)b;
    return diff >= 0 ? diff / 2 : diff / 2 + diff % 2;
}

uint32_t HELPER(neon_hsub_s16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = hsub_s16(S16_1(a), S16_1(b));
    const uint16_t lo = hsub_s16(S16_0(a), S16_0(b));
    return (hi << 16) | lo;
}

static uint16_t hsub_u16(uint16_t a, uint16_t b)
{
    int32_t diff = ((int32_t)a - (int32_t)b);
    if(diff < 0) {
        diff -= 1;
    }
    const int32_t hdiff = diff / 2;
    return hdiff >= 0 ? hdiff : UINT16_MAX + 1 + hdiff;
}

uint32_t HELPER(neon_hsub_u16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = hsub_u16(U16_1(a), U16_1(b));
    const uint16_t lo = hsub_u16(U16_0(a), U16_0(b));
    return (hi << 16) | lo;
}

int32_t HELPER(neon_hsub_s32)(int32_t a, int32_t b)
{
    const int64_t diff = (int64_t)a - (int64_t)b;
    return diff >= 0 ? diff / 2 : diff / 2 + diff % 2;
}

uint32_t HELPER(neon_hsub_u32)(uint32_t a, uint32_t b)
{
    int64_t diff = ((int64_t)a - (int64_t)b);
    if(diff < 0) {
        diff -= 1;
    }
    const int64_t hdiff = diff / 2;
    return hdiff >= 0 ? hdiff : UINT32_MAX + 1 + hdiff;
}

static int8_t min_s8(int8_t a, int8_t b)
{
    return a < b ? a : b;
}

uint32_t HELPER(neon_min_s8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = min_s8(S8_3(a), S8_3(b));
    const uint8_t out1 = min_s8(S8_2(a), S8_2(b));
    const uint8_t out2 = min_s8(S8_1(a), S8_1(b));
    const uint8_t out3 = min_s8(S8_0(a), S8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static uint8_t min_u8(uint8_t a, uint8_t b)
{
    return a < b ? a : b;
}

uint32_t HELPER(neon_min_u8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = min_u8(U8_3(a), U8_3(b));
    const uint8_t out1 = min_u8(U8_2(a), U8_2(b));
    const uint8_t out2 = min_u8(U8_1(a), U8_1(b));
    const uint8_t out3 = min_u8(U8_0(a), U8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static int16_t min_s16(int16_t a, int16_t b)
{
    return a < b ? a : b;
}

uint32_t HELPER(neon_min_s16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = min_s16(S16_1(a), S16_1(b));
    const uint16_t lo = min_s16(S16_0(a), S16_0(b));
    return (hi << 16) | lo;
}

static uint16_t min_u16(uint16_t a, uint16_t b)
{
    return a < b ? a : b;
}

uint32_t HELPER(neon_min_u16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = min_u16(U16_1(a), U16_1(b));
    const uint16_t lo = min_u16(U16_0(a), U16_0(b));
    return (hi << 16) | lo;
}

uint32_t HELPER(neon_min_s32)(int32_t a, int32_t b)
{
    return a < b ? a : b;
}

uint32_t HELPER(neon_min_u32)(uint32_t a, uint32_t b)
{
    return a < b ? a : b;
}

static int8_t max_s8(int8_t a, int8_t b)
{
    return a > b ? a : b;
}

uint32_t HELPER(neon_max_s8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = max_s8(S8_3(a), S8_3(b));
    const uint8_t out1 = max_s8(S8_2(a), S8_2(b));
    const uint8_t out2 = max_s8(S8_1(a), S8_1(b));
    const uint8_t out3 = max_s8(S8_0(a), S8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static uint8_t max_u8(uint8_t a, uint8_t b)
{
    return a > b ? a : b;
}

uint32_t HELPER(neon_max_u8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = max_u8(U8_3(a), U8_3(b));
    const uint8_t out1 = max_u8(U8_2(a), U8_2(b));
    const uint8_t out2 = max_u8(U8_1(a), U8_1(b));
    const uint8_t out3 = max_u8(U8_0(a), U8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static int16_t max_s16(int16_t a, int16_t b)
{
    return a > b ? a : b;
}

uint32_t HELPER(neon_max_s16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = max_s16(S16_1(a), S16_1(b));
    const uint16_t lo = max_s16(S16_0(a), S16_0(b));
    return (hi << 16) | lo;
}

static uint16_t max_u16(uint16_t a, uint16_t b)
{
    return a > b ? a : b;
}

uint32_t HELPER(neon_max_u16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = max_u16(U16_1(a), U16_1(b));
    const uint16_t lo = max_u16(U16_0(a), U16_0(b));
    return (hi << 16) | lo;
}

uint32_t HELPER(neon_max_s32)(int32_t a, int32_t b)
{
    return a > b ? a : b;
}

uint32_t HELPER(neon_max_u32)(uint32_t a, uint32_t b)
{
    return a > b ? a : b;
}

uint32_t HELPER(neon_pmin_s8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = min_s8(S8_3(b), S8_2(b));
    const uint8_t out1 = min_s8(S8_1(b), S8_0(b));
    const uint8_t out2 = min_s8(S8_3(a), S8_2(a));
    const uint8_t out3 = min_s8(S8_1(a), S8_0(a));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

uint32_t HELPER(neon_pmin_u8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = min_u8(U8_3(b), U8_2(b));
    const uint8_t out1 = min_u8(U8_1(b), U8_0(b));
    const uint8_t out2 = min_u8(U8_3(a), U8_2(a));
    const uint8_t out3 = min_u8(U8_1(a), U8_0(a));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

uint32_t HELPER(neon_pmin_s16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = min_s16(S16_1(b), S16_0(b));
    const uint16_t lo = min_s16(S16_1(a), S16_0(a));
    return (hi << 16) | lo;
}

uint32_t HELPER(neon_pmin_u16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = min_u16(U16_1(b), U16_0(b));
    const uint16_t lo = min_u16(U16_1(a), U16_0(a));
    return (hi << 16) | lo;
}

uint32_t HELPER(neon_pmax_s8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = max_s8(S8_3(b), S8_2(b));
    const uint8_t out1 = max_s8(S8_1(b), S8_0(b));
    const uint8_t out2 = max_s8(S8_3(a), S8_2(a));
    const uint8_t out3 = max_s8(S8_1(a), S8_0(a));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

uint32_t HELPER(neon_pmax_u8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = max_u8(U8_3(b), U8_2(b));
    const uint8_t out1 = max_u8(U8_1(b), U8_0(b));
    const uint8_t out2 = max_u8(U8_3(a), U8_2(a));
    const uint8_t out3 = max_u8(U8_1(a), U8_0(a));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

uint32_t HELPER(neon_pmax_s16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = max_s16(S16_1(b), S16_0(b));
    const uint16_t lo = max_s16(S16_1(a), S16_0(a));
    return (hi << 16) | lo;
}

uint32_t HELPER(neon_pmax_u16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = max_u16(U16_1(b), U16_0(b));
    const uint16_t lo = max_u16(U16_1(a), U16_0(a));
    return (hi << 16) | lo;
}

uint32_t HELPER(neon_padd_u8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = U8_3(b) + U8_2(b);
    const uint8_t out1 = U8_1(b) + U8_0(b);
    const uint8_t out2 = U8_3(a) + U8_2(a);
    const uint8_t out3 = U8_1(a) + U8_0(a);

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

uint32_t HELPER(neon_padd_u16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = U16_1(b) + U16_0(b);
    const uint16_t lo = U16_1(a) + U16_0(a);
    return (hi << 16) | lo;
}

static int32_t shl_s8(int8_t a, int8_t b)
{
    const int32_t a32 = a;
    const uint8_t bu = b;

    if(b >= ((int)sizeof(a)) * 8) {
        return 0;
    } else if(b <= -((int)sizeof(a)) * 8) {
        return a >> (sizeof(a) * 8 - 1);
    }

    if(b >= 0) {
        return a32 << bu;
    } else if(a >= 0) {
        return a32 >> -b;
    } else {  //  sign extend when right-shifting negative
        return (a32 >> -b) | (0xffffffffu << ((8 * sizeof a) + b));
    }
}

uint32_t HELPER(neon_shl_s8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = shl_s8(S8_3(a), S8_3(b));
    const uint8_t out1 = shl_s8(S8_2(a), S8_2(b));
    const uint8_t out2 = shl_s8(S8_1(a), S8_1(b));
    const uint8_t out3 = shl_s8(S8_0(a), S8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static uint32_t shl_u8(uint8_t a, int8_t b)
{
    uint32_t au = a;
    return b >= 0 ? au << b : au >> -b;
}

uint32_t HELPER(neon_shl_u8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = shl_u8(U8_3(a), S8_3(b));
    const uint8_t out1 = shl_u8(U8_2(a), S8_2(b));
    const uint8_t out2 = shl_u8(U8_1(a), S8_1(b));
    const uint8_t out3 = shl_u8(U8_0(a), S8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static int32_t shl_s16(int16_t a, int8_t b)
{
    const int32_t a32 = a;
    const uint16_t bu = b;

    if(b >= ((int)sizeof(a)) * 8) {
        return 0;
    } else if(b <= -((int)sizeof(a)) * 8) {
        return a >> (((int)sizeof(a)) * 8 - 1);
    }

    if(b >= 0) {
        return a32 << bu;
    } else if(a >= 0) {
        return a32 >> -b;
    } else {  //  sign extend when right-shifting negative
        return (a32 >> -b) | (0xffffffffu << ((8 * ((int)sizeof a)) + b));
    }
}

uint32_t HELPER(neon_shl_s16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = shl_s16(S16_1(a), S16_1(b));
    const uint16_t lo = shl_s16(S16_0(a), S16_0(b));
    return (hi << 16) | lo;
}

static uint32_t shl_u16(uint16_t a, int8_t b)
{
    uint32_t au = a;
    return b >= 0 ? au << b : au >> -b;
}

uint32_t HELPER(neon_shl_u16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = shl_u16(U16_1(a), S16_1(b));
    const uint16_t lo = shl_u16(U16_0(a), S16_0(b));
    return (hi << 16) | lo;
}

static int32_t shl_s32(int32_t a, int8_t b)
{
    const uint32_t au = a;

    if(b >= ((int)sizeof(a)) * 8) {
        return 0;
    } else if(b <= -((int)sizeof(a)) * 8) {
        return a >> (((int)sizeof(a)) * 8 - 1);
    }

    if(b >= 0) {
        return au << b;
    } else if(a >= 0) {
        return au >> -b;
    } else {  //  sign extend when right-shifting negative
        return (au >> -b) | (0xffffffffu << ((8 * ((int)sizeof a)) + b));
    }
}

uint32_t HELPER(neon_shl_s32)(uint32_t a, uint32_t b)
{
    return shl_s32(a, b);
}

static uint32_t shl_u32(uint32_t a, int8_t b)
{
    //  Shifting by the word size or more is undefined in C.
    if(abs(b) >= 8 * sizeof a) {
        return 0;
    }
    return b >= 0 ? a << b : a >> -b;
}

uint32_t HELPER(neon_shl_u32)(uint32_t a, uint32_t b)
{
    return shl_u32(a, b);
}

static int64_t shl_s64(int64_t a, int8_t b)
{
    const uint64_t au = a;

    //  Shifting by the word size or more is undefined in C.
    if(abs(b) >= 8 * ((int)sizeof a)) {
        return b < 0 && a < 0 ? 0xffffffffffffffffu : 0;
    }

    if(b >= 0) {
        return au << b;
    } else if(a >= 0) {
        return au >> -b;
    } else {  //  sign extend when right-shifting negative
        return (au >> -b) | (0xffffffffffffffffu << ((8 * ((int)sizeof a)) + b));
    }
}

uint64_t HELPER(neon_shl_s64)(uint64_t a, uint64_t b)
{
    return shl_s64(a, b);
}

static uint64_t shl_u64(uint64_t a, int8_t b)
{
    //  Shifting by the word size or more is undefined in C.
    if(abs(b) >= 8 * ((int)sizeof a)) {
        return 0;
    }
    return b >= 0 ? a << b : a >> -b;
}

uint64_t HELPER(neon_shl_u64)(uint64_t a, uint64_t b)
{
    return shl_u64(a, b);
}

static int8_t rshl_s8(int8_t a, int8_t b)
{
    const uint8_t au = a;
    uint8_t ret = shl_s8(a, b);

    if(b < 0 && -b <= 8 * ((int)sizeof a)) {
        ret += au >> (-b - 1) & 1;
    }

    return ret;
}

uint32_t HELPER(neon_rshl_s8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = rshl_s8(S8_3(a), S8_3(b));
    const uint8_t out1 = rshl_s8(S8_2(a), S8_2(b));
    const uint8_t out2 = rshl_s8(S8_1(a), S8_1(b));
    const uint8_t out3 = rshl_s8(S8_0(a), S8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static uint8_t rshl_u8(uint8_t a, int8_t b)
{
    uint8_t ret = shl_u8(a, b);

    if(b < 0 && -b <= 8 * ((int)sizeof a)) {
        ret += a >> (-b - 1) & 1;
    }

    return ret;
}

uint32_t HELPER(neon_rshl_u8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = rshl_u8(U8_3(a), S8_3(b));
    const uint8_t out1 = rshl_u8(U8_2(a), S8_2(b));
    const uint8_t out2 = rshl_u8(U8_1(a), S8_1(b));
    const uint8_t out3 = rshl_u8(U8_0(a), S8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static int16_t rshl_s16(int16_t a, int8_t b)
{
    const uint16_t au = a;
    uint16_t ret = shl_s16(a, b);

    if(b < 0 && -b <= 8 * sizeof a) {
        ret += au >> (-b - 1) & 1;
    }

    return ret;
}

uint32_t HELPER(neon_rshl_s16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = rshl_s16(S16_1(a), S16_1(b));
    const uint16_t lo = rshl_s16(S16_0(a), S16_0(b));
    return (hi << 16) | lo;
}

static uint16_t rshl_u16(uint16_t a, int8_t b)
{
    uint16_t ret = shl_u16(a, b);

    if(b < 0 && -b <= 8 * sizeof a) {
        ret += a >> (-b - 1) & 1;
    }

    return ret;
}

uint32_t HELPER(neon_rshl_u16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = rshl_u16(U16_1(a), S16_1(b));
    const uint16_t lo = rshl_u16(U16_0(a), S16_0(b));
    return (hi << 16) | lo;
}

uint32_t HELPER(neon_rshl_s32)(uint32_t a, uint32_t b)
{
    const int8_t bs = b;
    uint32_t ret = shl_s32(a, b);

    if(bs < 0 && -bs <= 8 * sizeof a) {
        ret += a >> (-bs - 1) & 1;
    }

    return ret;
}

uint32_t HELPER(neon_rshl_u32)(uint32_t a, uint32_t b)
{
    const int8_t bs = b;
    uint32_t ret = shl_u32(a, b);

    if(bs < 0 && -bs <= 8 * sizeof a) {
        ret += a >> (-bs - 1) & 1;
    }

    return ret;
}

uint64_t HELPER(neon_rshl_s64)(uint64_t a, uint64_t b)
{
    const int8_t bs = b;
    uint64_t ret = shl_s64(a, b);

    if(bs < 0 && -bs <= 8 * sizeof a) {
        ret += a >> (-bs - 1) & 1;
    }

    return ret;
}

uint64_t HELPER(neon_rshl_u64)(uint64_t a, uint64_t b)
{
    const int8_t bs = b;
    uint64_t ret = shl_u64(a, b);

    if(bs < 0 && -bs <= 8 * sizeof a) {
        ret += a >> (-bs - 1) & 1;
    }

    return ret;
}

static int8_t qshl_s8(CPUState *env, int8_t a, int8_t b)
{
    int32_t result;

    if(b >= ((int)sizeof(a)) * 8) {
        if(a > 0) {
            env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
            return INT8_MAX;
        } else if(a < 0) {
            env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
            return INT8_MIN;
        }
    }

    result = shl_s8(a, b);

    if(result < INT8_MIN) {
        result = INT8_MIN;
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
    } else if(result > INT8_MAX) {
        result = INT8_MAX;
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
    }
    return (int8_t)result;
}

uint32_t HELPER(neon_qshl_s8)(CPUState *env, uint32_t a, uint32_t b)
{
    const uint8_t out0 = qshl_s8(env, S8_3(a), S8_3(b));
    const uint8_t out1 = qshl_s8(env, S8_2(a), S8_2(b));
    const uint8_t out2 = qshl_s8(env, S8_1(a), S8_1(b));
    const uint8_t out3 = qshl_s8(env, S8_0(a), S8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static uint8_t qshl_u8(CPUState *env, uint8_t a, int8_t b)
{
    uint32_t result = shl_u8(a, b);
    if(result > UINT8_MAX) {
        //  Saturated?
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return UINT8_MAX;
    }

    return result;
}

uint32_t HELPER(neon_qshl_u8)(CPUState *env, uint32_t a, uint32_t b)
{
    const uint8_t out0 = qshl_u8(env, U8_3(a), S8_3(b));
    const uint8_t out1 = qshl_u8(env, U8_2(a), S8_2(b));
    const uint8_t out2 = qshl_u8(env, U8_1(a), S8_1(b));
    const uint8_t out3 = qshl_u8(env, U8_0(a), S8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static int16_t qshl_s16(CPUState *env, int16_t a, int8_t b)
{
    int32_t result;

    if(b >= ((int)sizeof(a)) * 8) {
        if(a > 0) {
            env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
            return INT16_MAX;
        } else if(a < 0) {
            env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
            return INT16_MIN;
        }
    }

    result = shl_s16(a, b);

    if(result < INT16_MIN) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        result = INT16_MIN;
    } else if(result > INT16_MAX) {
        result = INT16_MAX;
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
    }

    return (int16_t)result;
}

uint32_t HELPER(neon_qshl_s16)(CPUState *env, uint32_t a, uint32_t b)
{
    const uint16_t hi = qshl_s16(env, S16_1(a), S16_1(b));
    const uint16_t lo = qshl_s16(env, S16_0(a), S16_0(b));
    return (hi << 16) | lo;
}

static uint16_t qshl_u16(CPUState *env, uint16_t a, int8_t b)
{
    uint32_t result = shl_u16(a, b);
    if(result > UINT16_MAX) {
        //  Saturated?
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return UINT16_MAX;
    }

    return result;
}

uint32_t HELPER(neon_qshl_u16)(CPUState *env, uint32_t a, uint32_t b)
{
    const uint16_t hi = qshl_u16(env, U16_1(a), S16_1(b));
    const uint16_t lo = qshl_u16(env, U16_0(a), S16_0(b));
    return (hi << 16) | lo;
}

static int32_t qshl_s32(CPUState *env, int32_t a, int8_t b)
{
    if(b > 0) {
        const uint32_t mask = (INT32_MAX << (8 * sizeof a - b - 1)) & INT32_MAX;
        if(a >= 0) {
            if((b >= 8 * sizeof a) || (a & mask)) {
                env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
                return INT32_MAX;
            }
        } else {  //  a < 0
            if((b >= 8 * sizeof a) || (~a & mask)) {
                env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
                return INT32_MIN;
            }
        }
    }

    return shl_s32(a, b);
}

uint32_t HELPER(neon_qshl_s32)(CPUState *env, uint32_t a, uint32_t b)
{
    return qshl_s32(env, a, b);
}

static uint32_t qshl_u32(CPUState *env, uint32_t a, int8_t b)
{
    if(b > 0) {
        //  Saturated?
        const uint32_t mask = UINT32_MAX << (8 * sizeof a - b);
        if((b >= 8 * sizeof a) || (a & mask)) {
            env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
            return UINT32_MAX;
        }
    }

    return shl_u32(a, b);
}

uint32_t HELPER(neon_qshl_u32)(CPUState *env, uint32_t a, uint32_t b)
{
    return qshl_u32(env, a, b);
}

static int64_t qshl_s64(CPUState *env, int64_t a, int8_t b)
{
    if(b > 0) {
        const uint64_t mask = (INT64_MAX << (8 * sizeof a - b - 1)) & INT64_MAX;
        if(a >= 0) {
            if((b >= 8 * sizeof a) || (a & mask)) {
                env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
                return INT64_MAX;
            }
        } else {  //  a < 0
            if((b >= 8 * sizeof a) || (~a & mask)) {
                env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
                return INT64_MIN;
            }
        }
    }

    return shl_s64(a, b);
}

uint64_t HELPER(neon_qshl_s64)(CPUState *env, uint64_t a, uint64_t b)
{
    return qshl_s64(env, a, b);
}

static uint64_t qshl_u64(CPUState *env, uint64_t a, int8_t b)
{
    if(b > 0) {
        //  Saturated?
        const uint64_t mask = UINT64_MAX << (8 * sizeof a - b);
        if((b >= 8 * sizeof a) || (a & mask)) {
            env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
            return UINT64_MAX;
        }
    }

    return shl_u64(a, b);
}

uint64_t HELPER(neon_qshl_u64)(CPUState *env, uint64_t a, uint64_t b)
{
    return qshl_u64(env, a, b);
}

static uint8_t qshlu_s8(CPUState *env, int8_t a, uint8_t b)
{
    if(a < 0) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return 0;
    }

    return qshl_u8(env, a, b);
}

uint32_t HELPER(neon_qshlu_s8)(CPUState *env, uint32_t a, uint32_t b)
{
    const uint8_t out0 = qshlu_s8(env, S8_3(a), U8_3(b));
    const uint8_t out1 = qshlu_s8(env, S8_2(a), U8_2(b));
    const uint8_t out2 = qshlu_s8(env, S8_1(a), U8_1(b));
    const uint8_t out3 = qshlu_s8(env, S8_0(a), U8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static int16_t qshlu_s16(CPUState *env, int16_t a, int8_t b)
{
    if(a < 0) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return 0;
    }

    return qshl_u16(env, a, b);
}

uint32_t HELPER(neon_qshlu_s16)(CPUState *env, uint32_t a, uint32_t b)
{
    const uint16_t hi = qshlu_s16(env, S16_1(a), S16_1(b));
    const uint16_t lo = qshlu_s16(env, S16_0(a), S16_0(b));
    return (hi << 16) | lo;
}

static int32_t qshlu_s32(CPUState *env, int32_t a, int8_t b)
{
    if(a < 0) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return 0;
    }

    return qshl_u32(env, a, b);
}

uint32_t HELPER(neon_qshlu_s32)(CPUState *env, uint32_t a, uint32_t b)
{
    return qshlu_s32(env, a, b);
}

static int64_t qshlu_s64(CPUState *env, int64_t a, int8_t b)
{
    if(a < 0) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return 0;
    }

    return qshl_u64(env, a, b);
}

uint64_t HELPER(neon_qshlu_s64)(CPUState *env, uint64_t a, uint64_t b)
{
    return qshlu_s64(env, a, b);
}

static int8_t qrshl_s8(CPUState *env, int8_t a, int8_t b)
{
    const uint16_t au = a;
    uint8_t ret = qshl_s8(env, a, b);

    if(b < 0 && -b <= 8 * sizeof a) {
        ret += au >> (-b - 1) & 1;
    }

    return ret;
}

uint32_t HELPER(neon_qrshl_s8)(CPUState *env, uint32_t a, uint32_t b)
{
    const uint8_t out0 = qrshl_s8(env, S8_3(a), S8_3(b));
    const uint8_t out1 = qrshl_s8(env, S8_2(a), S8_2(b));
    const uint8_t out2 = qrshl_s8(env, S8_1(a), S8_1(b));
    const uint8_t out3 = qrshl_s8(env, S8_0(a), S8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static uint8_t qrshl_u8(CPUState *env, uint8_t a, int8_t b)
{
    uint8_t ret = qshl_u8(env, a, b);

    if(b < 0 && -b <= 8 * sizeof a) {
        ret += a >> (-b - 1) & 1;
    }

    return ret;
}

uint32_t HELPER(neon_qrshl_u8)(CPUState *env, uint32_t a, uint32_t b)
{
    const uint8_t out0 = qrshl_u8(env, U8_3(a), S8_3(b));
    const uint8_t out1 = qrshl_u8(env, U8_2(a), S8_2(b));
    const uint8_t out2 = qrshl_u8(env, U8_1(a), S8_1(b));
    const uint8_t out3 = qrshl_u8(env, U8_0(a), S8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static int16_t qrshl_s16(CPUState *env, int16_t a, int8_t b)
{
    const uint16_t au = a;
    uint16_t ret = qshl_s16(env, a, b);

    if(b < 0 && -b <= 8 * sizeof a) {
        ret += au >> (-b - 1) & 1;
    }

    return ret;
}

uint32_t HELPER(neon_qrshl_s16)(CPUState *env, uint32_t a, uint32_t b)
{
    const uint16_t hi = qrshl_s16(env, S16_1(a), S16_1(b));
    const uint16_t lo = qrshl_s16(env, S16_0(a), S16_0(b));
    return (hi << 16) | lo;
}

static uint16_t qrshl_u16(CPUState *env, uint16_t a, int8_t b)
{
    uint16_t ret = qshl_u16(env, a, b);

    if(b < 0 && -b <= 8 * sizeof a) {
        ret += a >> (-b - 1) & 1;
    }

    return ret;
}

uint32_t HELPER(neon_qrshl_u16)(CPUState *env, uint32_t a, uint32_t b)
{
    const uint16_t hi = qrshl_u16(env, U16_1(a), S16_1(b));
    const uint16_t lo = qrshl_u16(env, U16_0(a), S16_0(b));
    return (hi << 16) | lo;
}

uint32_t HELPER(neon_qrshl_s32)(CPUState *env, uint32_t a, uint32_t b)
{
    const int8_t bs = b;
    uint32_t ret = qshl_s32(env, a, b);

    if(bs < 0 && -bs <= 8 * sizeof a) {
        ret += a >> (-bs - 1) & 1;
    }

    return ret;
}

uint32_t HELPER(neon_qrshl_u32)(CPUState *env, uint32_t a, uint32_t b)
{
    const int8_t bs = b;
    uint32_t ret = qshl_u32(env, a, b);

    if(bs < 0 && -bs <= 8 * sizeof a) {
        ret += a >> (-bs - 1) & 1;
    }

    return ret;
}

uint64_t HELPER(neon_qrshl_s64)(CPUState *env, uint64_t a, uint64_t b)
{
    const int8_t bs = b;
    uint64_t ret = qshl_s64(env, a, b);

    if(bs < 0 && -bs <= 8 * sizeof a) {
        ret += a >> (-bs - 1) & 1;
    }

    return ret;
}

uint64_t HELPER(neon_qrshl_u64)(CPUState *env, uint64_t a, uint64_t b)
{
    const int8_t bs = b;
    uint64_t ret = qshl_u64(env, a, b);

    if(bs < 0 && -bs <= 8 * sizeof a) {
        ret += a >> (-bs - 1) & 1;
    }

    return ret;
}

static uint8_t clz_u8(uint8_t a)
{
    uint8_t count = 0;
    while((a & 0x80) == 0 && count < 8) {
        a <<= 1;
        count++;
    }
    return count;
}

uint32_t HELPER(neon_clz_u8)(uint32_t a)
{
    const uint8_t out0 = clz_u8(U8_3(a));
    const uint8_t out1 = clz_u8(U8_2(a));
    const uint8_t out2 = clz_u8(U8_1(a));
    const uint8_t out3 = clz_u8(U8_0(a));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static uint16_t clz_u16(uint16_t a)
{
    uint16_t count = 0;
    while((a & 0x8000) == 0 && count < 16) {
        a <<= 1;
        count++;
    }
    return count;
}

uint32_t HELPER(neon_clz_u16)(uint32_t a)
{
    const uint16_t hi = clz_u16(U16_1(a));
    const uint16_t lo = clz_u16(U16_0(a));
    return (hi << 16) | lo;
}

static uint8_t cls_s8(uint8_t a)
{
    uint8_t count = 0;
    const uint8_t sign = !!(a & 0x80);
    while(!!(a & 0x40) == sign && count < 7) {
        a <<= 1;
        count++;
    }
    return count;
}

uint32_t HELPER(neon_cls_s8)(uint32_t a)
{
    const uint8_t out0 = cls_s8(U8_3(a));
    const uint8_t out1 = cls_s8(U8_2(a));
    const uint8_t out2 = cls_s8(U8_1(a));
    const uint8_t out3 = cls_s8(U8_0(a));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static uint16_t cls_s16(uint16_t a)
{
    uint16_t count = 0;
    const uint16_t sign = !!(a & 0x8000);
    while(!!(a & 0x4000) == sign && count < 15) {
        a <<= 1;
        count++;
    }
    return count;
}

uint32_t HELPER(neon_cls_s16)(uint32_t a)
{
    const uint16_t hi = cls_s16(U16_1(a));
    const uint16_t lo = cls_s16(U16_0(a));
    return (hi << 16) | lo;
}

uint32_t HELPER(neon_cls_s32)(uint32_t a)
{
    uint32_t count = 0;
    const uint32_t sign = !!(a & 0x80000000);
    while(!!(a & 0x40000000) == sign && count < 31) {
        a <<= 1;
        count++;
    }
    return count;
}

static uint8_t cnt_u8(uint8_t a)
{
    int i;
    uint8_t count = 0;
    for(i = 0; i < 8; i++) {
        count += a & 1;
        a >>= 1;
    }
    return count;
}

uint32_t HELPER(neon_cnt_u8)(uint32_t a)
{
    const uint8_t out0 = cnt_u8(U8_3(a));
    const uint8_t out1 = cnt_u8(U8_2(a));
    const uint8_t out2 = cnt_u8(U8_1(a));
    const uint8_t out3 = cnt_u8(U8_0(a));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static uint8_t tst_u8(uint8_t a, uint8_t b)
{
    return a & b ? UINT8_MAX : 0;
}

uint32_t HELPER(neon_tst_u8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = tst_u8(U8_3(a), U8_3(b));
    const uint8_t out1 = tst_u8(U8_2(a), U8_2(b));
    const uint8_t out2 = tst_u8(U8_1(a), U8_1(b));
    const uint8_t out3 = tst_u8(U8_0(a), U8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static uint16_t tst_u16(uint16_t a, uint16_t b)
{
    return a & b ? UINT16_MAX : 0;
}

uint32_t HELPER(neon_tst_u16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = tst_u16(U16_1(a), U16_1(b));
    const uint16_t lo = tst_u16(U16_0(a), U16_0(b));
    return (hi << 16) | lo;
}

uint32_t HELPER(neon_tst_u32)(uint32_t a, uint32_t b)
{
    return a & b ? UINT32_MAX : 0;
}

static uint8_t ceq_u8(uint8_t a, uint8_t b)
{
    return a == b ? UINT8_MAX : 0;
}

uint32_t HELPER(neon_ceq_u8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = ceq_u8(U8_3(a), U8_3(b));
    const uint8_t out1 = ceq_u8(U8_2(a), U8_2(b));
    const uint8_t out2 = ceq_u8(U8_1(a), U8_1(b));
    const uint8_t out3 = ceq_u8(U8_0(a), U8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static uint16_t ceq_u16(uint16_t a, uint16_t b)
{
    return a == b ? UINT16_MAX : 0;
}

uint32_t HELPER(neon_ceq_u16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = ceq_u16(U16_1(a), U16_1(b));
    const uint16_t lo = ceq_u16(U16_0(a), U16_0(b));
    return (hi << 16) | lo;
}

uint32_t HELPER(neon_ceq_u32)(uint32_t a, uint32_t b)
{
    return a == b ? UINT32_MAX : 0;
}

static uint8_t cge_u8(uint8_t a, uint8_t b)
{
    return a >= b ? UINT8_MAX : 0;
}

uint32_t HELPER(neon_cge_u8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = cge_u8(U8_3(a), U8_3(b));
    const uint8_t out1 = cge_u8(U8_2(a), U8_2(b));
    const uint8_t out2 = cge_u8(U8_1(a), U8_1(b));
    const uint8_t out3 = cge_u8(U8_0(a), U8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static uint8_t cge_s8(int8_t a, int8_t b)
{
    return a >= b ? UINT8_MAX : 0;
}

uint32_t HELPER(neon_cge_s8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = cge_s8(S8_3(a), S8_3(b));
    const uint8_t out1 = cge_s8(S8_2(a), S8_2(b));
    const uint8_t out2 = cge_s8(S8_1(a), S8_1(b));
    const uint8_t out3 = cge_s8(S8_0(a), S8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static uint16_t cge_u16(uint16_t a, uint16_t b)
{
    return a >= b ? UINT16_MAX : 0;
}

uint32_t HELPER(neon_cge_u16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = cge_u16(U16_1(a), U16_1(b));
    const uint16_t lo = cge_u16(U16_0(a), U16_0(b));
    return (hi << 16) | lo;
}

static uint16_t cge_s16(int16_t a, int16_t b)
{
    return a >= b ? UINT16_MAX : 0;
}

uint32_t HELPER(neon_cge_s16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = cge_s16(S16_1(a), S16_1(b));
    const uint16_t lo = cge_s16(S16_0(a), S16_0(b));
    return (hi << 16) | lo;
}

uint32_t HELPER(neon_cge_u32)(uint32_t a, uint32_t b)
{
    return a >= b ? UINT32_MAX : 0;
}

uint32_t HELPER(neon_cge_s32)(uint32_t a, uint32_t b)
{
    return (int32_t)a >= (int32_t)b ? UINT32_MAX : 0;
}

static uint8_t cgt_u8(uint8_t a, uint8_t b)
{
    return a > b ? UINT8_MAX : 0;
}

uint32_t HELPER(neon_cgt_u8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = cgt_u8(U8_3(a), U8_3(b));
    const uint8_t out1 = cgt_u8(U8_2(a), U8_2(b));
    const uint8_t out2 = cgt_u8(U8_1(a), U8_1(b));
    const uint8_t out3 = cgt_u8(U8_0(a), U8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static uint8_t cgt_s8(int8_t a, int8_t b)
{
    return a > b ? UINT8_MAX : 0;
}

uint32_t HELPER(neon_cgt_s8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = cgt_s8(S8_3(a), S8_3(b));
    const uint8_t out1 = cgt_s8(S8_2(a), S8_2(b));
    const uint8_t out2 = cgt_s8(S8_1(a), S8_1(b));
    const uint8_t out3 = cgt_s8(S8_0(a), S8_0(b));

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

static uint16_t cgt_u16(uint16_t a, uint16_t b)
{
    return a > b ? UINT16_MAX : 0;
}

uint32_t HELPER(neon_cgt_u16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = cgt_u16(U16_1(a), U16_1(b));
    const uint16_t lo = cgt_u16(U16_0(a), U16_0(b));
    return (hi << 16) | lo;
}

static uint16_t cgt_s16(int16_t a, int16_t b)
{
    return a > b ? UINT16_MAX : 0;
}

uint32_t HELPER(neon_cgt_s16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = cgt_s16(S16_1(a), S16_1(b));
    const uint16_t lo = cgt_s16(S16_0(a), S16_0(b));
    return (hi << 16) | lo;
}

uint32_t HELPER(neon_cgt_u32)(uint32_t a, uint32_t b)
{
    return a > b ? UINT32_MAX : 0;
}

uint32_t HELPER(neon_cgt_s32)(uint32_t a, uint32_t b)
{
    return (int32_t)a > (int32_t)b ? UINT32_MAX : 0;
}

uint32_t HELPER(neon_mul_u8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = U8_3(a) * U8_3(b);
    const uint8_t out1 = U8_2(a) * U8_2(b);
    const uint8_t out2 = U8_1(a) * U8_1(b);
    const uint8_t out3 = U8_0(a) * U8_0(b);

    return out0 << 24 | out1 << 16 | out2 << 8 | out3;
}

uint32_t HELPER(neon_mul_u16)(uint32_t a, uint32_t b)
{
    const uint16_t hi = U16_1(a) * U16_1(b);
    const uint16_t lo = U16_0(a) * U16_0(b);
    return (hi << 16) | lo;
}

static int16_t qdmulh_s16(CPUState *env, int16_t a, int16_t b)
{
    if(a == INT16_MIN && b == INT16_MIN) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return INT16_MAX;
    }

    return (a * b * 2) >> 16;
}

uint32_t HELPER(neon_qdmulh_s16)(CPUState *env, uint32_t a, uint32_t b)
{
    const uint16_t hi = qdmulh_s16(env, S16_1(a), S16_1(b));
    const uint16_t lo = qdmulh_s16(env, S16_0(a), S16_0(b));
    return (hi << 16) | lo;
}

static int32_t qdmulh_s32(CPUState *env, int32_t a, int32_t b)
{
    const int64_t a64 = a;
    const int64_t b64 = b;

    if(a == INT32_MIN && b == INT32_MIN) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return INT32_MAX;
    }

    return (a64 * b64 * 2) >> 32;
}

uint32_t HELPER(neon_qdmulh_s32)(CPUState *env, uint32_t a, uint32_t b)
{
    return qdmulh_s32(env, a, b);
}

static int16_t qrdmulh_s16(CPUState *env, int16_t a, int16_t b)
{
    if(a == INT16_MIN && b == INT16_MIN) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return INT16_MAX;
    }

    const int32_t prod = a * b * 2;

    return prod & 0x8000 ? (prod >> 16) + 1 : prod >> 16;
}

uint32_t HELPER(neon_qrdmulh_s16)(CPUState *env, uint32_t a, uint32_t b)
{
    const uint16_t hi = qrdmulh_s16(env, S16_1(a), S16_1(b));
    const uint16_t lo = qrdmulh_s16(env, S16_0(a), S16_0(b));
    return (hi << 16) | lo;
}

static int32_t qrdmulh_s32(CPUState *env, int32_t a, int32_t b)
{
    if(a == INT32_MIN && b == INT32_MIN) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return INT32_MAX;
    }

    const int64_t prod = (int64_t)a * (int64_t)b * 2;

    return prod & 0x80000000 ? (prod >> 32) + 1 : prod >> 32;
}

uint32_t HELPER(neon_qrdmulh_s32)(CPUState *env, uint32_t a, uint32_t b)
{
    return qrdmulh_s32(env, a, b);
}

uint64_t HELPER(neon_addl_u16)(uint64_t a, uint64_t b)
{
    const uint16_t out0 = U16_3(a) + U16_3(b);
    const uint16_t out1 = U16_2(a) + U16_2(b);
    const uint16_t out2 = U16_1(a) + U16_1(b);
    const uint16_t out3 = U16_0(a) + U16_0(b);

    return (uint64_t)out0 << 48 | (uint64_t)out1 << 32 | (uint64_t)out2 << 16 | (uint64_t)out3;
}

uint64_t HELPER(neon_addl_u32)(uint64_t a, uint64_t b)
{
    const uint64_t hi = U32_1(a) + U32_1(b);
    const uint64_t lo = U32_0(a) + U32_0(b);
    return (hi << 32) | lo;
}

uint64_t HELPER(neon_subl_u16)(uint64_t a, uint64_t b)
{
    const uint16_t out0 = U16_3(a) - U16_3(b);
    const uint16_t out1 = U16_2(a) - U16_2(b);
    const uint16_t out2 = U16_1(a) - U16_1(b);
    const uint16_t out3 = U16_0(a) - U16_0(b);

    return (uint64_t)out0 << 48 | (uint64_t)out1 << 32 | (uint64_t)out2 << 16 | (uint64_t)out3;
}

uint64_t HELPER(neon_subl_u32)(uint64_t a, uint64_t b)
{
    const uint64_t hi = U32_1(a) - U32_1(b);
    const uint64_t lo = U32_0(a) - U32_0(b);

    return (hi << 32) | lo;
}

uint64_t HELPER(neon_paddl_u16)(uint64_t a, uint64_t b)
{
    const uint16_t out0 = U16_2(b) + U16_3(b);
    const uint16_t out1 = U16_0(b) + U16_1(b);
    const uint16_t out2 = U16_2(a) + U16_3(a);
    const uint16_t out3 = U16_0(a) + U16_1(a);

    return (uint64_t)out0 << 48 | (uint64_t)out1 << 32 | (uint64_t)out2 << 16 | (uint64_t)out3;
}

uint64_t HELPER(neon_paddl_u32)(uint64_t a, uint64_t b)
{
    const uint64_t hi = U32_1(b) + U32_0(b);
    const uint64_t lo = U32_1(a) + U32_0(a);

    return (hi << 32) | lo;
}

uint64_t HELPER(neon_mull_u8)(uint32_t a, uint32_t b)
{
    const uint16_t out0 = (uint16_t)U8_3(a) * (uint16_t)U8_3(b);
    const uint16_t out1 = (uint16_t)U8_2(a) * (uint16_t)U8_2(b);
    const uint16_t out2 = (uint16_t)U8_1(a) * (uint16_t)U8_1(b);
    const uint16_t out3 = (uint16_t)U8_0(a) * (uint16_t)U8_0(b);

    return (uint64_t)out0 << 48 | (uint64_t)out1 << 32 | (uint64_t)out2 << 16 | (uint64_t)out3;
}

uint64_t HELPER(neon_mull_s8)(uint32_t a, uint32_t b)
{
    const uint16_t out0 = (int16_t)S8_3(a) * (int16_t)S8_3(b);
    const uint16_t out1 = (int16_t)S8_2(a) * (int16_t)S8_2(b);
    const uint16_t out2 = (int16_t)S8_1(a) * (int16_t)S8_1(b);
    const uint16_t out3 = (int16_t)S8_0(a) * (int16_t)S8_0(b);

    return (uint64_t)out0 << 48 | (uint64_t)out1 << 32 | (uint64_t)out2 << 16 | (uint64_t)out3;
}

uint64_t HELPER(neon_mull_u16)(uint32_t a, uint32_t b)
{
    const uint32_t out0 = (uint32_t)U16_1(a) * (uint32_t)U16_1(b);
    const uint32_t out1 = (uint32_t)U16_0(a) * (uint32_t)U16_0(b);

    return (uint64_t)out0 << 32 | (uint64_t)out1;
}

uint64_t HELPER(neon_mull_s16)(uint32_t a, uint32_t b)
{
    const uint32_t out0 = (int32_t)S16_1(a) * (int32_t)S16_1(b);
    const uint32_t out1 = (int32_t)S16_0(a) * (int32_t)S16_0(b);

    return (uint64_t)out0 << 32 | (uint64_t)out1;
}

uint64_t HELPER(neon_abdl_u16)(uint32_t a, uint32_t b)
{
    const uint16_t out0 = abs((int16_t)U8_3(a) - (int16_t)U8_3(b));
    const uint16_t out1 = abs((int16_t)U8_2(a) - (int16_t)U8_2(b));
    const uint16_t out2 = abs((int16_t)U8_1(a) - (int16_t)U8_1(b));
    const uint16_t out3 = abs((int16_t)U8_0(a) - (int16_t)U8_0(b));

    return (uint64_t)out0 << 48 | (uint64_t)out1 << 32 | (uint64_t)out2 << 16 | (uint64_t)out3;
}

uint64_t HELPER(neon_abdl_s16)(uint32_t a, uint32_t b)
{
    const uint16_t out0 = abs((int16_t)S8_3(a) - (int16_t)S8_3(b));
    const uint16_t out1 = abs((int16_t)S8_2(a) - (int16_t)S8_2(b));
    const uint16_t out2 = abs((int16_t)S8_1(a) - (int16_t)S8_1(b));
    const uint16_t out3 = abs((int16_t)S8_0(a) - (int16_t)S8_0(b));

    return (uint64_t)out0 << 48 | (uint64_t)out1 << 32 | (uint64_t)out2 << 16 | (uint64_t)out3;
}

uint64_t HELPER(neon_abdl_u32)(uint32_t a, uint32_t b)
{
    const uint32_t out0 = abs((int32_t)U16_1(a) - (int32_t)U16_1(b));
    const uint32_t out1 = abs((int32_t)U16_0(a) - (int32_t)U16_0(b));

    return (uint64_t)out0 << 32 | (uint64_t)out1;
}

uint64_t HELPER(neon_abdl_s32)(uint32_t a, uint32_t b)
{
    const uint32_t out0 = abs((int32_t)S16_1(a) - (int32_t)S16_1(b));
    const uint32_t out1 = abs((int32_t)S16_0(a) - (int32_t)S16_0(b));

    return (uint64_t)out0 << 32 | (uint64_t)out1;
}

uint64_t HELPER(neon_abdl_u64)(uint32_t a, uint32_t b)
{
    return (uint64_t)(a > b ? a - b : b - a);
}

uint64_t HELPER(neon_abdl_s64)(uint32_t a, uint32_t b)
{
    int32_t ta = a;
    int32_t tb = b;
    return (uint64_t)(ta > tb ? ta - tb : tb - ta);
}

static uint16_t mul_p8(uint8_t a, uint8_t b)
{
    int i;
    uint16_t ret = 0;
    for(i = 0; i < 8; i++) {
        if(b & (1 << i)) {
            ret ^= a << i;
        }
    }
    return ret;
}

uint32_t HELPER(neon_mul_p8)(uint32_t a, uint32_t b)
{
    const uint8_t out0 = mul_p8(U8_3(a), U8_3(b));
    const uint8_t out1 = mul_p8(U8_2(a), U8_2(b));
    const uint8_t out2 = mul_p8(U8_1(a), U8_1(b));
    const uint8_t out3 = mul_p8(U8_0(a), U8_0(b));

    return (uint32_t)out0 << 24 | (uint32_t)out1 << 16 | (uint32_t)out2 << 8 | (uint32_t)out3;
}

uint64_t HELPER(neon_mull_p8)(uint32_t a, uint32_t b)
{
    const uint16_t out0 = mul_p8(U8_3(a), U8_3(b));
    const uint16_t out1 = mul_p8(U8_2(a), U8_2(b));
    const uint16_t out2 = mul_p8(U8_1(a), U8_1(b));
    const uint16_t out3 = mul_p8(U8_0(a), U8_0(b));

    return (uint64_t)out0 << 48 | (uint64_t)out1 << 32 | (uint64_t)out2 << 16 | (uint64_t)out3;
}

uint64_t HELPER(neon_widen_u8)(uint32_t a)
{
    const uint16_t out0 = U8_3(a);
    const uint16_t out1 = U8_2(a);
    const uint16_t out2 = U8_1(a);
    const uint16_t out3 = U8_0(a);

    return (uint64_t)out0 << 48 | (uint64_t)out1 << 32 | (uint64_t)out2 << 16 | (uint64_t)out3;
}

uint64_t HELPER(neon_widen_s8)(uint32_t a)
{
    const uint16_t out0 = S8_3(a);
    const uint16_t out1 = S8_2(a);
    const uint16_t out2 = S8_1(a);
    const uint16_t out3 = S8_0(a);

    return (uint64_t)out0 << 48 | (uint64_t)out1 << 32 | (uint64_t)out2 << 16 | (uint64_t)out3;
}

uint64_t HELPER(neon_widen_u16)(uint32_t a)
{
    const uint32_t out1 = U16_0(a);
    const uint32_t out0 = U16_1(a);

    return (uint64_t)out0 << 32 | (uint64_t)out1;
}

uint64_t HELPER(neon_widen_s16)(uint32_t a)
{
    const uint32_t out1 = S16_0(a);
    const uint32_t out0 = S16_1(a);

    return (uint64_t)out0 << 32 | (uint64_t)out1;
}

uint32_t HELPER(neon_narrow_u8)(uint64_t a)
{
    const uint8_t out3 = U16_0(a);
    const uint8_t out2 = U16_1(a);
    const uint8_t out1 = U16_2(a);
    const uint8_t out0 = U16_3(a);

    return (uint32_t)out0 << 24 | (uint32_t)out1 << 16 | (uint32_t)out2 << 8 | (uint32_t)out3;
}

uint32_t HELPER(neon_narrow_u16)(uint64_t a)
{
    const uint16_t out1 = U32_0(a);
    const uint16_t out0 = U32_1(a);
    return (uint32_t)out0 << 16 | (uint32_t)out1;
}

uint32_t HELPER(neon_narrow_high_u8)(uint64_t a)
{
    const uint8_t out3 = U16_0(a) >> 8;
    const uint8_t out2 = U16_1(a) >> 8;
    const uint8_t out1 = U16_2(a) >> 8;
    const uint8_t out0 = U16_3(a) >> 8;

    return (uint32_t)out0 << 24 | (uint32_t)out1 << 16 | (uint32_t)out2 << 8 | (uint32_t)out3;
}

uint32_t HELPER(neon_narrow_high_u16)(uint64_t a)
{
    const uint16_t out1 = U32_0(a) >> 16;
    const uint16_t out0 = U32_1(a) >> 16;
    return (uint32_t)out0 << 16 | (uint32_t)out1;
}

uint32_t HELPER(neon_narrow_round_high_u8)(uint64_t a)
{
    uint8_t out3 = U16_0(a) >> 8;
    uint8_t out2 = U16_1(a) >> 8;
    uint8_t out1 = U16_2(a) >> 8;
    uint8_t out0 = U16_3(a) >> 8;
    out3 += (U16_0(a) & 0x80) >> 7;
    out2 += (U16_1(a) & 0x80) >> 7;
    out1 += (U16_2(a) & 0x80) >> 7;
    out0 += (U16_3(a) & 0x80) >> 7;

    return (uint32_t)out0 << 24 | (uint32_t)out1 << 16 | (uint32_t)out2 << 8 | (uint32_t)out3;
}

uint32_t HELPER(neon_narrow_round_high_u16)(uint64_t a)
{
    uint16_t out1 = U32_0(a) >> 16;
    uint16_t out0 = U32_1(a) >> 16;
    out1 += (U32_0(a) & 0x8000) >> 15;
    out0 += (U32_1(a) & 0x8000) >> 15;

    return (uint32_t)out0 << 16 | (uint32_t)out1;
}

static int8 narrow_sat_s8(CPUState *env, int16_t a)
{
    if(a > INT8_MAX) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return INT8_MAX;
    } else if(a < INT8_MIN) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return INT8_MIN;
    }

    return a;
}

uint32_t HELPER(neon_narrow_sat_s8)(CPUState *env, uint64_t a)
{
    const uint8_t out3 = narrow_sat_s8(env, S16_0(a));
    const uint8_t out2 = narrow_sat_s8(env, S16_1(a));
    const uint8_t out1 = narrow_sat_s8(env, S16_2(a));
    const uint8_t out0 = narrow_sat_s8(env, S16_3(a));

    return (uint32_t)out0 << 24 | (uint32_t)out1 << 16 | (uint32_t)out2 << 8 | (uint32_t)out3;
}

static uint8 narrow_sat_u8(CPUState *env, uint16_t a)
{
    if(a > UINT8_MAX) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return UINT8_MAX;
    }

    return a;
}

uint32_t HELPER(neon_narrow_sat_u8)(CPUState *env, uint64_t a)
{
    const uint8_t out3 = narrow_sat_u8(env, U16_0(a));
    const uint8_t out2 = narrow_sat_u8(env, U16_1(a));
    const uint8_t out1 = narrow_sat_u8(env, U16_2(a));
    const uint8_t out0 = narrow_sat_u8(env, U16_3(a));

    return (uint32_t)out0 << 24 | (uint32_t)out1 << 16 | (uint32_t)out2 << 8 | (uint32_t)out3;
}

static int16_t narrow_sat_s16(CPUState *env, int32_t a)
{
    if(a > INT16_MAX) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return INT16_MAX;
    } else if(a < INT16_MIN) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return INT16_MIN;
    }

    return a;
}

uint32_t HELPER(neon_narrow_sat_s16)(CPUState *env, uint64_t a)
{
    const uint16_t out0 = narrow_sat_s16(env, S32_1(a));
    const uint16_t out1 = narrow_sat_s16(env, S32_0(a));

    return (uint32_t)out0 << 16 | (uint32_t)out1;
}

static uint16_t narrow_sat_u16(CPUState *env, uint32_t a)
{
    if(a > UINT16_MAX) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return UINT16_MAX;
    }

    return a;
}

uint32_t HELPER(neon_narrow_sat_u16)(CPUState *env, uint64_t a)
{
    const uint16_t out0 = narrow_sat_u16(env, U32_1(a));
    const uint16_t out1 = narrow_sat_u16(env, U32_0(a));

    return (uint32_t)out0 << 16 | (uint32_t)out1;
}

uint32_t HELPER(neon_narrow_sat_s32)(CPUState *env, uint64_t a)
{
    const int64_t sa = a;

    if(sa > INT32_MAX) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return INT32_MAX;
    } else if(sa < INT32_MIN) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return INT32_MIN;
    }

    return a;
}

uint32_t HELPER(neon_narrow_sat_u32)(CPUState *env, uint64_t a)
{
    if(a > UINT32_MAX) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return UINT32_MAX;
    }

    return a;
}

static uint8 unarrow_sat8(CPUState *env, int16_t a)
{
    if(a > UINT8_MAX) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return UINT8_MAX;
    } else if(a < 0) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return 0;
    }

    return a;
}

uint32_t HELPER(neon_unarrow_sat8)(CPUState *env, uint64_t a)
{
    const uint8_t out3 = unarrow_sat8(env, S16_0(a));
    const uint8_t out2 = unarrow_sat8(env, S16_1(a));
    const uint8_t out1 = unarrow_sat8(env, S16_2(a));
    const uint8_t out0 = unarrow_sat8(env, S16_3(a));

    return (uint32_t)out0 << 24 | (uint32_t)out1 << 16 | (uint32_t)out2 << 8 | (uint32_t)out3;
}

static uint16 unarrow_sat16(CPUState *env, int32_t a)
{
    if(a > UINT16_MAX) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return UINT16_MAX;
    } else if(a < 0) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return 0;
    }

    return a;
}

uint32_t HELPER(neon_unarrow_sat16)(CPUState *env, uint64_t a)
{
    const uint16_t out0 = unarrow_sat16(env, S32_1(a));
    const uint16_t out1 = unarrow_sat16(env, S32_0(a));

    return (uint32_t)out0 << 16 | (uint32_t)out1;
}

uint32_t HELPER(neon_unarrow_sat32)(CPUState *env, uint64_t a)
{
    const int64_t sa = a;

    if(sa > UINT32_MAX) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return UINT32_MAX;
    } else if(sa < 0) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        return 0;
    }

    return a;
}

uint64_t HELPER(neon_addl_saturate_s32)(CPUState *env, uint64_t a, uint64_t b)
{
    int64_t hi = (int64_t)S32_1(a) + (int64_t)S32_1(b);
    int64_t lo = (int64_t)S32_0(a) + (int64_t)S32_0(b);

    if(hi > INT32_MAX) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        hi = INT32_MAX;
    } else if(hi < INT32_MIN) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        hi = INT32_MIN;
    }

    if(lo > INT32_MAX) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        lo = INT32_MAX;
    } else if(lo < INT32_MIN) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        lo = INT32_MIN;
    }

    const uint32_t hi32 = hi;
    const uint32_t lo32 = lo;

    return (uint64_t)hi32 << 32 | (uint64_t)lo32;
}

uint64_t HELPER(neon_addl_saturate_s64)(CPUState *env, uint64_t a, uint64_t b)
{
    const int64_t sa = a;
    const int64_t sb = b;

    int64_t sum = sa + sb;

    if(sa > 0 && sb > 0 && sum < 0) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        sum = INT64_MAX;
    } else if(sa < 0 && sb < 0 && sum > 0) {
        env->vfp.xregs[ARM_VFP_FPSCR] |= CPSR_Q;
        sum = INT64_MIN;
    }

    return sum;
}

void HELPER(neon_zip8)(CPUState *env, uint32_t aNum, uint32_t bNum)
{
    tlib_abortf("neon_zip8 unimplemented");

    //  TODO: Enable after fixing VFP's regs (zregs); now these are 'ARMVectorReg'
    //        which for AArch64 is 'uint64_t[32]' instead of 'float64').
    //  uint64_t *aOut = &env->vfp.regs[aNum];
    //  uint64_t *bOut = &env->vfp.regs[bNum];
    //  const uint64_t a = *aOut;
    //  const uint64_t b = *bOut;

    //  *aOut = U8_3(b);
    //  *aOut <<= 8;
    //  *aOut |= U8_3(a);
    //  *aOut <<= 8;
    //  *aOut |= U8_2(b);
    //  *aOut <<= 8;
    //  *aOut |= U8_2(a);
    //  *aOut <<= 8;
    //  *aOut |= U8_1(b);
    //  *aOut <<= 8;
    //  *aOut |= U8_1(a);
    //  *aOut <<= 8;
    //  *aOut |= U8_0(b);
    //  *aOut <<= 8;
    //  *aOut |= U8_0(a);

    //  *bOut = U8_7(b);
    //  *bOut <<= 8;
    //  *bOut |= U8_7(a);
    //  *bOut <<= 8;
    //  *bOut |= U8_6(b);
    //  *bOut <<= 8;
    //  *bOut |= U8_6(a);
    //  *bOut <<= 8;
    //  *bOut |= U8_5(b);
    //  *bOut <<= 8;
    //  *bOut |= U8_5(a);
    //  *bOut <<= 8;
    //  *bOut |= U8_4(b);
    //  *bOut <<= 8;
    //  *bOut |= U8_4(a);
}

void HELPER(neon_qzip8)(CPUState *env, uint32_t da0Num, uint32_t db0Num)
{
    tlib_abortf("neon_qzip8 unimplemented");

    //  TODO: Enable after fixing VFP's regs (zregs); now these are 'ARMVectorReg'
    //        which for AArch64 is 'uint64_t[32]' instead of 'float64').
    //  const int da1Num = da0Num + 1;
    //  const int db1Num = db0Num + 1;
    //  uint64_t *da0Out = &env->vfp.regs[da0Num];
    //  uint64_t *da1Out = &env->vfp.regs[da1Num];
    //  uint64_t *db0Out = &env->vfp.regs[db0Num];
    //  uint64_t *db1Out = &env->vfp.regs[db1Num];
    //  const uint64_t da0 = *da0Out;
    //  const uint64_t da1 = *da1Out;
    //  const uint64_t db0 = *db0Out;
    //  const uint64_t db1 = *db1Out;

    //  *da1Out = U8_7(db0);
    //  *da1Out <<= 8;
    //  *da1Out |= U8_7(da0);
    //  *da1Out <<= 8;
    //  *da1Out |= U8_6(db0);
    //  *da1Out <<= 8;
    //  *da1Out |= U8_6(da0);
    //  *da1Out <<= 8;
    //  *da1Out |= U8_5(db0);
    //  *da1Out <<= 8;
    //  *da1Out |= U8_5(da0);
    //  *da1Out <<= 8;
    //  *da1Out |= U8_4(db0);
    //  *da1Out <<= 8;
    //  *da1Out |= U8_4(da0);

    //  *da0Out = U8_3(db0);
    //  *da0Out <<= 8;
    //  *da0Out |= U8_3(da0);
    //  *da0Out <<= 8;
    //  *da0Out |= U8_2(db0);
    //  *da0Out <<= 8;
    //  *da0Out |= U8_2(da0);
    //  *da0Out <<= 8;
    //  *da0Out |= U8_1(db0);
    //  *da0Out <<= 8;
    //  *da0Out |= U8_1(da0);
    //  *da0Out <<= 8;
    //  *da0Out |= U8_0(db0);
    //  *da0Out <<= 8;
    //  *da0Out |= U8_0(da0);

    //  *db1Out = U8_7(db1);
    //  *db1Out <<= 8;
    //  *db1Out |= U8_7(da1);
    //  *db1Out <<= 8;
    //  *db1Out |= U8_6(db1);
    //  *db1Out <<= 8;
    //  *db1Out |= U8_6(da1);
    //  *db1Out <<= 8;
    //  *db1Out |= U8_5(db1);
    //  *db1Out <<= 8;
    //  *db1Out |= U8_5(da1);
    //  *db1Out <<= 8;
    //  *db1Out |= U8_4(db1);
    //  *db1Out <<= 8;
    //  *db1Out |= U8_4(da1);

    //  *db0Out = U8_3(db1);
    //  *db0Out <<= 8;
    //  *db0Out |= U8_3(da1);
    //  *db0Out <<= 8;
    //  *db0Out |= U8_2(db1);
    //  *db0Out <<= 8;
    //  *db0Out |= U8_2(da1);
    //  *db0Out <<= 8;
    //  *db0Out |= U8_1(db1);
    //  *db0Out <<= 8;
    //  *db0Out |= U8_1(da1);
    //  *db0Out <<= 8;
    //  *db0Out |= U8_0(db1);
    //  *db0Out <<= 8;
    //  *db0Out |= U8_0(da1);
}

void HELPER(neon_unzip8)(CPUState *env, uint32_t aNum, uint32_t bNum)
{
    tlib_abortf("neon_unzip8 unimplemented");

    //  TODO: Enable after fixing VFP's regs (zregs); now these are 'ARMVectorReg'
    //        which for AArch64 is 'uint64_t[32]' instead of 'float64').
    //  uint64_t *aOut = &env->vfp.regs[aNum];
    //  uint64_t *bOut = &env->vfp.regs[bNum];
    //  const uint64_t a = *aOut;
    //  const uint64_t b = *bOut;

    //  *aOut = U8_6(b);
    //  *aOut <<= 8;
    //  *aOut |= U8_4(b);
    //  *aOut <<= 8;
    //  *aOut |= U8_2(b);
    //  *aOut <<= 8;
    //  *aOut |= U8_0(b);
    //  *aOut <<= 8;
    //  *aOut |= U8_6(a);
    //  *aOut <<= 8;
    //  *aOut |= U8_4(a);
    //  *aOut <<= 8;
    //  *aOut |= U8_2(a);
    //  *aOut <<= 8;
    //  *aOut |= U8_0(a);

    //  *bOut = U8_7(b);
    //  *bOut <<= 8;
    //  *bOut |= U8_5(b);
    //  *bOut <<= 8;
    //  *bOut |= U8_3(b);
    //  *bOut <<= 8;
    //  *bOut |= U8_1(b);
    //  *bOut <<= 8;
    //  *bOut |= U8_7(a);
    //  *bOut <<= 8;
    //  *bOut |= U8_5(a);
    //  *bOut <<= 8;
    //  *bOut |= U8_3(a);
    //  *bOut <<= 8;
    //  *bOut |= U8_1(a);
}

void HELPER(neon_qunzip8)(CPUState *env, uint32_t da0Num, uint32_t db0Num)
{
    tlib_abortf("neon_qunzip8 unimplemented");

    //  TODO: Enable after fixing VFP's regs (zregs); now these are 'ARMVectorReg'
    //        which for AArch64 is 'uint64_t[32]' instead of 'float64').
    //  const int da1Num = da0Num + 1;
    //  const int db1Num = db0Num + 1;
    //  uint64_t *da0Out = &env->vfp.regs[da0Num];
    //  uint64_t *da1Out = &env->vfp.regs[da1Num];
    //  uint64_t *db0Out = &env->vfp.regs[db0Num];
    //  uint64_t *db1Out = &env->vfp.regs[db1Num];
    //  const uint64_t da0 = *da0Out;
    //  const uint64_t da1 = *da1Out;
    //  const uint64_t db0 = *db0Out;
    //  const uint64_t db1 = *db1Out;

    //  *da1Out = U8_6(db1);
    //  *da1Out <<= 8;
    //  *da1Out |= U8_4(db1);
    //  *da1Out <<= 8;
    //  *da1Out |= U8_2(db1);
    //  *da1Out <<= 8;
    //  *da1Out |= U8_0(db1);
    //  *da1Out <<= 8;
    //  *da1Out |= U8_6(db0);
    //  *da1Out <<= 8;
    //  *da1Out |= U8_4(db0);
    //  *da1Out <<= 8;
    //  *da1Out |= U8_2(db0);
    //  *da1Out <<= 8;
    //  *da1Out |= U8_0(db0);

    //  *da0Out = U8_6(da1);
    //  *da0Out <<= 8;
    //  *da0Out |= U8_4(da1);
    //  *da0Out <<= 8;
    //  *da0Out |= U8_2(da1);
    //  *da0Out <<= 8;
    //  *da0Out |= U8_0(da1);
    //  *da0Out <<= 8;
    //  *da0Out |= U8_6(da0);
    //  *da0Out <<= 8;
    //  *da0Out |= U8_4(da0);
    //  *da0Out <<= 8;
    //  *da0Out |= U8_2(da0);
    //  *da0Out <<= 8;
    //  *da0Out |= U8_0(da0);

    //  *db1Out = U8_7(db1);
    //  *db1Out <<= 8;
    //  *db1Out |= U8_5(db1);
    //  *db1Out <<= 8;
    //  *db1Out |= U8_3(db1);
    //  *db1Out <<= 8;
    //  *db1Out |= U8_1(db1);
    //  *db1Out <<= 8;
    //  *db1Out |= U8_7(db0);
    //  *db1Out <<= 8;
    //  *db1Out |= U8_5(db0);
    //  *db1Out <<= 8;
    //  *db1Out |= U8_3(db0);
    //  *db1Out <<= 8;
    //  *db1Out |= U8_1(db0);

    //  *db0Out = U8_7(da1);
    //  *db0Out <<= 8;
    //  *db0Out |= U8_5(da1);
    //  *db0Out <<= 8;
    //  *db0Out |= U8_3(da1);
    //  *db0Out <<= 8;
    //  *db0Out |= U8_1(da1);
    //  *db0Out <<= 8;
    //  *db0Out |= U8_7(da0);
    //  *db0Out <<= 8;
    //  *db0Out |= U8_5(da0);
    //  *db0Out <<= 8;
    //  *db0Out |= U8_3(da0);
    //  *db0Out <<= 8;
    //  *db0Out |= U8_1(da0);
}

void HELPER(neon_zip16)(CPUState *env, uint32_t aNum, uint32_t bNum)
{
    tlib_abortf("neon_zip16 unimplemented");

    //  TODO: Enable after fixing VFP's regs (zregs); now these are 'ARMVectorReg'
    //        which for AArch64 is 'uint64_t[32]' instead of 'float64').
    //  uint64_t *aOut = &env->vfp.regs[aNum];
    //  uint64_t *bOut = &env->vfp.regs[bNum];
    //  const uint64_t a = *aOut;
    //  const uint64_t b = *bOut;

    //  *aOut = U16_1(b);
    //  *aOut <<= 16;
    //  *aOut |= U16_1(a);
    //  *aOut <<= 16;
    //  *aOut |= U16_0(b);
    //  *aOut <<= 16;
    //  *aOut |= U16_0(a);

    //  *bOut = U16_3(b);
    //  *bOut <<= 16;
    //  *bOut |= U16_3(a);
    //  *bOut <<= 16;
    //  *bOut |= U16_2(b);
    //  *bOut <<= 16;
    //  *bOut |= U16_2(a);
}

void HELPER(neon_qzip16)(CPUState *env, uint32_t da0Num, uint32_t db0Num)
{
    tlib_abortf("neon_qzip16 unimplemented");

    //  TODO: Enable after fixing VFP's regs (zregs); now these are 'ARMVectorReg'
    //        which for AArch64 is 'uint64_t[32]' instead of 'float64').
    //  const int da1Num = da0Num + 1;
    //  const int db1Num = db0Num + 1;
    //  uint64_t *da0Out = &env->vfp.regs[da0Num];
    //  uint64_t *da1Out = &env->vfp.regs[da1Num];
    //  uint64_t *db0Out = &env->vfp.regs[db0Num];
    //  uint64_t *db1Out = &env->vfp.regs[db1Num];
    //  const uint64_t da0 = *da0Out;
    //  const uint64_t da1 = *da1Out;
    //  const uint64_t db0 = *db0Out;
    //  const uint64_t db1 = *db1Out;

    //  *da1Out = U16_3(db0);
    //  *da1Out <<= 16;
    //  *da1Out |= U16_3(da0);
    //  *da1Out <<= 16;
    //  *da1Out |= U16_2(db0);
    //  *da1Out <<= 16;
    //  *da1Out |= U16_2(da0);

    //  *da0Out = U16_1(db0);
    //  *da0Out <<= 16;
    //  *da0Out |= U16_1(da0);
    //  *da0Out <<= 16;
    //  *da0Out |= U16_0(db0);
    //  *da0Out <<= 16;
    //  *da0Out |= U16_0(da0);

    //  *db1Out = U16_3(db1);
    //  *db1Out <<= 16;
    //  *db1Out |= U16_3(da1);
    //  *db1Out <<= 16;
    //  *db1Out |= U16_2(db1);
    //  *db1Out <<= 16;
    //  *db1Out |= U16_2(da1);

    //  *db0Out = U16_1(db1);
    //  *db0Out <<= 16;
    //  *db0Out |= U16_1(da1);
    //  *db0Out <<= 16;
    //  *db0Out |= U16_0(db1);
    //  *db0Out <<= 16;
    //  *db0Out |= U16_0(da1);
}

void HELPER(neon_unzip16)(CPUState *env, uint32_t aNum, uint32_t bNum)
{
    tlib_abortf("neon_unzip16 unimplemented");

    //  TODO: Enable after fixing VFP's regs (zregs); now these are 'ARMVectorReg'
    //        which for AArch64 is 'uint64_t[32]' instead of 'float64').
    //  uint64_t *aOut = &env->vfp.regs[aNum];
    //  uint64_t *bOut = &env->vfp.regs[bNum];
    //  const uint64_t a = *aOut;
    //  const uint64_t b = *bOut;

    //  *aOut = U16_2(b);
    //  *aOut <<= 16;
    //  *aOut |= U16_0(b);
    //  *aOut <<= 16;
    //  *aOut |= U16_2(a);
    //  *aOut <<= 16;
    //  *aOut |= U16_0(a);

    //  *bOut = U16_3(b);
    //  *bOut <<= 16;
    //  *bOut |= U16_1(b);
    //  *bOut <<= 16;
    //  *bOut |= U16_3(a);
    //  *bOut <<= 16;
    //  *bOut |= U16_1(a);
}

void HELPER(neon_qunzip16)(CPUState *env, uint32_t da0Num, uint32_t db0Num)
{
    tlib_abortf("neon_qunzip16 unimplemented");

    //  TODO: Enable after fixing VFP's regs (zregs); now these are 'ARMVectorReg'
    //        which for AArch64 is 'uint64_t[32]' instead of 'float64').
    //  const int da1Num = da0Num + 1;
    //  const int db1Num = db0Num + 1;
    //  uint64_t *da0Out = &env->vfp.regs[da0Num];
    //  uint64_t *da1Out = &env->vfp.regs[da1Num];
    //  uint64_t *db0Out = &env->vfp.regs[db0Num];
    //  uint64_t *db1Out = &env->vfp.regs[db1Num];
    //  const uint64_t da0 = *da0Out;
    //  const uint64_t da1 = *da1Out;
    //  const uint64_t db0 = *db0Out;
    //  const uint64_t db1 = *db1Out;

    //  *da1Out = U16_2(db1);
    //  *da1Out <<= 16;
    //  *da1Out |= U16_0(db1);
    //  *da1Out <<= 16;
    //  *da1Out |= U16_2(db0);
    //  *da1Out <<= 16;
    //  *da1Out |= U16_0(db0);

    //  *da0Out = U16_2(da1);
    //  *da0Out <<= 16;
    //  *da0Out |= U16_0(da1);
    //  *da0Out <<= 16;
    //  *da0Out |= U16_2(da0);
    //  *da0Out <<= 16;
    //  *da0Out |= U16_0(da0);

    //  *db1Out = U16_3(db1);
    //  *db1Out <<= 16;
    //  *db1Out |= U16_1(db1);
    //  *db1Out <<= 16;
    //  *db1Out |= U16_3(db0);
    //  *db1Out <<= 16;
    //  *db1Out |= U16_1(db0);

    //  *db0Out = U16_3(da1);
    //  *db0Out <<= 16;
    //  *db0Out |= U16_1(da1);
    //  *db0Out <<= 16;
    //  *db0Out |= U16_3(da0);
    //  *db0Out <<= 16;
    //  *db0Out |= U16_1(da0);
}

void HELPER(neon_qzip32)(CPUState *env, uint32_t da0Num, uint32_t db0Num)
{
    tlib_abortf("neon_qzip32 unimplemented");

    //  TODO: Enable after fixing VFP's regs (zregs); now these are 'ARMVectorReg'
    //        which for AArch64 is 'uint64_t[32]' instead of 'float64').
    //  const int da1Num = da0Num + 1;
    //  const int db1Num = db0Num + 1;
    //  uint64_t *da0Out = &env->vfp.regs[da0Num];
    //  uint64_t *da1Out = &env->vfp.regs[da1Num];
    //  uint64_t *db0Out = &env->vfp.regs[db0Num];
    //  uint64_t *db1Out = &env->vfp.regs[db1Num];
    //  const uint64_t da0 = *da0Out;
    //  const uint64_t da1 = *da1Out;
    //  const uint64_t db0 = *db0Out;
    //  const uint64_t db1 = *db1Out;

    //  *da1Out = U32_1(db0);
    //  *da1Out <<= 32;
    //  *da1Out |= U32_1(da0);

    //  *da0Out = U32_0(db0);
    //  *da0Out <<= 32;
    //  *da0Out |= U32_0(da0);

    //  *db1Out = U32_1(db1);
    //  *db1Out <<= 32;
    //  *db1Out |= U32_1(da1);

    //  *db0Out = U32_0(db1);
    //  *db0Out <<= 32;
    //  *db0Out |= U32_0(da1);
}

void HELPER(neon_qunzip32)(CPUState *env, uint32_t da0Num, uint32_t db0Num)
{
    tlib_abortf("neon_qunzip32 unimplemented");

    //  TODO: Enable after fixing VFP's regs (zregs); now these are 'ARMVectorReg'
    //        which for AArch64 is 'uint64_t[32]' instead of 'float64').
    //  const int da1Num = da0Num + 1;
    //  const int db1Num = db0Num + 1;
    //  uint64_t *da0Out = &env->vfp.regs[da0Num];
    //  uint64_t *da1Out = &env->vfp.regs[da1Num];
    //  uint64_t *db0Out = &env->vfp.regs[db0Num];
    //  uint64_t *db1Out = &env->vfp.regs[db1Num];
    //  const uint64_t da0 = *da0Out;
    //  const uint64_t da1 = *da1Out;
    //  const uint64_t db0 = *db0Out;
    //  const uint64_t db1 = *db1Out;

    //  *da1Out = U32_0(db1);
    //  *da1Out <<= 32;
    //  *da1Out |= U32_0(db0);

    //  *da0Out = U32_0(da1);
    //  *da0Out <<= 32;
    //  *da0Out |= U32_0(da0);

    //  *db1Out = U32_1(db1);
    //  *db1Out <<= 32;
    //  *db1Out |= U32_1(db0);

    //  *db0Out = U32_1(da1);
    //  *db0Out <<= 32;
    //  *db0Out |= U32_1(da0);
}

uint32_t HELPER(neon_abd_f32)(uint32_t a, uint32_t b, void *fpstatus)
{
    float32 af = make_float32(a);
    float32 bf = make_float32(b);
    float32 diff = float32_sub(af, bf, (float_status *)fpstatus);
    float32 diff_abs = float32_abs(diff);
    return float32_val(diff_abs);
}

uint32_t HELPER(neon_min_f32)(uint32_t a, uint32_t b, void *fpstatus)
{
    float32 af = make_float32(a);
    float32 bf = make_float32(b);
    float32 min = float32_min(af, bf, (float_status *)fpstatus);
    return float32_val(min);
}

uint32_t HELPER(neon_max_f32)(uint32_t a, uint32_t b, void *fpstatus)
{
    float32 af = make_float32(a);
    float32 bf = make_float32(b);
    float32 max = float32_max(af, bf, (float_status *)fpstatus);
    return float32_val(max);
}

uint32_t HELPER(neon_ceq_f32)(uint32_t a, uint32_t b, void *fpstatus)
{
    float32 af = make_float32(a);
    float32 bf = make_float32(b);
    int32_t res = float32_eq_quiet(af, bf, (float_status *)fpstatus);
    return res ? -1 : 0;
}

uint32_t HELPER(neon_cge_f32)(uint32_t a, uint32_t b, void *fpstatus)
{
    float32 af = make_float32(a);
    float32 bf = make_float32(b);
    int32_t res = float32_le(bf, af, (float_status *)fpstatus);
    return res ? -1 : 0;
}

uint32_t HELPER(neon_cgt_f32)(uint32_t a, uint32_t b, void *fpstatus)
{
    float32 af = make_float32(a);
    float32 bf = make_float32(b);
    int32_t res = float32_lt(bf, af, (float_status *)fpstatus);
    return res ? -1 : 0;
}

uint32_t HELPER(neon_acge_f32)(uint32_t a, uint32_t b, void *fpstatus)
{
    float32 af = make_float32(a);
    float32 bf = make_float32(b);
    float32 af_abs = float32_abs(af);
    float32 bf_abs = float32_abs(bf);
    int32_t res = float32_le(bf_abs, af_abs, (float_status *)fpstatus);
    return res ? -1 : 0;
}

uint32_t HELPER(neon_acgt_f32)(uint32_t a, uint32_t b, void *fpstatus)
{
    float32 af = make_float32(a);
    float32 bf = make_float32(b);
    float32 af_abs = float32_abs(af);
    float32 bf_abs = float32_abs(bf);
    int32_t res = float32_lt(bf_abs, af_abs, (float_status *)fpstatus);
    return res ? -1 : 0;
}
