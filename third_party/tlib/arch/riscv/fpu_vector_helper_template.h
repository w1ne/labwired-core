/*
 *  RRISCV vector extention helpers
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
#ifdef MASKED
#define POSTFIX _m
#else
#define POSTFIX
#endif

#ifdef MASKED
#define TEST_MASK(ei)                              \
    if(!(V(0)[(ei) >> 3] & (1 << ((ei) & 0x7)))) { \
        continue;                                  \
    }
#else
#define TEST_MASK(ei)
#endif

#define VFOP_VVX(NAME, HELPER)                                                                              \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, float64 imm)         \
    {                                                                                                       \
        const target_ulong eew = env->vsew;                                                                 \
        ensure_vector_float_embedded_extension_or_raise_exception(cpu, eew);                                \
        if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2)) {                                                       \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                      \
        }                                                                                                   \
        switch(eew) {                                                                                       \
            case 16:                                                                                        \
                imm = unbox_float(RISCV_HALF_PRECISION, env, imm);                                          \
                break;                                                                                      \
            case 32:                                                                                        \
                imm = unbox_float(RISCV_SINGLE_PRECISION, env, imm);                                        \
                break;                                                                                      \
        }                                                                                                   \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                     \
            TEST_MASK(ei)                                                                                   \
            switch(eew) {                                                                                   \
                case 16:                                                                                    \
                    ((float16 *)V(vd))[ei] = glue(HELPER, _h)(env, ((float16 *)V(vs2))[ei], imm, env->frm); \
                    break;                                                                                  \
                case 32:                                                                                    \
                    ((float32 *)V(vd))[ei] = glue(HELPER, _s)(env, ((float32 *)V(vs2))[ei], imm, env->frm); \
                    break;                                                                                  \
                case 64:                                                                                    \
                    ((float64 *)V(vd))[ei] = glue(HELPER, _d)(env, ((float64 *)V(vs2))[ei], imm, env->frm); \
                    break;                                                                                  \
                default:                                                                                    \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                              \
                    break;                                                                                  \
            }                                                                                               \
        }                                                                                                   \
    }

#define VFOP_VVV(NAME, HELPER)                                                                                                  \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1)                            \
    {                                                                                                                           \
        const target_ulong eew = env->vsew;                                                                                     \
        ensure_vector_float_embedded_extension_or_raise_exception(cpu, eew);                                                    \
        if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {                                                     \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                                          \
        }                                                                                                                       \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                                         \
            TEST_MASK(ei)                                                                                                       \
            switch(eew) {                                                                                                       \
                case 16:                                                                                                        \
                    ((float16 *)V(vd))[ei] = glue(HELPER, _h)(env, ((float16 *)V(vs2))[ei], ((float16 *)V(vs1))[ei], env->frm); \
                    break;                                                                                                      \
                case 32:                                                                                                        \
                    ((float32 *)V(vd))[ei] = glue(HELPER, _s)(env, ((float32 *)V(vs2))[ei], ((float32 *)V(vs1))[ei], env->frm); \
                    break;                                                                                                      \
                case 64:                                                                                                        \
                    ((float64 *)V(vd))[ei] = glue(HELPER, _d)(env, ((float64 *)V(vs2))[ei], ((float64 *)V(vs1))[ei], env->frm); \
                    break;                                                                                                      \
                default:                                                                                                        \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                                  \
                    break;                                                                                                      \
            }                                                                                                                   \
        }                                                                                                                       \
    }

#define VFOP_WVX(NAME, HELPER)                                                                                         \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, float64 imm)                    \
    {                                                                                                                  \
        const target_ulong eew = env->vsew;                                                                            \
        ensure_vector_float_embedded_extension_or_raise_exception(cpu, eew << 1);                                      \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID(vs2)) {                                                    \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                                 \
        }                                                                                                              \
        switch(eew) {                                                                                                  \
            case 32:                                                                                                   \
                imm = helper_fcvt_d_s(env, unbox_float(RISCV_SINGLE_PRECISION, env, imm), env->frm);                   \
                break;                                                                                                 \
        }                                                                                                              \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                                \
            TEST_MASK(ei)                                                                                              \
            switch(eew) {                                                                                              \
                case 32:                                                                                               \
                    ((float64 *)V(vd))[ei] =                                                                           \
                        glue(HELPER, _d)(env, helper_fcvt_d_s(env, ((float32 *)V(vs2))[ei], env->frm), imm, env->frm); \
                    break;                                                                                             \
                default:                                                                                               \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                         \
                    break;                                                                                             \
            }                                                                                                          \
        }                                                                                                              \
    }

#define VFOP_WVV(NAME, HELPER)                                                                                              \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1)                        \
    {                                                                                                                       \
        const target_ulong eew = env->vsew;                                                                                 \
        ensure_vector_float_embedded_extension_or_raise_exception(cpu, eew << 1);                                           \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {                                   \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                                      \
        }                                                                                                                   \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                                     \
            TEST_MASK(ei)                                                                                                   \
            switch(eew) {                                                                                                   \
                case 32:                                                                                                    \
                    ((float64 *)V(vd))[ei] = glue(HELPER, _d)(env, helper_fcvt_d_s(env, ((float32 *)V(vs2))[ei], env->frm), \
                                                              helper_fcvt_d_s(env, ((float32 *)V(vs1))[ei], env->frm),      \
                                                              env->frm);                                                    \
                    break;                                                                                                  \
                default:                                                                                                    \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                              \
                    break;                                                                                                  \
            }                                                                                                               \
        }                                                                                                                   \
    }

#define VFOP_WWX(NAME, HELPER)                                                                              \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, float64 imm)         \
    {                                                                                                       \
        const target_ulong eew = env->vsew;                                                                 \
        ensure_vector_float_embedded_extension_or_raise_exception(cpu, eew << 1);                           \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID_EEW(vs2, eew << 1)) {                           \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                      \
        }                                                                                                   \
        switch(eew) {                                                                                       \
            case 32:                                                                                        \
                imm = helper_fcvt_d_s(env, unbox_float(RISCV_SINGLE_PRECISION, env, imm), env->frm);        \
                break;                                                                                      \
        }                                                                                                   \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                     \
            TEST_MASK(ei)                                                                                   \
            switch(eew) {                                                                                   \
                case 32:                                                                                    \
                    ((float64 *)V(vd))[ei] = glue(HELPER, _d)(env, ((float64 *)V(vs2))[ei], imm, env->frm); \
                    break;                                                                                  \
                default:                                                                                    \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                              \
                    break;                                                                                  \
            }                                                                                               \
        }                                                                                                   \
    }

#define VFOP_WWV(NAME, HELPER)                                                                                            \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1)                      \
    {                                                                                                                     \
        const target_ulong eew = env->vsew;                                                                               \
        ensure_vector_float_embedded_extension_or_raise_exception(cpu, eew << 1);                                         \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID_EEW(vs2, eew << 1) || V_IDX_INVALID(vs1)) {                   \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                                    \
        }                                                                                                                 \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                                   \
            TEST_MASK(ei)                                                                                                 \
            switch(eew) {                                                                                                 \
                case 32:                                                                                                  \
                    ((float64 *)V(vd))[ei] = glue(HELPER, _d)(                                                            \
                        env, ((float64 *)V(vs2))[ei], helper_fcvt_d_s(env, ((float32 *)V(vs1))[ei], env->frm), env->frm); \
                    break;                                                                                                \
                default:                                                                                                  \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                            \
                    break;                                                                                                \
            }                                                                                                             \
        }                                                                                                                 \
    }

#define VF3OP_VVX(NAME, HELPER)                                                                                \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, float64 imm)            \
    {                                                                                                          \
        const target_ulong eew = env->vsew;                                                                    \
        ensure_vector_float_embedded_extension_or_raise_exception(cpu, eew);                                   \
        if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2)) {                                                          \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                         \
        }                                                                                                      \
        switch(eew) {                                                                                          \
            case 16:                                                                                           \
                imm = unbox_float(RISCV_HALF_PRECISION, env, imm);                                             \
                break;                                                                                         \
            case 32:                                                                                           \
                imm = unbox_float(RISCV_SINGLE_PRECISION, env, imm);                                           \
                break;                                                                                         \
        }                                                                                                      \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                        \
            TEST_MASK(ei)                                                                                      \
            switch(eew) {                                                                                      \
                case 16:                                                                                       \
                    ((float16 *)V(vd))[ei] =                                                                   \
                        glue(HELPER, _h)(env, ((float16 *)V(vd))[ei], ((float16 *)V(vs2))[ei], imm, env->frm); \
                    break;                                                                                     \
                case 32:                                                                                       \
                    ((float32 *)V(vd))[ei] =                                                                   \
                        glue(HELPER, _s)(env, ((float32 *)V(vd))[ei], ((float32 *)V(vs2))[ei], imm, env->frm); \
                    break;                                                                                     \
                case 64:                                                                                       \
                    ((float64 *)V(vd))[ei] =                                                                   \
                        glue(HELPER, _d)(env, ((float64 *)V(vd))[ei], ((float64 *)V(vs2))[ei], imm, env->frm); \
                    break;                                                                                     \
                default:                                                                                       \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                 \
                    break;                                                                                     \
            }                                                                                                  \
        }                                                                                                      \
    }

#define VF3OP_VVV(NAME, HELPER)                                                                                     \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1)                \
    {                                                                                                               \
        const target_ulong eew = env->vsew;                                                                         \
        ensure_vector_float_embedded_extension_or_raise_exception(cpu, eew);                                        \
        if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {                                         \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                              \
        }                                                                                                           \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                             \
            TEST_MASK(ei)                                                                                           \
            switch(eew) {                                                                                           \
                case 16:                                                                                            \
                    ((float16 *)V(vd))[ei] = glue(HELPER, _h)(env, ((float16 *)V(vd))[ei], ((float16 *)V(vs2))[ei], \
                                                              ((float16 *)V(vs1))[ei], env->frm);                   \
                    break;                                                                                          \
                case 32:                                                                                            \
                    ((float32 *)V(vd))[ei] = glue(HELPER, _s)(env, ((float32 *)V(vd))[ei], ((float32 *)V(vs2))[ei], \
                                                              ((float32 *)V(vs1))[ei], env->frm);                   \
                    break;                                                                                          \
                case 64:                                                                                            \
                    ((float64 *)V(vd))[ei] = glue(HELPER, _d)(env, ((float64 *)V(vd))[ei], ((float64 *)V(vs2))[ei], \
                                                              ((float64 *)V(vs1))[ei], env->frm);                   \
                    break;                                                                                          \
                default:                                                                                            \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                      \
                    break;                                                                                          \
            }                                                                                                       \
        }                                                                                                           \
    }

#define VF3OP_WVX(NAME, HELPER)                                                                                               \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, float64 imm)                           \
    {                                                                                                                         \
        const target_ulong eew = env->vsew;                                                                                   \
        ensure_vector_float_embedded_extension_or_raise_exception(cpu, eew << 1);                                             \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID(vs2)) {                                                           \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                                        \
        }                                                                                                                     \
        switch(eew) {                                                                                                         \
            case 32:                                                                                                          \
                imm = helper_fcvt_d_s(env, unbox_float(RISCV_SINGLE_PRECISION, env, imm), env->frm);                          \
                break;                                                                                                        \
        }                                                                                                                     \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                                       \
            TEST_MASK(ei)                                                                                                     \
            switch(eew) {                                                                                                     \
                case 32:                                                                                                      \
                    ((float64 *)V(vd))[ei] = glue(HELPER, _d)(                                                                \
                        env, ((float64 *)V(vd))[ei], helper_fcvt_d_s(env, ((float32 *)V(vs2))[ei], env->frm), imm, env->frm); \
                    break;                                                                                                    \
                default:                                                                                                      \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                                \
                    break;                                                                                                    \
            }                                                                                                                 \
        }                                                                                                                     \
    }

#define VF3OP_WVV(NAME, HELPER)                                                                                                \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1)                           \
    {                                                                                                                          \
        const target_ulong eew = env->vsew;                                                                                    \
        ensure_vector_float_embedded_extension_or_raise_exception(cpu, eew << 1);                                              \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {                                      \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                                         \
        }                                                                                                                      \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                                        \
            TEST_MASK(ei)                                                                                                      \
            switch(eew) {                                                                                                      \
                case 32:                                                                                                       \
                    ((float64 *)V(vd))[ei] =                                                                                   \
                        glue(HELPER, _d)(env, ((float64 *)V(vd))[ei], helper_fcvt_d_s(env, ((float32 *)V(vs2))[ei], env->frm), \
                                         helper_fcvt_d_s(env, ((float32 *)V(vs1))[ei], env->frm), env->frm);                   \
                    break;                                                                                                     \
                default:                                                                                                       \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                                 \
                    break;                                                                                                     \
            }                                                                                                                  \
        }                                                                                                                      \
    }

#define MS_VL_MASK ((env->vl - ei > 0x7) ? 0 : (0xff << (env->vl & 0x7)))
#ifdef MASKED
#define MS_MASK()      (MS_VL_MASK | ~V(0)[(ei >> 3)])
#define MS_TEST_MASK() (~mask & (1 << (ei & 0x7)))
#else
#define MS_MASK()      MS_VL_MASK
#define MS_TEST_MASK() (true)
#endif

#define VFMOP_LOOP_PREFIX() \
    if(!(ei & 0x7)) {       \
        mask = MS_MASK();   \
    }
#define VFMOP_LOOP_SUFFIX()                        \
    if(!((ei + 1) & 0x7) || (ei + 1) >= env->vl) { \
        V(vd)[(ei >> 3)] &= mask;                  \
        V(vd)[(ei >> 3)] |= value;                 \
        value = 0;                                 \
    }

#define VFMOP_VVX(NAME, HELPER)                                                                     \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, float64 imm) \
    {                                                                                               \
        const target_ulong eew = env->vsew;                                                         \
        ensure_vector_float_embedded_extension_or_raise_exception(cpu, eew);                        \
        if(V_IDX_INVALID(vs2)) {                                                                    \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                              \
        }                                                                                           \
        switch(eew) {                                                                               \
            case 16:                                                                                \
                imm = unbox_float(RISCV_HALF_PRECISION, env, imm);                                  \
                break;                                                                              \
            case 32:                                                                                \
                imm = unbox_float(RISCV_SINGLE_PRECISION, env, imm);                                \
                break;                                                                              \
        }                                                                                           \
        uint8_t mask = 0, value = 0;                                                                \
        for(int ei = 0; ei < env->vl; ++ei) {                                                       \
            VFMOP_LOOP_PREFIX();                                                                    \
            if(MS_TEST_MASK()) {                                                                    \
                switch(eew) {                                                                       \
                    case 16:                                                                        \
                        value |= glue(HELPER, _h)(env, ((float16 *)V(vs2))[ei], imm) << (ei & 0x7); \
                        break;                                                                      \
                    case 32:                                                                        \
                        value |= glue(HELPER, _s)(env, ((float32 *)V(vs2))[ei], imm) << (ei & 0x7); \
                        break;                                                                      \
                    case 64:                                                                        \
                        value |= glue(HELPER, _d)(env, ((float64 *)V(vs2))[ei], imm) << (ei & 0x7); \
                        break;                                                                      \
                    default:                                                                        \
                        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                  \
                        break;                                                                      \
                }                                                                                   \
            }                                                                                       \
            VFMOP_LOOP_SUFFIX();                                                                    \
        }                                                                                           \
    }

#define VFMOP_VVV(NAME, HELPER)                                                                                         \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1)                    \
    {                                                                                                                   \
        const target_ulong eew = env->vsew;                                                                             \
        ensure_vector_float_embedded_extension_or_raise_exception(cpu, eew);                                            \
        if(V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {                                                                  \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                                  \
        }                                                                                                               \
        uint8_t mask = 0, value = 0;                                                                                    \
        for(int ei = 0; ei < env->vl; ++ei) {                                                                           \
            VFMOP_LOOP_PREFIX();                                                                                        \
            if(MS_TEST_MASK()) {                                                                                        \
                switch(eew) {                                                                                           \
                    case 16:                                                                                            \
                        value |= glue(HELPER, _h)(env, ((float16 *)V(vs2))[ei], ((float16 *)V(vs1))[ei]) << (ei & 0x7); \
                        break;                                                                                          \
                    case 32:                                                                                            \
                        value |= glue(HELPER, _s)(env, ((float32 *)V(vs2))[ei], ((float32 *)V(vs1))[ei]) << (ei & 0x7); \
                        break;                                                                                          \
                    case 64:                                                                                            \
                        value |= glue(HELPER, _d)(env, ((float64 *)V(vs2))[ei], ((float64 *)V(vs1))[ei]) << (ei & 0x7); \
                        break;                                                                                          \
                    default:                                                                                            \
                        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                      \
                        break;                                                                                          \
                }                                                                                                       \
            }                                                                                                           \
            VFMOP_LOOP_SUFFIX();                                                                                        \
        }                                                                                                               \
    }

#define VFOP_VV(NAME, HELPER)                                                                \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2)       \
    {                                                                                        \
        require_fp;                                                                          \
        const target_ulong eew = env->vsew;                                                  \
        ensure_vector_float_embedded_extension_or_raise_exception(cpu, eew);                 \
        if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2)) {                                        \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                       \
        }                                                                                    \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                      \
            TEST_MASK(ei)                                                                    \
            switch(eew) {                                                                    \
                case 16:                                                                     \
                    ((float16 *)V(vd))[ei] = glue(HELPER, _h)(env, ((float16 *)V(vs2))[ei]); \
                    break;                                                                   \
                case 32:                                                                     \
                    ((float32 *)V(vd))[ei] = glue(HELPER, _s)(env, ((float32 *)V(vs2))[ei]); \
                    break;                                                                   \
                case 64:                                                                     \
                    ((float64 *)V(vd))[ei] = glue(HELPER, _d)(env, ((float64 *)V(vs2))[ei]); \
                    break;                                                                   \
                default:                                                                     \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);               \
                    break;                                                                   \
            }                                                                                \
        }                                                                                    \
    }

#define VFOP_WV_FF(NAME, HELPER)                                                             \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2)       \
    {                                                                                        \
        const target_ulong eew = env->vsew;                                                  \
        ensure_vector_float_embedded_extension_or_raise_exception(cpu, eew << 1);            \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID(vs2)) {                          \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                       \
        }                                                                                    \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                      \
            TEST_MASK(ei)                                                                    \
            switch(eew) {                                                                    \
                case 32:                                                                     \
                    ((float64 *)V(vd))[ei] = glue(HELPER, _d)(env, ((float32 *)V(vs2))[ei]); \
                    break;                                                                   \
                default:                                                                     \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);               \
                    break;                                                                   \
            }                                                                                \
        }                                                                                    \
    }

#define VFOP_WV_FX(NAME, HELPER)                                                              \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2)        \
    {                                                                                         \
        const target_ulong eew = env->vsew;                                                   \
        ensure_vector_float_embedded_extension_or_raise_exception(cpu, eew << 1);             \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID(vs2)) {                           \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                        \
        }                                                                                     \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                       \
            TEST_MASK(ei)                                                                     \
            switch(eew) {                                                                     \
                case 16:                                                                      \
                    ((float32 *)V(vd))[ei] = glue(HELPER, _s)(env, ((uint16_t *)V(vs2))[ei]); \
                    break;                                                                    \
                case 32:                                                                      \
                    ((float64 *)V(vd))[ei] = glue(HELPER, _d)(env, ((uint32_t *)V(vs2))[ei]); \
                    break;                                                                    \
                default:                                                                      \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                \
                    break;                                                                    \
            }                                                                                 \
        }                                                                                     \
    }

#define VFOP_WV_XF(NAME, HELPER)                                                              \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2)        \
    {                                                                                         \
        const target_ulong eew = env->vsew;                                                   \
        ensure_vector_float_embedded_extension_or_raise_exception(cpu, eew);                  \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID(vs2)) {                           \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                        \
        }                                                                                     \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                       \
            TEST_MASK(ei)                                                                     \
            switch(eew) {                                                                     \
                case 32:                                                                      \
                    ((uint64_t *)V(vd))[ei] = glue(HELPER, _d)(env, ((float32 *)V(vs2))[ei]); \
                    break;                                                                    \
                default:                                                                      \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                \
                    break;                                                                    \
            }                                                                                 \
        }                                                                                     \
    }

#define VFOP_VW_FF(NAME, HELPER)                                                             \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2)       \
    {                                                                                        \
        const target_ulong eew = env->vsew;                                                  \
        ensure_vector_float_embedded_extension_or_raise_exception(cpu, eew << 1);            \
        if(V_IDX_INVALID(vd) || V_IDX_INVALID_EEW(vs2, eew << 1)) {                          \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                       \
        }                                                                                    \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                      \
            TEST_MASK(ei)                                                                    \
            switch(eew) {                                                                    \
                case 32:                                                                     \
                    ((float32 *)V(vd))[ei] = glue(HELPER, _s)(env, ((float64 *)V(vs2))[ei]); \
                    break;                                                                   \
                default:                                                                     \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);               \
                    break;                                                                   \
            }                                                                                \
        }                                                                                    \
    }

#define VFOP_VW_FX(NAME, HELPER)                                                              \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2)        \
    {                                                                                         \
        const target_ulong eew = env->vsew;                                                   \
        ensure_vector_float_embedded_extension_or_raise_exception(cpu, eew);                  \
        if(V_IDX_INVALID(vd) || V_IDX_INVALID_EEW(vs2, eew << 1)) {                           \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                        \
        }                                                                                     \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                       \
            TEST_MASK(ei)                                                                     \
            switch(eew) {                                                                     \
                case 32:                                                                      \
                    ((float32 *)V(vd))[ei] = glue(HELPER, _s)(env, ((uint64_t *)V(vs2))[ei]); \
                    break;                                                                    \
                default:                                                                      \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                \
                    break;                                                                    \
            }                                                                                 \
        }                                                                                     \
    }

#define VFOP_VW_XF(NAME, HELPER)                                                              \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2)        \
    {                                                                                         \
        const target_ulong eew = env->vsew;                                                   \
        ensure_vector_float_embedded_extension_or_raise_exception(cpu, eew << 1);             \
        if(V_IDX_INVALID(vd) || V_IDX_INVALID_EEW(vs2, eew << 1)) {                           \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                        \
        }                                                                                     \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                       \
            TEST_MASK(ei)                                                                     \
            switch(eew) {                                                                     \
                case 16:                                                                      \
                    ((uint16_t *)V(vd))[ei] = glue(HELPER, _s)(env, ((float32 *)V(vs2))[ei]); \
                    break;                                                                    \
                case 32:                                                                      \
                    ((uint32_t *)V(vd))[ei] = glue(HELPER, _d)(env, ((float64 *)V(vs2))[ei]); \
                    break;                                                                    \
                default:                                                                      \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                \
                    break;                                                                    \
            }                                                                                 \
        }                                                                                     \
    }

#define VFOP_RED_VVV(NAME, HELPER)                                                                   \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1) \
    {                                                                                                \
        const target_ulong eew = env->vsew;                                                          \
        ensure_vector_float_embedded_extension_or_raise_exception(cpu, eew);                         \
        if(env->vl == 0)                                                                             \
            return;                                                                                  \
        if(V_IDX_INVALID(vs2) || env->vstart != 0) {                                                 \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                               \
        }                                                                                            \
        float64 acc;                                                                                 \
        switch(eew) {                                                                                \
            case 32:                                                                                 \
                acc = ((float32 *)V(vs1))[0];                                                        \
                break;                                                                               \
            case 64:                                                                                 \
                acc = ((float64 *)V(vs1))[0];                                                        \
                break;                                                                               \
            default:                                                                                 \
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                           \
                break;                                                                               \
        }                                                                                            \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                              \
            TEST_MASK(ei)                                                                            \
            switch(eew) {                                                                            \
                case 32:                                                                             \
                    acc = glue(HELPER, _s)(env, acc, ((float32 *)V(vs2))[ei]);                       \
                    break;                                                                           \
                case 64:                                                                             \
                    acc = glue(HELPER, _d)(env, acc, ((float64 *)V(vs2))[ei]);                       \
                    break;                                                                           \
            }                                                                                        \
        }                                                                                            \
        MAYBE_CANONICALIZE_NAN                                                                       \
        switch(eew) {                                                                                \
            case 32:                                                                                 \
                ((float32 *)V(vd))[0] = acc;                                                         \
                break;                                                                               \
            case 64:                                                                                 \
                ((float64 *)V(vd))[0] = acc;                                                         \
                break;                                                                               \
        }                                                                                            \
    }

#define VFOP_RED_WVV(NAME, HELPER)                                                                             \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1)           \
    {                                                                                                          \
        const target_ulong eew = env->vsew;                                                                    \
        ensure_vector_float_embedded_extension_or_raise_exception(cpu, eew);                                   \
        if(env->vl == 0)                                                                                       \
            return;                                                                                            \
        if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1) || env->vstart != 0) {                \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                         \
        }                                                                                                      \
        float64 acc;                                                                                           \
        switch(eew) {                                                                                          \
            case 32:                                                                                           \
                acc = ((float64 *)V(vs1))[0];                                                                  \
                break;                                                                                         \
            default:                                                                                           \
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                     \
                break;                                                                                         \
        }                                                                                                      \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                        \
            TEST_MASK(ei)                                                                                      \
            switch(eew) {                                                                                      \
                case 32:                                                                                       \
                    acc = glue(HELPER, _d)(env, acc, helper_fcvt_d_s(env, ((float32 *)V(vs2))[ei], env->frm)); \
                    break;                                                                                     \
            }                                                                                                  \
        }                                                                                                      \
        MAYBE_CANONICALIZE_NAN                                                                                 \
        switch(eew) {                                                                                          \
            case 32:                                                                                           \
                ((float64 *)V(vd))[0] = acc;                                                                   \
                break;                                                                                         \
        }                                                                                                      \
    }

void glue(helper_vfslide1up, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, float64 imm)
{
    const target_ulong eew = env->vsew;

    ensure_vector_float_embedded_extension_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }

    if(env->vl == 0) {
        return;
    }

    if(env->vstart == 0
#ifdef MASKED
       && (V(0)[0] & 0x1)
#endif
    ) {
        switch(eew) {
            case 16:
                ((float16 *)V(vd))[0] = unbox_float(RISCV_HALF_PRECISION, env, imm);
                break;
            case 32:
                ((float32 *)V(vd))[0] = unbox_float(RISCV_SINGLE_PRECISION, env, imm);
                break;
            case 64:
                ((float64 *)V(vd))[0] = imm;
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
    for(int ei = (env->vstart ? env->vstart : 1); ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 16:
                ((float16 *)V(vd))[ei] = ((float16 *)V(vs2))[ei - 1];
                break;
            case 32:
                ((float32 *)V(vd))[ei] = ((float32 *)V(vs2))[ei - 1];
                break;
            case 64:
                ((float64 *)V(vd))[ei] = ((float64 *)V(vs2))[ei - 1];
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vfslide1down, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, float64 imm)
{
    const target_ulong eew = env->vsew;
    ensure_vector_float_embedded_extension_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }

    const int src_max = env->vl - 1;
    if(env->vl == 0) {
        return;
    }

    for(int ei = env->vstart; ei < src_max; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 16:
                ((float16 *)V(vd))[ei] = ((float16 *)V(vs2))[ei + 1];
                break;
            case 32:
                ((float32 *)V(vd))[ei] = ((float32 *)V(vs2))[ei + 1];
                break;
            case 64:
                ((float64 *)V(vd))[ei] = ((float64 *)V(vs2))[ei + 1];
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
#ifdef MASKED
    if(V(0)[src_max >> 3] & (1 << (src_max & 0x7))) {
#endif
        switch(eew) {
            case 16:
                ((float16 *)V(vd))[src_max] = unbox_float(RISCV_HALF_PRECISION, env, imm);
                break;
            case 32:
                ((float32 *)V(vd))[src_max] = unbox_float(RISCV_SINGLE_PRECISION, env, imm);
                break;
            case 64:
                ((float64 *)V(vd))[src_max] = imm;
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
#ifdef MASKED
    }
#endif
}

#define MAYBE_CANONICALIZE_NAN

VFOP_VVV(vfadd_vv, helper_fadd)
VFOP_VVX(vfadd_vf, helper_fadd)
VFOP_VVV(vfsub_vv, helper_fsub)
VFOP_VVX(vfsub_vf, helper_fsub)

#define VFOP_RSUB_h(ENV, A, B, FRM) helper_fsub_h(ENV, B, A, FRM)
#define VFOP_RSUB_s(ENV, A, B, FRM) helper_fsub_s(ENV, B, A, FRM)
#define VFOP_RSUB_d(ENV, A, B, FRM) helper_fsub_d(ENV, B, A, FRM)
VFOP_VVX(vfrsub_vf, VFOP_RSUB)

VFOP_WVX(vfwadd_vf, helper_fadd)
VFOP_WVV(vfwadd_vv, helper_fadd)

VFOP_WWX(vfwadd_wf, helper_fadd)
VFOP_WWV(vfwadd_wv, helper_fadd)

VFOP_WVX(vfwsub_vf, helper_fsub)
VFOP_WVV(vfwsub_vv, helper_fsub)

VFOP_WWX(vfwsub_wf, helper_fsub)
VFOP_WWV(vfwsub_wv, helper_fsub)

VFOP_VVV(vfmul_vv, helper_fmul)
VFOP_VVX(vfmul_vf, helper_fmul)

VFOP_VVV(vfdiv_vv, helper_fdiv)
VFOP_VVX(vfdiv_vf, helper_fdiv)

#define VFOP_FRDIV_h(ENV, A, B, FRM) helper_fdiv_h(ENV, B, A, FRM)
#define VFOP_FRDIV_s(ENV, A, B, FRM) helper_fdiv_s(ENV, B, A, FRM)
#define VFOP_FRDIV_d(ENV, A, B, FRM) helper_fdiv_d(ENV, B, A, FRM)
VFOP_VVX(vfrdiv_vf, VFOP_FRDIV)

VFOP_WVX(vfwmul_vf, helper_fmul)
VFOP_WVV(vfwmul_vv, helper_fmul)

#define VFOP_MACC_h(ENV, DEST, OP1, OP2, FRM) helper_fmadd_h(ENV, OP1, OP2, DEST, FRM)
#define VFOP_MACC_s(ENV, DEST, OP1, OP2, FRM) helper_fmadd_s(ENV, OP1, OP2, DEST, FRM)
#define VFOP_MACC_d(ENV, DEST, OP1, OP2, FRM) helper_fmadd_d(ENV, OP1, OP2, DEST, FRM)
VF3OP_VVX(vfmacc_vf, VFOP_MACC)
VF3OP_VVV(vfmacc_vv, VFOP_MACC)

#define VFOP_NMACC_h(ENV, DEST, OP1, OP2, FRM) helper_fnmadd_h(ENV, OP1, OP2, DEST, FRM)
#define VFOP_NMACC_s(ENV, DEST, OP1, OP2, FRM) helper_fnmadd_s(ENV, OP1, OP2, DEST, FRM)
#define VFOP_NMACC_d(ENV, DEST, OP1, OP2, FRM) helper_fnmadd_d(ENV, OP1, OP2, DEST, FRM)
VF3OP_VVX(vfnmacc_vf, VFOP_NMACC)
VF3OP_VVV(vfnmacc_vv, VFOP_NMACC)

#define VFOP_MSAC_h(ENV, DEST, OP1, OP2, FRM) helper_fmsub_h(ENV, OP1, OP2, DEST, FRM)
#define VFOP_MSAC_s(ENV, DEST, OP1, OP2, FRM) helper_fmsub_s(ENV, OP1, OP2, DEST, FRM)
#define VFOP_MSAC_d(ENV, DEST, OP1, OP2, FRM) helper_fmsub_d(ENV, OP1, OP2, DEST, FRM)
VF3OP_VVX(vfmsac_vf, VFOP_MSAC)
VF3OP_VVV(vfmsac_vv, VFOP_MSAC)

#define VFOP_NMSAC_h(ENV, DEST, OP1, OP2, FRM) helper_fnmsub_h(ENV, OP1, OP2, DEST, FRM)
#define VFOP_NMSAC_s(ENV, DEST, OP1, OP2, FRM) helper_fnmsub_s(ENV, OP1, OP2, DEST, FRM)
#define VFOP_NMSAC_d(ENV, DEST, OP1, OP2, FRM) helper_fnmsub_d(ENV, OP1, OP2, DEST, FRM)
VF3OP_VVX(vfnmsac_vf, VFOP_NMSAC)
VF3OP_VVV(vfnmsac_vv, VFOP_NMSAC)

#define VFOP_MADD_h(ENV, DEST, OP1, OP2, FRM) helper_fmadd_h(ENV, OP2, DEST, OP1, FRM)
#define VFOP_MADD_s(ENV, DEST, OP1, OP2, FRM) helper_fmadd_s(ENV, OP2, DEST, OP1, FRM)
#define VFOP_MADD_d(ENV, DEST, OP1, OP2, FRM) helper_fmadd_d(ENV, OP2, DEST, OP1, FRM)
VF3OP_VVX(vfmadd_vf, VFOP_MADD)
VF3OP_VVV(vfmadd_vv, VFOP_MADD)

#define VFOP_NMADD_h(ENV, DEST, OP1, OP2, FRM) helper_fnmadd_h(ENV, OP2, DEST, OP1, FRM)
#define VFOP_NMADD_s(ENV, DEST, OP1, OP2, FRM) helper_fnmadd_s(ENV, OP2, DEST, OP1, FRM)
#define VFOP_NMADD_d(ENV, DEST, OP1, OP2, FRM) helper_fnmadd_d(ENV, OP2, DEST, OP1, FRM)
VF3OP_VVX(vfnmadd_vf, VFOP_NMADD)
VF3OP_VVV(vfnmadd_vv, VFOP_NMADD)

#define VFOP_MSUB_h(ENV, DEST, OP1, OP2, FRM) helper_fmsub_h(ENV, OP2, DEST, OP1, FRM)
#define VFOP_MSUB_s(ENV, DEST, OP1, OP2, FRM) helper_fmsub_s(ENV, OP2, DEST, OP1, FRM)
#define VFOP_MSUB_d(ENV, DEST, OP1, OP2, FRM) helper_fmsub_d(ENV, OP2, DEST, OP1, FRM)
VF3OP_VVX(vfmsub_vf, VFOP_MSUB)
VF3OP_VVV(vfmsub_vv, VFOP_MSUB)

#define VFOP_NMSUB_h(ENV, DEST, OP1, OP2, FRM) helper_fnmsub_h(ENV, OP2, DEST, OP1, FRM)
#define VFOP_NMSUB_s(ENV, DEST, OP1, OP2, FRM) helper_fnmsub_s(ENV, OP2, DEST, OP1, FRM)
#define VFOP_NMSUB_d(ENV, DEST, OP1, OP2, FRM) helper_fnmsub_d(ENV, OP2, DEST, OP1, FRM)
VF3OP_VVX(vfnmsub_vf, VFOP_NMSUB)
VF3OP_VVV(vfnmsub_vv, VFOP_NMSUB)

VF3OP_WVX(vfwmacc_vf, VFOP_MACC)
VF3OP_WVV(vfwmacc_vv, VFOP_MACC)

VF3OP_WVX(vfwnmacc_vf, VFOP_NMACC)
VF3OP_WVV(vfwnmacc_vv, VFOP_NMACC)

VF3OP_WVX(vfwmsac_vf, VFOP_MSAC)
VF3OP_WVV(vfwmsac_vv, VFOP_MSAC)

VF3OP_WVX(vfwnmsac_vf, VFOP_NMSAC)
VF3OP_WVV(vfwnmsac_vv, VFOP_NMSAC)

#define VFOP_FMIN_h(ENV, A, B, FRM) helper_fmin_h(ENV, A, B)
#define VFOP_FMIN_s(ENV, A, B, FRM) helper_fmin_s(ENV, A, B)
#define VFOP_FMIN_d(ENV, A, B, FRM) helper_fmin_d(ENV, A, B)
VFOP_VVV(vfmin_vv, VFOP_FMIN)
VFOP_VVX(vfmin_vf, VFOP_FMIN)

#define VFOP_FMAX_h(ENV, A, B, FRM) helper_fmax_h(ENV, A, B)
#define VFOP_FMAX_s(ENV, A, B, FRM) helper_fmax_s(ENV, A, B)
#define VFOP_FMAX_d(ENV, A, B, FRM) helper_fmax_d(ENV, A, B)
VFOP_VVV(vfmax_vv, VFOP_FMAX)
VFOP_VVX(vfmax_vf, VFOP_FMAX)

#define VFOP_FSGNJ_h(ENV, A, B, FRM) ((A & ~INT16_MIN) | (B & INT16_MIN))
#define VFOP_FSGNJ_s(ENV, A, B, FRM) ((A & ~INT32_MIN) | (B & INT32_MIN))
#define VFOP_FSGNJ_d(ENV, A, B, FRM) ((A & ~INT64_MIN) | (B & INT64_MIN))
VFOP_VVV(vfsgnj_vv, VFOP_FSGNJ)
VFOP_VVX(vfsgnj_vf, VFOP_FSGNJ)

#define VFOP_FSGNJN_h(ENV, A, B, FRM) ((A & ~INT16_MIN) | (~B & INT16_MIN))
#define VFOP_FSGNJN_s(ENV, A, B, FRM) ((A & ~INT32_MIN) | (~B & INT32_MIN))
#define VFOP_FSGNJN_d(ENV, A, B, FRM) ((A & ~INT64_MIN) | (~B & INT64_MIN))
VFOP_VVV(vfsgnjn_vv, VFOP_FSGNJN)
VFOP_VVX(vfsgnjn_vf, VFOP_FSGNJN)

#define VFOP_FSGNJX_h(ENV, A, B, FRM) (A ^ (B & INT16_MIN))
#define VFOP_FSGNJX_s(ENV, A, B, FRM) (A ^ (B & INT32_MIN))
#define VFOP_FSGNJX_d(ENV, A, B, FRM) (A ^ (B & INT64_MIN))
VFOP_VVV(vfsgnjx_vv, VFOP_FSGNJX)
VFOP_VVX(vfsgnjx_vf, VFOP_FSGNJX)

VFMOP_VVV(vfeq_vv, helper_feq)
VFMOP_VVX(vfeq_vf, helper_feq)

VFMOP_VVV(vfne_vv, !helper_feq)
VFMOP_VVX(vfne_vf, !helper_feq)

VFMOP_VVV(vflt_vv, helper_flt)
VFMOP_VVX(vflt_vf, helper_flt)

VFMOP_VVV(vfle_vv, helper_fle)
VFMOP_VVX(vfle_vf, helper_fle)

VFMOP_VVX(vfgt_vf, helper_fgt)
VFMOP_VVX(vfge_vf, helper_fge)

#define VFOP_RSQRT7_h(ENV, OP1) helper_f16_rsqrte7(ENV, OP1)
#define VFOP_RSQRT7_s(ENV, OP1) helper_f32_rsqrte7(ENV, OP1)
#define VFOP_RSQRT7_d(ENV, OP1) helper_f64_rsqrte7(ENV, OP1)
VFOP_VV(vfrsqrt7_v, VFOP_RSQRT7)

#define VFOP_REC7_h(ENV, OP1) helper_f16_recip7(ENV, OP1)
#define VFOP_REC7_s(ENV, OP1) helper_f32_recip7(ENV, OP1)
#define VFOP_REC7_d(ENV, OP1) helper_f64_recip7(ENV, OP1)
VFOP_VV(vfrec7_v, VFOP_REC7)

#define VFOP_SQRT_h(ENV, OP1) helper_fsqrt_h(ENV, OP1, env->frm)
#define VFOP_SQRT_s(ENV, OP1) helper_fsqrt_s(ENV, OP1, env->frm)
#define VFOP_SQRT_d(ENV, OP1) helper_fsqrt_d(ENV, OP1, env->frm)
VFOP_VV(vfsqrt_v, VFOP_SQRT)

VFOP_VV(vfclass_v, helper_fclass)

#define VFOP_FCVT_XUF_h(ENV, OP1) helper_fcvt_wu_h(ENV, OP1, ENV->frm)
#define VFOP_FCVT_XUF_s(ENV, OP1) helper_fcvt_wu_s(ENV, OP1, ENV->frm)
#define VFOP_FCVT_XUF_d(ENV, OP1) helper_fcvt_lu_d(ENV, OP1, ENV->frm)
VFOP_VV(vfcvt_xuf_v, VFOP_FCVT_XUF)

#define VFOP_FCVT_XF_h(ENV, OP1) helper_fcvt_w_h(ENV, OP1, ENV->frm)
#define VFOP_FCVT_XF_s(ENV, OP1) helper_fcvt_w_s(ENV, OP1, ENV->frm)
#define VFOP_FCVT_XF_d(ENV, OP1) helper_fcvt_l_d(ENV, OP1, ENV->frm)
VFOP_VV(vfcvt_xf_v, VFOP_FCVT_XF)

#define VFOP_FCVT_RTZ_XUF_h(ENV, OP1) helper_fcvt_wu_h(ENV, OP1, riscv_float_round_to_zero)
#define VFOP_FCVT_RTZ_XUF_s(ENV, OP1) helper_fcvt_wu_s(ENV, OP1, riscv_float_round_to_zero)
#define VFOP_FCVT_RTZ_XUF_d(ENV, OP1) helper_fcvt_lu_d(ENV, OP1, riscv_float_round_to_zero)
VFOP_VV(vfcvt_rtz_xuf_v, VFOP_FCVT_RTZ_XUF)

#define VFOP_FCVT_RTZ_XF_h(ENV, OP1) helper_fcvt_w_h(ENV, OP1, riscv_float_round_to_zero)
#define VFOP_FCVT_RTZ_XF_s(ENV, OP1) helper_fcvt_w_s(ENV, OP1, riscv_float_round_to_zero)
#define VFOP_FCVT_RTZ_XF_d(ENV, OP1) helper_fcvt_l_d(ENV, OP1, riscv_float_round_to_zero)
VFOP_VV(vfcvt_rtz_xf_v, VFOP_FCVT_RTZ_XF)

#define VFOP_FCVT_FXU_h(ENV, OP1) helper_fcvt_h_wu(ENV, OP1, ENV->frm)
#define VFOP_FCVT_FXU_s(ENV, OP1) helper_fcvt_s_wu(ENV, OP1, ENV->frm)
#define VFOP_FCVT_FXU_d(ENV, OP1) helper_fcvt_d_lu(ENV, OP1, ENV->frm)
VFOP_VV(vfcvt_fxu_v, VFOP_FCVT_FXU)

#define VFOP_FCVT_FX_h(ENV, OP1) helper_fcvt_h_w(ENV, OP1, ENV->frm)
#define VFOP_FCVT_FX_s(ENV, OP1) helper_fcvt_s_w(ENV, OP1, ENV->frm)
#define VFOP_FCVT_FX_d(ENV, OP1) helper_fcvt_d_l(ENV, OP1, ENV->frm)
VFOP_VV(vfcvt_fx_v, VFOP_FCVT_FX)

#define VFOP_FWCVT_XUF_d(ENV, OP1) helper_fcvt_lu_s(ENV, OP1, ENV->frm)
VFOP_WV_XF(vfwcvt_xuf_v, VFOP_FWCVT_XUF)

#define VFOP_FWCVT_XF_d(ENV, OP1) helper_fcvt_l_s(ENV, OP1, ENV->frm)
VFOP_WV_XF(vfwcvt_xf_v, VFOP_FWCVT_XF)

#define VFOP_FWCVT_RTZ_XUF_d(ENV, OP1) helper_fcvt_lu_s(ENV, OP1, riscv_float_round_to_zero)
VFOP_WV_XF(vfwcvt_rtz_xuf_v, VFOP_FWCVT_RTZ_XUF)

#define VFOP_FWCVT_RTZ_XF_d(ENV, OP1) helper_fcvt_l_s(ENV, OP1, riscv_float_round_to_zero)
VFOP_WV_XF(vfwcvt_rtz_xf_v, VFOP_FWCVT_RTZ_XF)

#define VFOP_FWCVT_FXU_s(ENV, OP1) helper_fcvt_s_hwu(ENV, OP1, ENV->frm)
#define VFOP_FWCVT_FXU_d(ENV, OP1) helper_fcvt_d_wu(ENV, OP1, ENV->frm)
VFOP_WV_FX(vfwcvt_fxu_v, VFOP_FWCVT_FXU)

#define VFOP_FWCVT_FX_s(ENV, OP1) helper_fcvt_s_hw(ENV, OP1, ENV->frm)
#define VFOP_FWCVT_FX_d(ENV, OP1) helper_fcvt_d_w(ENV, OP1, ENV->frm)
VFOP_WV_FX(vfwcvt_fx_v, VFOP_FWCVT_FX)

#define VFOP_FWCVT_FF_d(ENV, OP1) helper_fcvt_d_s(ENV, OP1, ENV->frm)
VFOP_WV_FF(vfwcvt_ff_v, VFOP_FWCVT_FF)

#define VFOP_FNCVT_XUF_s(ENV, OP1) helper_fcvt_hwu_s(ENV, OP1, ENV->frm)
#define VFOP_FNCVT_XUF_d(ENV, OP1) helper_fcvt_wu_d(ENV, OP1, ENV->frm)
VFOP_VW_XF(vfncvt_xuf_w, VFOP_FNCVT_XUF)

#define VFOP_FNCVT_XF_s(ENV, OP1) helper_fcvt_hw_s(ENV, OP1, ENV->frm)
#define VFOP_FNCVT_XF_d(ENV, OP1) helper_fcvt_w_d(ENV, OP1, ENV->frm)
VFOP_VW_XF(vfncvt_xf_w, VFOP_FNCVT_XF)

#define VFOP_FNCVT_RTZ_XUF_s(ENV, OP1) helper_fcvt_hwu_s(ENV, OP1, riscv_float_round_to_zero)
#define VFOP_FNCVT_RTZ_XUF_d(ENV, OP1) helper_fcvt_wu_d(ENV, OP1, riscv_float_round_to_zero)
VFOP_VW_XF(vfncvt_rtz_xuf_w, VFOP_FNCVT_RTZ_XUF)

#define VFOP_FNCVT_RTZ_XF_s(ENV, OP1) helper_fcvt_hw_s(ENV, OP1, riscv_float_round_to_zero)
#define VFOP_FNCVT_RTZ_XF_d(ENV, OP1) helper_fcvt_w_d(ENV, OP1, riscv_float_round_to_zero)
VFOP_VW_XF(vfncvt_rtz_xf_w, VFOP_FNCVT_RTZ_XF)

#define VFOP_FNCVT_FXU_s(ENV, OP1) helper_fcvt_s_lu(ENV, OP1, ENV->frm)
VFOP_VW_FX(vfncvt_fxu_w, VFOP_FNCVT_FXU)

#define VFOP_FNCVT_FX_s(ENV, OP1) helper_fcvt_s_l(ENV, OP1, ENV->frm)
VFOP_VW_FX(vfncvt_fx_w, VFOP_FNCVT_FX)

#define VFOP_FNCVT_FF_s(ENV, OP1) helper_fcvt_s_d(ENV, OP1, ENV->frm)
VFOP_VW_FF(vfncvt_ff_w, VFOP_FNCVT_FF)

#define VFOP_FNCVT_ROD_FF_s(ENV, OP1) helper_fcvt_s_d_rod(ENV, OP1)
VFOP_VW_FF(vfncvt_rod_ff_w, VFOP_FNCVT_ROD_FF)

#define VFOP_FADD_s(ENV, A, B) helper_fadd_s(ENV, A, B, ENV->frm)
#define VFOP_FADD_d(ENV, A, B) helper_fadd_d(ENV, A, B, ENV->frm)

VFOP_RED_VVV(vfredosum_vs, VFOP_FADD)
VFOP_RED_VVV(vfredmax_vs, helper_fmax)
VFOP_RED_VVV(vfredmin_vs, helper_fmin)

VFOP_RED_WVV(vfwredosum_vs, VFOP_FADD)

#undef MAYBE_CANONICALIZE_NAN
#define MAYBE_CANONICALIZE_NAN             \
    switch(eew) {                          \
        case 32:                           \
            if(float32_is_any_nan(acc)) {  \
                acc = float32_default_nan; \
            }                              \
            break;                         \
        case 64:                           \
            if(float64_is_any_nan(acc)) {  \
                acc = float64_default_nan; \
            }                              \
            break;                         \
    }

VFOP_RED_VVV(vfredusum_vs, VFOP_FADD)
#undef MAYBE_CANONICALIZE_NAN

#define MAYBE_CANONICALIZE_NAN     \
    if(float64_is_any_nan(acc)) {  \
        acc = float64_default_nan; \
    }
VFOP_RED_WVV(vfwredusum_vs, VFOP_FADD)
#undef MAYBE_CANONICALIZE_NAN

#undef MS_MASK
#undef MS_TEST_MASK
#undef VFMOP_LOOP_PREFIX
#undef TEST_MASK
#undef MASKED
#undef POSTFIX
