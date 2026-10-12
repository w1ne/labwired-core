/*
 *  RISC-V FPU emulation helpers
 *
 *  Author: Sagar Karandikar, sagark@eecs.berkeley.edu
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

/*
 * Parts of this code are derived from Spike https://github.com/riscv-software-src/riscv-isa-sim,
 * under the following license:
 */

/*============================================================================

This C source file is part of the SoftFloat IEEE Floating-Point Arithmetic
Package, Release 3d, by John R. Hauser.

Copyright 2011, 2012, 2013, 2014, 2015, 2016 The Regents of the University of
California.  All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

 1. Redistributions of source code must retain the above copyright notice,
    this list of conditions, and the following disclaimer.

 2. Redistributions in binary form must reproduce the above copyright notice,
    this list of conditions, and the following disclaimer in the documentation
    and/or other materials provided with the distribution.

 3. Neither the name of the University nor the names of its contributors may
    be used to endorse or promote products derived from this software without
    specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE REGENTS AND CONTRIBUTORS "AS IS", AND ANY
EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE, ARE
DISCLAIMED.  IN NO EVENT SHALL THE REGENTS OR CONTRIBUTORS BE LIABLE FOR ANY
DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
(INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

=============================================================================*/

#include "cpu.h"

/* env->fp_status is not used with SoftFloat 3.
 * SoftFloat 3 relies on global variables initialized by the library
 * to control rounding mode and store exception flags.
 */
extern THREAD_LOCAL uint_fast8_t softfloat_roundingMode;
extern THREAD_LOCAL uint_fast8_t softfloat_exceptionFlags;

typedef enum {
    riscv_float_round_nearest_even = 0,
    riscv_float_round_to_zero = 1,
    riscv_float_round_down = 2,
    riscv_float_round_up = 3,
    riscv_float_round_ties_away = 4
} riscv_float_round_mode;

typedef enum {
    riscv_float_exception_inexact = 1,             //  NX
    riscv_float_exception_underflow = 2,           //  UF
    riscv_float_exception_overflow = 4,            //  OF
    riscv_float_exception_divide_by_zero = 8,      //  DZ
    riscv_float_exception_invalid_operation = 16,  //  NV
} riscv_float_exception_flag;

/* obtain rm value to use in computation
 * as the last step, convert rm codes to what the SoftFloat-3 library expects
 * Adapted from Spike's decode.h:RM
 */
#define RM_3                                                                        \
    ({                                                                              \
        if(rm == 7) {                                                               \
            rm = env->frm;                                                          \
        }                                                                           \
        if(rm > 4) {                                                                \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);              \
        }                                                                           \
        /* Because RISC-V and SoftFloat-3 has a one-to-one relation of its rounding \
           mode values, it is enough to return the value of `rm` immediately. */    \
        rm;                                                                         \
    })

#define require_fp                                                 \
    if(!(env->mstatus & MSTATUS_FS)) {                             \
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST); \
    }

/* adapted from spike */
#define isNaNF32UI(ui) (0xFF000000 < (uint32_t)((uint_fast32_t)ui << 1))
#define signF32UI(a)   ((bool)((uint32_t)a >> 31))
#define expF32UI(a)    ((int_fast16_t)(a >> 23) & 0xFF)
#define fracF32UI(a)   (a & 0x007FFFFF)

union ui32_f32 {
    uint32_t ui;
    uint32_t f;
};

#define isNaNF64UI(ui) (UINT64_C(0xFFE0000000000000) < (uint64_t)((uint_fast64_t)ui << 1))
#define signF64UI(a)   ((bool)((uint64_t)a >> 63))
#define expF64UI(a)    ((int_fast16_t)(a >> 52) & 0x7FF)
#define fracF64UI(a)   (a & UINT64_C(0x000FFFFFFFFFFFFF))

union ui64_f64 {
    uint64_t ui;
    uint64_t f;
};

unsigned int softfloat3_flags_to_riscv(unsigned int flags)
{
    int rv_flags = 0;
    rv_flags |= (flags & softfloat_flag_inexact) ? riscv_float_exception_inexact : 0;
    rv_flags |= (flags & softfloat_flag_underflow) ? riscv_float_exception_underflow : 0;
    rv_flags |= (flags & softfloat_flag_overflow) ? riscv_float_exception_overflow : 0;
    rv_flags |= (flags & softfloat_flag_infinite) ? riscv_float_exception_divide_by_zero : 0;
    rv_flags |= (flags & softfloat_flag_invalid) ? riscv_float_exception_invalid_operation : 0;
    return rv_flags;
}

void set_float3_rounding_mode(int val)
{
    softfloat_roundingMode = (uint_fast8_t)val;
}

#define set_fp3_exceptions()                                                \
    do {                                                                    \
        env->fflags |= softfloat3_flags_to_riscv(softfloat_exceptionFlags); \
        softfloat_exceptionFlags = 0;                                       \
    } while(0)

uint64_t helper_fmadd_s(CPUState *env, uint64_t frs1, uint64_t frs2, uint64_t frs3, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float32_t f1, f2, f3;
    f1.v = (uint32_t)frs1;
    f2.v = (uint32_t)frs2;
    f3.v = (uint32_t)frs3;
    frs1 = f32_mulAdd(f1, f2, f3).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fmadd_d(CPUState *env, uint64_t frs1, uint64_t frs2, uint64_t frs3, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float64_t f1, f2, f3;
    f1.v = frs1;
    f2.v = frs2;
    f3.v = frs3;
    frs1 = f64_mulAdd(f1, f2, f3).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fmadd_h(CPUState *env, uint64_t frs1, uint64_t frs2, uint64_t frs3, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float16_t f1, f2, f3;
    f1.v = (uint16_t)frs1;
    f2.v = (uint16_t)frs2;
    f3.v = (uint16_t)frs3;
    frs1 = f16_mulAdd(f1, f2, f3).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fmsub_s(CPUState *env, uint64_t frs1, uint64_t frs2, uint64_t frs3, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float32_t f1, f2, f3;
    f1.v = (uint32_t)frs1;
    f2.v = (uint32_t)frs2;
    f3.v = (uint32_t)(frs3 ^ (uint64_t)INT32_MIN);
    frs1 = f32_mulAdd(f1, f2, f3).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fmsub_d(CPUState *env, uint64_t frs1, uint64_t frs2, uint64_t frs3, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float64_t f1, f2, f3;
    f1.v = frs1;
    f2.v = frs2;
    f3.v = frs3 ^ (uint64_t)INT64_MIN;
    frs1 = f64_mulAdd(f1, f2, f3).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fmsub_h(CPUState *env, uint64_t frs1, uint64_t frs2, uint64_t frs3, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float16_t f1, f2, f3;
    f1.v = (uint16_t)frs1;
    f2.v = (uint16_t)frs2;
    f3.v = (uint16_t)(frs3 ^ (uint64_t)INT16_MIN);
    frs1 = f16_mulAdd(f1, f2, f3).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fnmsub_s(CPUState *env, uint64_t frs1, uint64_t frs2, uint64_t frs3, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float32_t f1, f2, f3;
    f1.v = (uint32_t)(frs1 ^ (uint64_t)INT32_MIN);
    f2.v = (uint32_t)frs2;
    f3.v = (uint32_t)frs3;
    frs1 = f32_mulAdd(f1, f2, f3).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fnmsub_d(CPUState *env, uint64_t frs1, uint64_t frs2, uint64_t frs3, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float64_t f1, f2, f3;
    f1.v = frs1 ^ (uint64_t)INT64_MIN;
    f2.v = frs2;
    f3.v = frs3;
    frs1 = f64_mulAdd(f1, f2, f3).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fnmsub_h(CPUState *env, uint64_t frs1, uint64_t frs2, uint64_t frs3, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float16_t f1, f2, f3;
    f1.v = (uint16_t)(frs1 ^ (uint64_t)INT16_MIN);
    f2.v = (uint16_t)frs2;
    f3.v = (uint16_t)frs3;
    frs1 = f16_mulAdd(f1, f2, f3).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fnmadd_s(CPUState *env, uint64_t frs1, uint64_t frs2, uint64_t frs3, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float32_t f1, f2, f3;
    f1.v = (uint32_t)(frs1 ^ (uint64_t)INT32_MIN);
    f2.v = (uint32_t)frs2;
    f3.v = (uint32_t)(frs3 ^ (uint64_t)INT32_MIN);
    frs1 = f32_mulAdd(f1, f2, f3).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fnmadd_d(CPUState *env, uint64_t frs1, uint64_t frs2, uint64_t frs3, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float64_t f1, f2, f3;
    f1.v = frs1 ^ (uint64_t)INT64_MIN;
    f2.v = frs2;
    f3.v = frs3 ^ (uint64_t)INT64_MIN;
    frs1 = f64_mulAdd(f1, f2, f3).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fnmadd_h(CPUState *env, uint64_t frs1, uint64_t frs2, uint64_t frs3, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float16_t f1, f2, f3;
    f1.v = (uint16_t)(frs1 ^ (uint64_t)INT16_MIN);
    f2.v = (uint16_t)frs2;
    f3.v = (uint16_t)(frs3 ^ (uint64_t)INT16_MIN);
    frs1 = f16_mulAdd(f1, f2, f3).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fadd_s(CPUState *env, uint64_t frs1, uint64_t frs2, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float32_t f1, f2;
    f1.v = (uint32_t)frs1;
    f2.v = (uint32_t)frs2;
    frs1 = f32_add(f1, f2).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fsub_s(CPUState *env, uint64_t frs1, uint64_t frs2, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float32_t f1, f2;
    f1.v = (uint32_t)frs1;
    f2.v = (uint32_t)frs2;
    frs1 = f32_sub(f1, f2).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fmul_s(CPUState *env, uint64_t frs1, uint64_t frs2, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float32_t f1, f2;
    f1.v = (uint32_t)frs1;
    f2.v = (uint32_t)frs2;
    frs1 = f32_mul(f1, f2).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fdiv_s(CPUState *env, uint64_t frs1, uint64_t frs2, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float32_t f1, f2;
    f1.v = (uint32_t)frs1;
    f2.v = (uint32_t)frs2;
    frs1 = f32_div(f1, f2).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fmin_s(CPUState *env, uint64_t frs1, uint64_t frs2)
{
    require_fp;

    float32_t f1, f2;
    f1.v = (uint32_t)frs1;
    f2.v = (uint32_t)frs2;
    frs1 = f32_min(f1, f2).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fmax_s(CPUState *env, uint64_t frs1, uint64_t frs2)
{
    require_fp;

    float32_t f1, f2;
    f1.v = (uint32_t)frs1;
    f2.v = (uint32_t)frs2;
    frs1 = f32_max(f1, f2).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fsqrt_s(CPUState *env, uint64_t frs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float32_t f1;
    f1.v = (uint32_t)frs1;
    frs1 = f32_sqrt(f1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

target_ulong helper_fle_s(CPUState *env, uint64_t frs1, uint64_t frs2)
{
    require_fp;

    float32_t f1, f2;
    f1.v = (uint32_t)frs1;
    f2.v = (uint32_t)frs2;
    frs1 = f32_le(f1, f2);

    set_fp3_exceptions();
    return frs1;
}

target_ulong helper_flt_s(CPUState *env, uint64_t frs1, uint64_t frs2)
{
    require_fp;

    float32_t f1, f2;
    f1.v = (uint32_t)frs1;
    f2.v = (uint32_t)frs2;
    frs1 = f32_lt(f1, f2);

    set_fp3_exceptions();
    return frs1;
}

target_ulong helper_fge_s(CPUState *env, uint64_t frs1, uint64_t frs2)
{
    return helper_fle_s(env, frs2, frs1);
}

target_ulong helper_fgt_s(CPUState *env, uint64_t frs1, uint64_t frs2)
{
    return helper_flt_s(env, frs2, frs1);
}

target_ulong helper_feq_s(CPUState *env, uint64_t frs1, uint64_t frs2)
{
    require_fp;

    float32_t f1, f2;
    f1.v = (uint32_t)frs1;
    f2.v = (uint32_t)frs2;
    frs1 = f32_eq(f1, f2);

    set_fp3_exceptions();
    return frs1;
}

target_ulong helper_fcvt_hw_s(CPUState *env, uint64_t frs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float32_t f1;
    f1.v = (uint32_t)frs1;
    frs1 = (int16_t)f32_to_i16(f1, RM_3, true);

    set_fp3_exceptions();
    return frs1;
}

target_ulong helper_fcvt_hwu_s(CPUState *env, uint64_t frs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float32_t f1;
    f1.v = (uint32_t)frs1;
    frs1 = (uint16_t)f32_to_ui16(f1, RM_3, true);

    set_fp3_exceptions();
    return frs1;
}

target_ulong helper_fcvt_w_s(CPUState *env, uint64_t frs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float32_t f1;
    f1.v = (uint32_t)frs1;
    frs1 = (int64_t)((int32_t)f32_to_i32(f1, RM_3, true));

    set_fp3_exceptions();
    return frs1;
}

target_ulong helper_fcvt_wu_s(CPUState *env, uint64_t frs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float32_t f1;
    f1.v = (uint32_t)frs1;
    frs1 = (int64_t)((int32_t)f32_to_ui32(f1, RM_3, true));

    set_fp3_exceptions();
    return frs1;
}

uint64_t helper_fcvt_l_s(CPUState *env, uint64_t frs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float32_t f1;
    f1.v = (uint32_t)frs1;
    frs1 = f32_to_i64(f1, RM_3, true);

    set_fp3_exceptions();
    return frs1;
}

uint64_t helper_fcvt_lu_s(CPUState *env, uint64_t frs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float32_t f1;
    f1.v = (uint32_t)frs1;
    frs1 = f32_to_ui64(f1, RM_3, true);

    set_fp3_exceptions();
    return frs1;
}

uint64_t helper_fcvt_s_hw(CPUState *env, target_ulong rs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    rs1 = i32_to_f32((int16_t)rs1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return rs1;
}

uint64_t helper_fcvt_s_hwu(CPUState *env, target_ulong rs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    rs1 = ui32_to_f32((uint16_t)rs1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return rs1;
}

uint64_t helper_fcvt_s_w(CPUState *env, target_ulong rs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    rs1 = i32_to_f32((uint32_t)rs1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return rs1;
}

uint64_t helper_fcvt_s_wu(CPUState *env, target_ulong rs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    uint64_t res;
    res = ui32_to_f32((uint32_t)rs1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return res;
}

uint64_t helper_fcvt_s_l(CPUState *env, uint64_t rs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    rs1 = i64_to_f32(rs1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return rs1;
}

uint64_t helper_fcvt_s_lu(CPUState *env, uint64_t rs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    rs1 = ui64_to_f32(rs1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return rs1;
}

uint32_t helper_fcvt_wu_s_rod(CPUState *env, uint32_t frs1)
{
    require_fp;
    set_float3_rounding_mode(softfloat_round_odd);

    float32_t f1;
    f1.v = (uint32_t)frs1;
    frs1 = (int64_t)((int32_t)f32_to_ui32(f1, softfloat_round_odd, true));

    set_fp3_exceptions();
    return frs1;
}

int32_t helper_fcvt_w_s_rod(CPUState *env, uint32_t frs1)
{
    require_fp;
    set_float3_rounding_mode(softfloat_round_odd);

    float32_t f1;
    f1.v = (uint32_t)frs1;
    frs1 = (int64_t)((int32_t)f32_to_i32(f1, softfloat_round_odd, true));

    set_fp3_exceptions();
    return frs1;
}

uint64_t helper_fcvt_lu_d_rod(CPUState *env, uint64_t frs1)
{
    require_fp;
    set_float3_rounding_mode(softfloat_round_odd);

    float64_t f1;
    f1.v = frs1;
    frs1 = f64_to_ui64(f1, softfloat_round_odd, true);

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

int64_t helper_fcvt_l_d_rod(CPUState *env, uint64_t frs1)
{
    require_fp;
    set_float3_rounding_mode(softfloat_round_odd);

    float64_t f1;
    f1.v = frs1;
    frs1 = f64_to_i64(f1, softfloat_round_odd, true);

    set_fp3_exceptions();
    return frs1;
}

uint64_t helper_fcvt_lu_s_rod(CPUState *env, uint32_t frs1)
{
    require_fp;
    set_float3_rounding_mode(softfloat_round_odd);

    float32_t f1;
    f1.v = (uint32_t)frs1;
    frs1 = f32_to_ui64(f1, softfloat_round_odd, true);

    set_fp3_exceptions();
    return frs1;
}

int64_t helper_fcvt_l_s_rod(CPUState *env, uint32_t frs1)
{
    require_fp;
    set_float3_rounding_mode(softfloat_round_odd);

    float32_t f1;
    f1.v = (uint32_t)frs1;
    frs1 = f32_to_i64(f1, softfloat_round_odd, true);

    set_fp3_exceptions();
    return frs1;
}

target_ulong helper_fclass_s(CPUState *env, uint64_t frs1)
{
    require_fp;

    float32_t f1;
    f1.v = (uint32_t)frs1;
    frs1 = f32_classify(f1);

    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fadd_d(CPUState *env, uint64_t frs1, uint64_t frs2, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float64_t f1, f2;
    f1.v = frs1;
    f2.v = frs2;
    frs1 = f64_add(f1, f2).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fsub_d(CPUState *env, uint64_t frs1, uint64_t frs2, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float64_t f1, f2;
    f1.v = frs1;
    f2.v = frs2;
    frs1 = f64_sub(f1, f2).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fmul_d(CPUState *env, uint64_t frs1, uint64_t frs2, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float64_t f1, f2;
    f1.v = frs1;
    f2.v = frs2;
    frs1 = f64_mul(f1, f2).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fdiv_d(CPUState *env, uint64_t frs1, uint64_t frs2, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float64_t f1, f2;
    f1.v = frs1;
    f2.v = frs2;
    frs1 = f64_div(f1, f2).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fmin_d(CPUState *env, uint64_t frs1, uint64_t frs2)
{
    require_fp;

    float64_t f1, f2;
    f1.v = frs1;
    f2.v = frs2;
    frs1 = f64_min(f1, f2).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fmax_d(CPUState *env, uint64_t frs1, uint64_t frs2)
{
    require_fp;

    float64_t f1, f2;
    f1.v = frs1;
    f2.v = frs2;
    frs1 = f64_max(f1, f2).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fcvt_s_d(CPUState *env, uint64_t rs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float64_t f1;
    f1.v = rs1;
    rs1 = f64_to_f32(f1).v;

    set_fp3_exceptions();
    return rs1;
}

uint64_t helper_fcvt_d_s(CPUState *env, uint64_t rs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float32_t f1;
    f1.v = (uint32_t)rs1;
    rs1 = f32_to_f64(f1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return rs1;
}

uint64_t helper_fsqrt_d(CPUState *env, uint64_t frs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float64_t f1;
    f1.v = frs1;
    frs1 = f64_sqrt(f1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

target_ulong helper_fle_d(CPUState *env, uint64_t frs1, uint64_t frs2)
{
    require_fp;

    float64_t f1, f2;
    f1.v = frs1;
    f2.v = frs2;
    frs1 = f64_le(f1, f2);

    set_fp3_exceptions();
    return frs1;
}

target_ulong helper_flt_d(CPUState *env, uint64_t frs1, uint64_t frs2)
{
    require_fp;

    float64_t f1, f2;
    f1.v = frs1;
    f2.v = frs2;
    frs1 = f64_lt(f1, f2);

    set_fp3_exceptions();
    return frs1;
}

target_ulong helper_fge_d(CPUState *env, uint64_t frs1, uint64_t frs2)
{
    return helper_fle_d(env, frs2, frs1);
}

target_ulong helper_fgt_d(CPUState *env, uint64_t frs1, uint64_t frs2)
{
    return helper_flt_d(env, frs2, frs1);
}

target_ulong helper_feq_d(CPUState *env, uint64_t frs1, uint64_t frs2)
{
    require_fp;

    float64_t f1, f2;
    f1.v = frs1;
    f2.v = frs2;
    frs1 = f64_eq(f1, f2);

    set_fp3_exceptions();
    return frs1;
}

target_ulong helper_fcvt_w_d(CPUState *env, uint64_t frs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float64_t f1;
    f1.v = frs1;
    frs1 = (int64_t)(f64_to_i32(f1, RM_3, true));

    set_fp3_exceptions();
    return frs1;
}

target_ulong helper_fcvt_wu_d(CPUState *env, uint64_t frs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float64_t f1;
    f1.v = frs1;
    frs1 = (int64_t)((int32_t)f64_to_ui32(f1, RM_3, true));

    set_fp3_exceptions();
    return frs1;
}

uint64_t helper_fcvt_l_d(CPUState *env, uint64_t frs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float64_t f1;
    f1.v = frs1;
    frs1 = f64_to_i64(f1, RM_3, true);

    set_fp3_exceptions();
    return frs1;
}

uint64_t helper_fcvt_lu_d(CPUState *env, uint64_t frs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float64_t f1;
    f1.v = frs1;
    frs1 = f64_to_ui64(f1, RM_3, true);

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fcvt_d_w(CPUState *env, target_ulong rs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    uint64_t res;
    res = i32_to_f64((int32_t)rs1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return res;
}

uint64_t helper_fcvt_d_wu(CPUState *env, target_ulong rs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    uint64_t res;
    res = ui32_to_f64((uint32_t)rs1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return res;
}

uint64_t helper_fcvt_d_l(CPUState *env, uint64_t rs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    rs1 = i64_to_f64(rs1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return rs1;
}

uint64_t helper_fcvt_d_lu(CPUState *env, uint64_t rs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    rs1 = ui64_to_f64(rs1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return rs1;
}

target_ulong helper_fcvt_wu_d_rod(CPUState *env, uint64_t frs1)
{
    require_fp;
    set_float3_rounding_mode(softfloat_round_odd);

    float64_t f1;
    f1.v = frs1;
    frs1 = (int64_t)((int32_t)f64_to_ui32(f1, softfloat_round_odd, true));

    set_fp3_exceptions();
    return frs1;
}

target_ulong helper_fcvt_w_d_rod(CPUState *env, uint64_t frs1)
{
    require_fp;
    set_float3_rounding_mode(softfloat_round_odd);

    float64_t f1;
    f1.v = frs1;
    frs1 = (int64_t)(f64_to_i32(f1, softfloat_round_odd, true));

    set_fp3_exceptions();
    return frs1;
}

uint64_t helper_fcvt_s_d_rod(CPUState *env, uint64_t rs1)
{
    require_fp;
    set_float3_rounding_mode(softfloat_round_odd);

    float64_t f1;
    f1.v = rs1;
    rs1 = f64_to_f32(f1).v;

    set_fp3_exceptions();
    return rs1;
}

uint64_t helper_fadd_h(CPUState *env, uint64_t frs1, uint64_t frs2, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float16_t f1, f2;
    f1.v = (uint16_t)frs1;
    f2.v = (uint16_t)frs2;
    frs1 = f16_add(f1, f2).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fsub_h(CPUState *env, uint64_t frs1, uint64_t frs2, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float16_t f1, f2;
    f1.v = (uint16_t)frs1;
    f2.v = (uint16_t)frs2;
    frs1 = f16_sub(f1, f2).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fmul_h(CPUState *env, uint64_t frs1, uint64_t frs2, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float16_t f1, f2;
    f1.v = (uint16_t)frs1;
    f2.v = (uint16_t)frs2;
    frs1 = f16_mul(f1, f2).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fdiv_h(CPUState *env, uint64_t frs1, uint64_t frs2, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float16_t f1, f2;
    f1.v = (uint16_t)frs1;
    f2.v = (uint16_t)frs2;
    frs1 = f16_div(f1, f2).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fmin_h(CPUState *env, uint64_t frs1, uint64_t frs2)
{
    require_fp;

    float16_t f1, f2;
    f1.v = (uint16_t)frs1;
    f2.v = (uint16_t)frs2;
    frs1 = f16_min(f1, f2).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fmax_h(CPUState *env, uint64_t frs1, uint64_t frs2)
{
    require_fp;

    float16_t f1, f2;
    f1.v = (uint16_t)frs1;
    f2.v = (uint16_t)frs2;
    frs1 = f16_max(f1, f2).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fcvt_s_h(CPUState *env, uint64_t rs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float16_t f1;
    f1.v = (uint16_t)rs1;
    rs1 = f16_to_f32(f1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return rs1;
}

uint64_t helper_fcvt_h_s(CPUState *env, uint64_t rs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float32_t f1;
    f1.v = (uint32_t)rs1;
    rs1 = f32_to_f16(f1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return rs1;
}

uint64_t helper_fcvt_d_h(CPUState *env, uint64_t rs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float16_t f1;
    f1.v = (uint16_t)rs1;
    rs1 = f16_to_f64(f1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return rs1;
}

uint64_t helper_fcvt_h_d(CPUState *env, uint64_t rs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float64_t f1;
    f1.v = rs1;
    rs1 = f64_to_f16(f1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return rs1;
}

uint64_t helper_fsqrt_h(CPUState *env, uint64_t frs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float16_t f1;
    f1.v = (uint16_t)frs1;
    frs1 = f16_sqrt(f1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

target_ulong helper_fle_h(CPUState *env, uint64_t frs1, uint64_t frs2)
{
    require_fp;

    float16_t f1, f2;
    f1.v = (uint16_t)frs1;
    f2.v = (uint16_t)frs2;
    frs1 = f16_le(f1, f2);

    set_fp3_exceptions();
    return frs1;
}

target_ulong helper_flt_h(CPUState *env, uint64_t frs1, uint64_t frs2)
{
    require_fp;

    float16_t f1, f2;
    f1.v = (uint16_t)frs1;
    f2.v = (uint16_t)frs2;
    frs1 = f16_lt(f1, f2);

    set_fp3_exceptions();
    return frs1;
}

target_ulong helper_fge_h(CPUState *env, uint64_t frs1, uint64_t frs2)
{
    return helper_fle_h(env, frs2, frs1);
}

target_ulong helper_fgt_h(CPUState *env, uint64_t frs1, uint64_t frs2)
{
    return helper_flt_h(env, frs2, frs1);
}

target_ulong helper_feq_h(CPUState *env, uint64_t frs1, uint64_t frs2)
{
    require_fp;

    float16_t f1, f2;
    f1.v = (uint16_t)frs1;
    f2.v = (uint16_t)frs2;
    frs1 = f16_eq(f1, f2);

    set_fp3_exceptions();
    return frs1;
}

target_ulong helper_fcvt_w_h(CPUState *env, uint64_t frs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float16_t f1;
    f1.v = (uint16_t)frs1;
    frs1 = (int64_t)((int32_t)f16_to_i32(f1, RM_3, true));

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

target_ulong helper_fcvt_wu_h(CPUState *env, uint64_t frs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float16_t f1;
    f1.v = (uint16_t)frs1;
    frs1 = (int64_t)((int32_t)f16_to_ui32(f1, RM_3, true));

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fcvt_l_h(CPUState *env, uint64_t frs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float16_t f1;
    f1.v = (uint16_t)frs1;
    frs1 = f16_to_i64(f1, RM_3, true);

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fcvt_lu_h(CPUState *env, uint64_t frs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    float16_t f1;
    f1.v = (uint16_t)frs1;
    frs1 = f16_to_ui64(f1, RM_3, true);

    set_fp3_exceptions();
    mark_fs_dirty();
    return frs1;
}

uint64_t helper_fcvt_h_w(CPUState *env, target_ulong rs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    uint64_t res;
    res = i32_to_f16((uint32_t)rs1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return res;
}

uint64_t helper_fcvt_h_wu(CPUState *env, target_ulong rs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    uint64_t res;
    res = ui32_to_f16((uint32_t)rs1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return res;
}

uint64_t helper_fcvt_h_l(CPUState *env, uint64_t rs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    rs1 = i64_to_f16(rs1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return rs1;
}

uint64_t helper_fcvt_h_lu(CPUState *env, uint64_t rs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    rs1 = ui64_to_f16(rs1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return rs1;
}

uint64_t helper_fmv_x_h(CPUState *env, uint64_t rs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    rs1 = (int64_t)((int32_t)((int16_t)rs1));

    set_fp3_exceptions();
    mark_fs_dirty();
    return rs1;
}

uint64_t helper_fmv_h_x(CPUState *env, uint64_t rs1, uint64_t rm)
{
    require_fp;
    set_float3_rounding_mode(RM_3);

    //  FMV.H.X moves the half-precision value encoded in IEEE 754-2008 standard encoding from the
    //  lower 16 bits of integer register rs1 to the floating-point register rd, NaN-boxing the result
    rs1 = box_float(RISCV_HALF_PRECISION, rs1);

    set_fp3_exceptions();
    mark_fs_dirty();
    return rs1;
}

target_ulong helper_fclass_h(CPUState *env, uint64_t frs1)
{
    require_fp;

    float16_t f1;
    f1.v = (uint16_t)frs1;
    frs1 = f16_classify(f1);

    mark_fs_dirty();
    return frs1;
}

void helper_vfmv_vf(CPUState *env, uint32_t vd, uint64_t f1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(env, eew);
    if(V_IDX_INVALID(vd)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    switch(eew) {
        case 16:
            f1 = unbox_float(RISCV_HALF_PRECISION, env, f1);
            break;
        case 32:
            f1 = unbox_float(RISCV_SINGLE_PRECISION, env, f1);
            break;
        default:
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
            return;
    }
    for(int ei = 0; ei < env->vl; ++ei) {
        switch(eew) {
            case 16:
                ((uint16_t *)V(vd))[ei] = f1;
                break;
            case 32:
                ((uint32_t *)V(vd))[ei] = f1;
                break;
            case 64:
                ((uint64_t *)V(vd))[ei] = f1;
                break;
        }
    }
}

void helper_vfmv_fs(CPUState *env, int32_t vd, int32_t vs2)
{
    const target_ulong eew = env->vsew;
    ensure_vector_float_embedded_extension_or_raise_exception(env, eew);
    switch(eew) {
        case 16:
            env->fpr[vd] = box_float(RISCV_HALF_PRECISION, ((uint16_t *)V(vs2))[0]);
            break;
        case 32:
            env->fpr[vd] = box_float(RISCV_SINGLE_PRECISION, ((uint32_t *)V(vs2))[0]);
            break;
        case 64:
            env->fpr[vd] = ((uint64_t *)V(vs2))[0];
            break;
        default:
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
            break;
    }
}

void helper_vfmv_sf(CPUState *env, uint32_t vd, float64 rs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_float_embedded_extension_or_raise_exception(env, eew);
    if(env->vstart >= env->vl) {
        return;
    }
    switch(eew) {
        case 16:
            ((int16_t *)V(vd))[0] = unbox_float(RISCV_HALF_PRECISION, env, rs1);
            break;
        case 32:
            ((int32_t *)V(vd))[0] = unbox_float(RISCV_SINGLE_PRECISION, env, rs1);
            break;
        case 64:
            ((int64_t *)V(vd))[0] = rs1;
            break;
        default:
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
            break;
    }
}

target_ulong helper_fclass_d(CPUState *env, uint64_t frs1)
{
    require_fp;

    float64_t f1;
    f1.v = frs1;
    frs1 = f64_classify(f1);

    mark_fs_dirty();
    return frs1;
}

uint16_t helper_f16_rsqrte7(CPUState *env, uint16_t in)
{
    require_fp;

    float16_t f1;
    f1.v = in;
    in = f16_rsqrte7(f1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return in;
}

uint32_t helper_f32_rsqrte7(CPUState *env, uint32_t in)
{
    require_fp;

    float32_t f1;
    f1.v = in;
    in = f32_rsqrte7(f1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return in;
}

uint64_t helper_f64_rsqrte7(CPUState *env, uint64_t in)
{
    require_fp;

    float64_t f1;
    f1.v = in;
    in = f64_rsqrte7(f1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return in;
}

uint16_t helper_f16_recip7(CPUState *env, uint16_t in)
{
    require_fp;

    float16_t f1;
    f1.v = in;
    in = f16_recip7(f1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return in;
}

uint32_t helper_f32_recip7(CPUState *env, uint32_t in)
{
    require_fp;

    float32_t f1;
    f1.v = in;
    in = f32_recip7(f1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return in;
}

uint64_t helper_f64_recip7(CPUState *env, uint64_t in)
{
    require_fp;

    float64_t f1;
    f1.v = in;
    in = f64_recip7(f1).v;

    set_fp3_exceptions();
    mark_fs_dirty();
    return in;
}

//  Note that MASKED is not defined for the 2nd include
#define MASKED
#include "fpu_vector_helper_template.h"
#include "fpu_vector_helper_template.h"
