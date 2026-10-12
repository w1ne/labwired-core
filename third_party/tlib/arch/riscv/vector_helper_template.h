/*
 *  RISCV vector extention helpers
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
#include "arch_callbacks.h"

#define DATA_SIZE (1 << SHIFT)

#ifdef MASKED
#define POSTFIX               _m
#define IS_ELEMENT_ACTIVE(ei) (V(0)[(ei) >> 3] & (1 << ((ei) & 0x7)))
#else
#define POSTFIX
#define IS_ELEMENT_ACTIVE(ei) (true)
#endif

#define TEST_MASK(ei)            \
    if(!IS_ELEMENT_ACTIVE(ei)) { \
        continue;                \
    }

#if DATA_SIZE == 8
#define BITS               64
#define SUFFIX             q
#define USUFFIX            q
#define DATA_TYPE          uint64_t
#define DATA_TYPE_MAX      UINT64_MAX
#define DATA_TYPE_DOUBLED  __uint128_t
#define DATA_STYPE         int64_t
#define DATA_STYPE_MAX     INT64_MAX
#define DATA_STYPE_MIN     INT64_MIN
#define DATA_STYPE_DOUBLED __int128_t
#elif DATA_SIZE == 4
#define BITS               32
#define SUFFIX             l
#define USUFFIX            l
#define DATA_TYPE          uint32_t
#define DATA_TYPE_MAX      UINT32_MAX
#define DATA_TYPE_DOUBLED  uint64_t
#define DATA_STYPE         int32_t
#define DATA_STYPE_MAX     INT32_MAX
#define DATA_STYPE_MIN     INT32_MIN
#define DATA_STYPE_DOUBLED int64_t
#elif DATA_SIZE == 2
#define BITS               16
#define SUFFIX             w
#define USUFFIX            uw
#define DATA_TYPE          uint16_t
#define DATA_TYPE_MAX      UINT16_MAX
#define DATA_TYPE_DOUBLED  uint32_t
#define DATA_STYPE         int16_t
#define DATA_STYPE_MAX     INT16_MAX
#define DATA_STYPE_MIN     INT16_MIN
#define DATA_STYPE_DOUBLED int32_t
#elif DATA_SIZE == 1
#define BITS               8
#define SUFFIX             b
#define USUFFIX            ub
#define DATA_TYPE          uint8_t
#define DATA_TYPE_MAX      UINT8_MAX
#define DATA_TYPE_DOUBLED  uint16_t
#define DATA_STYPE         int8_t
#define DATA_STYPE_MAX     INT8_MAX
#define DATA_STYPE_MIN     INT8_MIN
#define DATA_STYPE_DOUBLED int16_t
#else
#error unsupported data size
#endif

#ifdef MASKED

static inline DATA_TYPE glue(roundoff_u, BITS)(DATA_TYPE v, uint16_t d, uint8_t rm)
{
    if(d == 0) {
        return v;
    }
    DATA_TYPE r = 0;
    switch(rm & 0b11) {
        case 0b00:  //  rnu
            r = (v >> (d - 1)) & 0b1;
            break;
        case 0b01:  //  rne
            r = ((v >> (d - 1)) & 0b1) && (((v >> d) & 0b1) || (d > 1 && (v & (((DATA_TYPE)1 << (d - 1)) - 1))));
            break;
        case 0b10:  //  rdn
            r = 0;
            break;
        case 0b11:  //  rod
            r = !((v >> d) & 0b1) && (v & (((DATA_TYPE)1 << d) - 1));
            break;
    }
    return (v >> d) + r;
}

static inline DATA_STYPE glue(roundoff_i, BITS)(DATA_STYPE v, uint16_t d, uint8_t rm)
{
    if(d == 0) {
        return v;
    }
    DATA_STYPE r = 0;
    switch(rm & 0b11) {
        case 0b00:  //  rnu
            r = (v >> (d - 1)) & 0b1;
            break;
        case 0b01:  //  rne
            r = ((v >> (d - 1)) & 0b1) && (((v >> d) & 0b1) || (d > 1 && (v & (((DATA_STYPE)1 << (d - 1)) - 1))));
            break;
        case 0b10:  //  rdn
            r = 0;
            break;
        case 0b11:  //  rod
            r = !((v >> d) & 0b1) && (v & (((DATA_STYPE)1 << d) - 1));
            break;
    }
    return (v >> d) + r;
}

static inline DATA_TYPE glue(clipto_u, BITS)(DATA_TYPE_DOUBLED val)
{
    if(val > DATA_TYPE_MAX) {
        env->vxsat |= 1;
        env->vcsr = (env->vcsr & ~VCSR_VXSAT) | env->vxsat;
        return DATA_TYPE_MAX;
    }
    return (DATA_TYPE)val;
}

static inline DATA_STYPE glue(clipto_i, BITS)(DATA_STYPE_DOUBLED val)
{
    if(val < DATA_STYPE_MIN) {
        env->vxsat |= 1;
        env->vcsr = (env->vcsr & ~VCSR_VXSAT) | env->vxsat;
        return DATA_STYPE_MIN;
    } else if(val > DATA_STYPE_MAX) {
        env->vxsat |= 1;
        env->vcsr = (env->vcsr & ~VCSR_VXSAT) | env->vxsat;
        return DATA_STYPE_MAX;
    }
    return (DATA_STYPE)val;
}

static inline DATA_TYPE glue(divu_, BITS)(DATA_TYPE dividend, DATA_TYPE divisor)
{
    if(divisor == 0) {
        return glue(glue(UINT, BITS), _MAX);
    } else {
        return dividend / divisor;
    }
}

static inline DATA_STYPE glue(div_, BITS)(DATA_STYPE dividend, DATA_STYPE divisor)
{
    if(divisor == 0) {
        return -1;
    } else if(divisor == -1 && dividend == glue(glue(INT, BITS), _MIN)) {
        return glue(glue(INT, BITS), _MIN);
    } else {
        return dividend / divisor;
    }
}

static inline DATA_TYPE glue(remu_, BITS)(DATA_TYPE dividend, DATA_TYPE divisor)
{
    if(divisor == 0) {
        return dividend;
    } else {
        return dividend % divisor;
    }
}

static inline DATA_STYPE glue(rem_, BITS)(DATA_STYPE dividend, DATA_STYPE divisor)
{
    if(divisor == 0) {
        return dividend;
    } else if(divisor == -1 && dividend == glue(glue(INT, BITS), _MIN)) {
        return 0;
    } else {
        return dividend % divisor;
    }
}

#endif
void glue(glue(helper_vle, BITS), POSTFIX)(CPUState *env, uint32_t vd, uint32_t rs1, uint32_t nf)
{
    void *retaddr = GETPC();
    //  This implementation handles both vle*.v and vlseg*.v instructions.
    const target_ulong emul = EMUL(SHIFT);
    const target_ulong nfields = nf + 1;
    if(emul == RESERVED_EMUL || V_IDX_INVALID_EMUL(vd, emul) || V_INVALID_NF(vd, nf, emul)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    target_ulong src_addr = env->gpr[rs1];
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        for(int fi = 0; fi < nfields; ++fi) {
            if(IS_ELEMENT_ACTIVE(ei)) {
                if(unlikely((env->is_pre_stack_access_hook_enabled && rs1 == SP))) {
                    tlib_handle_pre_stack_access_hook(src_addr, sizeof(DATA_TYPE) * 8, false);
                }
                ((DATA_TYPE *)V(vd + (fi << emul)))[ei] = glue(glue(ld, USUFFIX), _data_inner)(src_addr, retaddr);
            }
            src_addr += sizeof(DATA_TYPE);
        }
    }
}

void glue(glue(glue(helper_vle, BITS), ff), POSTFIX)(CPUState *env, uint32_t vd, uint32_t rs1, uint32_t nf)
{
    void *retaddr = GETPC();
    const target_ulong emul = EMUL(SHIFT);
    const target_ulong nfields = nf + 1;
    if(emul == RESERVED_EMUL || V_IDX_INVALID_EMUL(vd, emul) || V_INVALID_NF(vd, nf, emul)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }

    target_ulong src_addr = env->gpr[rs1];
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        for(int fi = 0; fi < nfields; ++fi) {
            if(IS_ELEMENT_ACTIVE(ei)) {
                DATA_TYPE *destination = &(((DATA_TYPE *)V(vd + (fi << emul)))[ei]);
                if(unlikely((env->is_pre_stack_access_hook_enabled && rs1 == SP))) {
                    tlib_handle_pre_stack_access_hook(src_addr, sizeof(DATA_TYPE) * 8, false);
                }
                if(ei == 0) {
                    *destination = glue(glue(ld, USUFFIX), _data_inner)(src_addr, retaddr);
                } else {
                    int memory_access_fail = 0;
                    *destination = glue(glue(ld, USUFFIX), _graceful)(src_addr, &memory_access_fail);
                    if(memory_access_fail) {
                        env->vl = ei;
                        env->exception_index = 0;
                        break;
                    }
                }
            }
            src_addr += sizeof(DATA_TYPE);
        }
    }
}

void glue(glue(helper_vlse, BITS), POSTFIX)(CPUState *env, uint32_t vd, uint32_t rs1, uint32_t rs2, uint32_t nf)
{
    void *retaddr = GETPC();
    //  This implementation handles both vlse*.v and vlsseg*.v instructions.
    const target_ulong emul = EMUL(SHIFT);
    const target_ulong nfields = nf + 1;
    if(emul == RESERVED_EMUL || V_IDX_INVALID_EMUL(vd, emul) || V_INVALID_NF(vd, nf, emul)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }

    target_ulong src_addr = env->gpr[rs1];
    target_long offset = env->gpr[rs2];
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        if(IS_ELEMENT_ACTIVE(ei)) {
            for(int fi = 0; fi < nfields; ++fi) {
                if(unlikely((env->is_pre_stack_access_hook_enabled && rs1 == SP))) {
                    tlib_handle_pre_stack_access_hook(src_addr, sizeof(DATA_TYPE) * 8, false);
                }
                ((DATA_TYPE *)V(vd + (fi << emul)))[ei] =
                    glue(glue(ld, USUFFIX), _data_inner)(src_addr + fi * sizeof(DATA_TYPE), retaddr);
            }
        }
        src_addr += offset;
    }
}

void glue(glue(helper_vlxei, BITS), POSTFIX)(CPUState *env, uint32_t vd, uint32_t rs1, uint32_t vs2, uint32_t nf)
{
    void *retaddr = GETPC();
    //  vd  has EEW=SEW, EMUL=LMUL
    //  vs2 has EEW encoded in the instruction with EMUL=(EEW/SEW)*LMUL
    const target_ulong data_emul = EMUL(SEW());
    const target_ulong index_emul = EMUL(SHIFT);
    if(index_emul == RESERVED_EMUL || data_emul == RESERVED_EMUL || V_IDX_INVALID_EMUL(vd, data_emul) ||
       V_IDX_INVALID_EMUL(vs2, index_emul) || V_INVALID_NF(vd, nf, data_emul)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    const target_ulong dst_eew = env->vsew;
    const target_ulong dst_shift = SEW();
    const target_ulong src_addr = env->gpr[rs1];
    const DATA_TYPE *offsets = (DATA_TYPE *)V(vs2);

    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        env->vstart = ei;
        for(int fi = 0; fi <= nf; ++fi) {
            int fi_offset = fi << dst_shift;
            target_ulong addr = src_addr + (target_ulong)offsets[ei] + fi_offset;
            if(unlikely((env->is_pre_stack_access_hook_enabled && rs1 == SP))) {
                tlib_handle_pre_stack_access_hook(addr, dst_eew, false);
            }
            switch(dst_eew) {
                case 8:
                    ((uint8_t *)V(vd + fi_offset))[ei] = ldub_data_inner(addr, retaddr);
                    break;
                case 16:
                    ((uint16_t *)V(vd + fi_offset))[ei] = lduw_data_inner(addr, retaddr);
                    break;
                case 32:
                    ((uint32_t *)V(vd + fi_offset))[ei] = ldl_data_inner(addr, retaddr);
                    break;
                case 64:
                    ((uint64_t *)V(vd + fi_offset))[ei] = ldq_data_inner(addr, retaddr);
                    break;
                default:
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                    break;
            }
        }
    }
}

void glue(glue(helper_vse, BITS), POSTFIX)(CPUState *env, uint32_t vd, uint32_t rs1, uint32_t nf)
{
    void *retaddr = GETPC();
    //  This implementation handles both vse*.v and vsseg*.v instructions.
    const target_ulong emul = EMUL(SHIFT);
    const target_ulong nfields = nf + 1;
    if(emul == RESERVED_EMUL || V_IDX_INVALID_EMUL(vd, emul) || V_INVALID_NF(vd, nf, emul)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }

    target_ulong dest_addr = env->gpr[rs1];
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        for(int fi = 0; fi < nfields; ++fi) {
            if(IS_ELEMENT_ACTIVE(ei)) {
                if(unlikely((env->is_pre_stack_access_hook_enabled && rs1 == SP))) {
                    tlib_handle_pre_stack_access_hook(dest_addr, sizeof(DATA_TYPE) * 8, true);
                }
                glue(glue(st, SUFFIX), _data_inner)(dest_addr, ((DATA_TYPE *)V(vd + (fi << emul)))[ei], retaddr);
            }
            dest_addr += sizeof(DATA_TYPE);
        }
    }
}

void glue(glue(helper_vsse, BITS), POSTFIX)(CPUState *env, uint32_t vd, uint32_t rs1, uint32_t rs2, uint32_t nf)
{
    void *retaddr = GETPC();
    //  This implementation handles both vsse*.v and vssseg*.v instructions.
    const target_ulong emul = EMUL(SHIFT);
    const target_ulong nfields = nf + 1;
    if(emul == RESERVED_EMUL || V_IDX_INVALID_EMUL(vd, emul) || V_INVALID_NF(vd, nf, emul)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    target_ulong dest_addr = env->gpr[rs1];
    target_long offset = env->gpr[rs2];
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        if(IS_ELEMENT_ACTIVE(ei)) {
            if(unlikely((env->is_pre_stack_access_hook_enabled && rs1 == SP))) {
                tlib_handle_pre_stack_access_hook(dest_addr, sizeof(DATA_TYPE) * nfields * 8, true);
            }
            for(int fi = 0; fi < nfields; ++fi) {
                glue(glue(st, SUFFIX), _data_inner)(dest_addr + fi * sizeof(DATA_TYPE), ((DATA_TYPE *)V(vd + (fi << emul)))[ei],
                                                    retaddr);
            }
        }
        dest_addr += offset;
    }
}

void glue(glue(helper_vsxei, BITS), POSTFIX)(CPUState *env, uint32_t vd, uint32_t rs1, uint32_t vs2, uint32_t nf)
{
    void *retaddr = GETPC();
    const target_ulong emul = EMUL(SHIFT);
    if(emul == RESERVED_EMUL || V_IDX_INVALID(vd) || V_IDX_INVALID_EMUL(vs2, emul) || V_INVALID_NF(vd, nf, emul)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    target_ulong src_addr = env->gpr[rs1];
    DATA_TYPE *offsets = (DATA_TYPE *)V(vs2);
    const target_ulong dst_eew = env->vsew;

    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        env->vstart = ei;
        for(int fi = 0; fi <= nf; ++fi) {
            if(unlikely((env->is_pre_stack_access_hook_enabled && rs1 == SP))) {
                tlib_handle_pre_stack_access_hook(src_addr + (target_ulong)offsets[ei], dst_eew, true);
            }
            switch(dst_eew) {
                case 8:
                    stb_data_inner(src_addr + (target_ulong)offsets[ei] + (fi << 0), V(vd + (fi << SHIFT))[ei], retaddr);
                    break;
                case 16:
                    stw_data_inner(src_addr + (target_ulong)offsets[ei] + (fi << 1), ((uint16_t *)V(vd + (fi << SHIFT)))[ei],
                                   retaddr);
                    break;
                case 32:
                    stl_data_inner(src_addr + (target_ulong)offsets[ei] + (fi << 2), ((uint32_t *)V(vd + (fi << SHIFT)))[ei],
                                   retaddr);
                    break;
                case 64:
                    stq_data_inner(src_addr + (target_ulong)offsets[ei] + (fi << 3), ((uint64_t *)V(vd + (fi << SHIFT)))[ei],
                                   retaddr);
                    break;
                default:
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                    break;
            }
        }
    }
}

#ifndef V_HELPER_ONCE
#define V_HELPER_ONCE

static inline __uint128_t roundoff_u128(__uint128_t v, uint16_t d, uint8_t rm)
{
    if(d == 0) {
        return v;
    }
    DATA_TYPE r = 0;
    switch(rm & 0b11) {
        case 0b00:  //  rnu
            r = (v >> (d - 1)) & 0b1;
            break;
        case 0b01:  //  rne
            r = ((v >> (d - 1)) & 0b1) && (((v >> d) & 0b1) || (d > 1 && (v & ((1 << (d - 1)) - 1))));
            break;
        case 0b10:  //  rdn
            r = 0;
            break;
        case 0b11:  //  rod
            r = !((v >> d) & 0b1) && (v & ((1 << d) - 1));
            break;
    }
    return (v >> d) + r;
}

static inline __int128_t roundoff_i128(__int128_t v, uint16_t d, uint8_t rm)
{
    if(d == 0) {
        return v;
    }
    DATA_STYPE r = 0;
    switch(rm & 0b11) {
        case 0b00:  //  rnu
            r = (v >> (d - 1)) & 0b1;
            break;
        case 0b01:  //  rne
            r = ((v >> (d - 1)) & 0b1) && (((v >> d) & 0b1) || (d > 1 && (v & ((1 << (d - 1)) - 1))));
            break;
        case 0b10:  //  rdn
            r = 0;
            break;
        case 0b11:  //  rod
            r = !((v >> d) & 0b1) && (v & ((1 << d) - 1));
            break;
    }
    return (v >> d) + r;
}

void helper_vl_wr(CPUState *env, uint32_t vd, uint32_t rs1, uint32_t nf)
{
    uint8_t *v = V(vd);
    uint8_t nfield = nf + 1;
    target_ulong src_addr = env->gpr[rs1];
    int32_t elements = env->vlenb * nfield;
    if(unlikely((env->is_pre_stack_access_hook_enabled && rs1 == SP))) {
        tlib_handle_pre_stack_access_hook(src_addr, elements * 8, false);
    }
    for(int i = 0; i < elements; ++i) {
        env->vstart = i;
        v[i] = ldub(src_addr + i);
    }
}

void helper_vs_wr(CPUState *env, uint32_t vd, uint32_t rs1, uint32_t nf)
{
    uint8_t *v = V(vd);
    uint8_t nfield = nf + 1;
    target_ulong src_addr = env->gpr[rs1];
    int32_t elements = env->vlenb * nfield;
    if(unlikely((env->is_pre_stack_access_hook_enabled && rs1 == SP))) {
        tlib_handle_pre_stack_access_hook(src_addr, elements * 8, true);
    }
    for(int i = 0; i < elements; ++i) {
        env->vstart = i;
        stb(src_addr + i, v[i]);
    }
}

void helper_vlm(CPUState *env, uint32_t vd, uint32_t rs1)
{
    uint8_t *v = V(vd);
    target_ulong src_addr = env->gpr[rs1];
    uint32_t elements = ((env->vl + 7) / 8);
    if(unlikely((env->is_pre_stack_access_hook_enabled && rs1 == SP))) {
        tlib_handle_pre_stack_access_hook(src_addr, elements * 8, false);
    }
    for(int i = env->vstart; i < elements; ++i) {
        env->vstart = i;
        v[i] = ldub(src_addr + i);
    }
}

void helper_vsm(CPUState *env, uint32_t vd, uint32_t rs1)
{
    uint8_t *v = V(vd);
    target_ulong src_addr = env->gpr[rs1];
    uint32_t elements = ((env->vl + 7) / 8);
    if(unlikely((env->is_pre_stack_access_hook_enabled && rs1 == SP))) {
        tlib_handle_pre_stack_access_hook(src_addr, elements * 8, true);
    }
    for(int i = env->vstart; i < elements; ++i) {
        env->vstart = i;
        stb(src_addr + i, v[i]);
    }
}

#endif

#if SHIFT == 3

#define VOP_UNSIGNED_VVX(NAME, OP)                                                                      \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, target_long imm) \
    {                                                                                                   \
        const target_ulong eew = env->vsew;                                                             \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);                          \
        if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2)) {                                                   \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                  \
        }                                                                                               \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                 \
            TEST_MASK(ei)                                                                               \
            switch(eew) {                                                                               \
                case 8:                                                                                 \
                    ((uint8_t *)V(vd))[ei] = OP(((uint8_t *)V(vs2))[ei], ((uint8_t)(int8_t)imm));       \
                    break;                                                                              \
                case 16:                                                                                \
                    ((uint16_t *)V(vd))[ei] = OP(((uint16_t *)V(vs2))[ei], ((uint16_t)(int16_t)imm));   \
                    break;                                                                              \
                case 32:                                                                                \
                    ((uint32_t *)V(vd))[ei] = OP(((uint32_t *)V(vs2))[ei], ((uint32_t)(int32_t)imm));   \
                    break;                                                                              \
                case 64:                                                                                \
                    ((uint64_t *)V(vd))[ei] = OP(((uint64_t *)V(vs2))[ei], ((uint64_t)(int64_t)imm));   \
                    break;                                                                              \
                default:                                                                                \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                          \
                    break;                                                                              \
            }                                                                                           \
        }                                                                                               \
    }

#define VOP_SIGNED_VVX(NAME, OP)                                                                        \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, target_long imm) \
    {                                                                                                   \
        const target_ulong eew = env->vsew;                                                             \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);                          \
        if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2)) {                                                   \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                  \
        }                                                                                               \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                 \
            TEST_MASK(ei)                                                                               \
            switch(eew) {                                                                               \
                case 8:                                                                                 \
                    ((int8_t *)V(vd))[ei] = OP(((int8_t *)V(vs2))[ei], ((int8_t)imm));                  \
                    break;                                                                              \
                case 16:                                                                                \
                    ((int16_t *)V(vd))[ei] = OP(((int16_t *)V(vs2))[ei], ((int16_t)imm));               \
                    break;                                                                              \
                case 32:                                                                                \
                    ((int32_t *)V(vd))[ei] = OP(((int32_t *)V(vs2))[ei], ((int32_t)imm));               \
                    break;                                                                              \
                case 64:                                                                                \
                    ((int64_t *)V(vd))[ei] = OP(((int64_t *)V(vs2))[ei], ((int64_t)imm));               \
                    break;                                                                              \
                default:                                                                                \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                          \
                    break;                                                                              \
            }                                                                                           \
        }                                                                                               \
    }

#define VOP_SIGNED_UNSIGNED_VVX(NAME, OP)                                                               \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, target_long imm) \
    {                                                                                                   \
        const target_ulong eew = env->vsew;                                                             \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);                          \
        if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2)) {                                                   \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                  \
        }                                                                                               \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                 \
            TEST_MASK(ei)                                                                               \
            switch(eew) {                                                                               \
                case 8:                                                                                 \
                    ((int8_t *)V(vd))[ei] = OP(((int8_t *)V(vs2))[ei], ((uint8_t)(int8_t)imm));         \
                    break;                                                                              \
                case 16:                                                                                \
                    ((int16_t *)V(vd))[ei] = OP(((int16_t *)V(vs2))[ei], ((uint16_t)(int16_t)imm));     \
                    break;                                                                              \
                case 32:                                                                                \
                    ((int32_t *)V(vd))[ei] = OP(((int32_t *)V(vs2))[ei], ((uint32_t)(int32_t)imm));     \
                    break;                                                                              \
                case 64:                                                                                \
                    ((int64_t *)V(vd))[ei] = OP(((int64_t *)V(vs2))[ei], ((uint64_t)(int64_t)imm));     \
                    break;                                                                              \
                default:                                                                                \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                          \
                    break;                                                                              \
            }                                                                                           \
        }                                                                                               \
    }

#define VOP_UNSIGNED_VVV(NAME, OP)                                                                    \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1)  \
    {                                                                                                 \
        const target_ulong eew = env->vsew;                                                           \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);                        \
        if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {                           \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                \
        }                                                                                             \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                               \
            TEST_MASK(ei)                                                                             \
            switch(eew) {                                                                             \
                case 8:                                                                               \
                    ((uint8_t *)V(vd))[ei] = OP(((uint8_t *)V(vs2))[ei], ((uint8_t *)V(vs1))[ei]);    \
                    break;                                                                            \
                case 16:                                                                              \
                    ((uint16_t *)V(vd))[ei] = OP(((uint16_t *)V(vs2))[ei], ((uint16_t *)V(vs1))[ei]); \
                    break;                                                                            \
                case 32:                                                                              \
                    ((uint32_t *)V(vd))[ei] = OP(((uint32_t *)V(vs2))[ei], ((uint32_t *)V(vs1))[ei]); \
                    break;                                                                            \
                case 64:                                                                              \
                    ((uint64_t *)V(vd))[ei] = OP(((uint64_t *)V(vs2))[ei], ((uint64_t *)V(vs1))[ei]); \
                    break;                                                                            \
                default:                                                                              \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                        \
                    break;                                                                            \
            }                                                                                         \
        }                                                                                             \
    }

#define VOP_SIGNED_VVV(NAME, OP)                                                                     \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1) \
    {                                                                                                \
        const target_ulong eew = env->vsew;                                                          \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);                       \
        if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {                          \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                               \
        }                                                                                            \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                              \
            TEST_MASK(ei)                                                                            \
            switch(eew) {                                                                            \
                case 8:                                                                              \
                    ((int8_t *)V(vd))[ei] = OP(((int8_t *)V(vs2))[ei], ((int8_t *)V(vs1))[ei]);      \
                    break;                                                                           \
                case 16:                                                                             \
                    ((int16_t *)V(vd))[ei] = OP(((int16_t *)V(vs2))[ei], ((int16_t *)V(vs1))[ei]);   \
                    break;                                                                           \
                case 32:                                                                             \
                    ((int32_t *)V(vd))[ei] = OP(((int32_t *)V(vs2))[ei], ((int32_t *)V(vs1))[ei]);   \
                    break;                                                                           \
                case 64:                                                                             \
                    ((int64_t *)V(vd))[ei] = OP(((int64_t *)V(vs2))[ei], ((int64_t *)V(vs1))[ei]);   \
                    break;                                                                           \
                default:                                                                             \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                       \
                    break;                                                                           \
            }                                                                                        \
        }                                                                                            \
    }

#define VOP_SIGNED_UNSIGNED_VVV(NAME, OP)                                                            \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1) \
    {                                                                                                \
        const target_ulong eew = env->vsew;                                                          \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);                       \
        if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {                          \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                               \
        }                                                                                            \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                              \
            TEST_MASK(ei)                                                                            \
            switch(eew) {                                                                            \
                case 8:                                                                              \
                    ((int8_t *)V(vd))[ei] = OP(((int8_t *)V(vs2))[ei], ((uint8_t *)V(vs1))[ei]);     \
                    break;                                                                           \
                case 16:                                                                             \
                    ((int16_t *)V(vd))[ei] = OP(((int16_t *)V(vs2))[ei], ((uint16_t *)V(vs1))[ei]);  \
                    break;                                                                           \
                case 32:                                                                             \
                    ((int32_t *)V(vd))[ei] = OP(((int32_t *)V(vs2))[ei], ((uint32_t *)V(vs1))[ei]);  \
                    break;                                                                           \
                case 64:                                                                             \
                    ((int64_t *)V(vd))[ei] = OP(((int64_t *)V(vs2))[ei], ((uint64_t *)V(vs1))[ei]);  \
                    break;                                                                           \
                default:                                                                             \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                       \
                    break;                                                                           \
            }                                                                                        \
        }                                                                                            \
    }

#define VOP_UNSIGNED_WVX(NAME, OP)                                                                                        \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, target_long imm)                   \
    {                                                                                                                     \
        const target_ulong eew = env->vsew;                                                                               \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);                                       \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID(vs2)) {                                                       \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                                    \
        }                                                                                                                 \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                                   \
            TEST_MASK(ei)                                                                                                 \
            switch(eew) {                                                                                                 \
                case 8:                                                                                                   \
                    ((uint16_t *)V(vd))[ei] = OP((uint16_t)((uint8_t *)V(vs2))[ei], (uint16_t)((uint8_t)(int8_t)imm));    \
                    break;                                                                                                \
                case 16:                                                                                                  \
                    ((uint32_t *)V(vd))[ei] = OP((uint32_t)((uint16_t *)V(vs2))[ei], (uint32_t)((uint16_t)(int16_t)imm)); \
                    break;                                                                                                \
                case 32:                                                                                                  \
                    ((uint64_t *)V(vd))[ei] = OP((uint64_t)((uint32_t *)V(vs2))[ei], (uint64_t)((uint32_t)(int32_t)imm)); \
                    break;                                                                                                \
                default:                                                                                                  \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                            \
                    break;                                                                                                \
            }                                                                                                             \
        }                                                                                                                 \
    }

#define VOP_SIGNED_WVX(NAME, OP)                                                                            \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, target_long imm)     \
    {                                                                                                       \
        const target_ulong eew = env->vsew;                                                                 \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);                         \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID(vs2)) {                                         \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                      \
        }                                                                                                   \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                     \
            TEST_MASK(ei)                                                                                   \
            switch(eew) {                                                                                   \
                case 8:                                                                                     \
                    ((int16_t *)V(vd))[ei] = OP((int16_t)((int8_t *)V(vs2))[ei], (int16_t)((int8_t)imm));   \
                    break;                                                                                  \
                case 16:                                                                                    \
                    ((int32_t *)V(vd))[ei] = OP((int32_t)((int16_t *)V(vs2))[ei], (int32_t)((int16_t)imm)); \
                    break;                                                                                  \
                case 32:                                                                                    \
                    ((int64_t *)V(vd))[ei] = OP((int64_t)((int32_t *)V(vs2))[ei], (int64_t)((int32_t)imm)); \
                    break;                                                                                  \
                default:                                                                                    \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                              \
                    break;                                                                                  \
            }                                                                                               \
        }                                                                                                   \
    }

#define VOP_SIGNED_UNSIGNED_WVX(NAME, OP)                                                                              \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, target_long imm)                \
    {                                                                                                                  \
        const target_ulong eew = env->vsew;                                                                            \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);                                    \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID(vs2)) {                                                    \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                                 \
        }                                                                                                              \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                                \
            TEST_MASK(ei)                                                                                              \
            switch(eew) {                                                                                              \
                case 8:                                                                                                \
                    ((int16_t *)V(vd))[ei] = OP((int16_t)((int8_t *)V(vs2))[ei], (uint16_t)((uint8_t)(int8_t)imm));    \
                    break;                                                                                             \
                case 16:                                                                                               \
                    ((int32_t *)V(vd))[ei] = OP((int32_t)((int16_t *)V(vs2))[ei], (uint32_t)((uint16_t)(int16_t)imm)); \
                    break;                                                                                             \
                case 32:                                                                                               \
                    ((int64_t *)V(vd))[ei] = OP((int64_t)((int32_t *)V(vs2))[ei], (uint64_t)((uint32_t)(int32_t)imm)); \
                    break;                                                                                             \
                default:                                                                                               \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                         \
                    break;                                                                                             \
            }                                                                                                          \
        }                                                                                                              \
    }

#define VOP_UNSIGNED_WVV(NAME, OP)                                                                                        \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1)                      \
    {                                                                                                                     \
        const target_ulong eew = env->vsew;                                                                               \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);                                       \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {                                 \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                                    \
        }                                                                                                                 \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                                   \
            TEST_MASK(ei)                                                                                                 \
            switch(eew) {                                                                                                 \
                case 8:                                                                                                   \
                    ((uint16_t *)V(vd))[ei] = OP((uint16_t)((uint8_t *)V(vs2))[ei], (uint16_t)((uint8_t *)V(vs1))[ei]);   \
                    break;                                                                                                \
                case 16:                                                                                                  \
                    ((uint32_t *)V(vd))[ei] = OP((uint32_t)((uint16_t *)V(vs2))[ei], (uint32_t)((uint16_t *)V(vs1))[ei]); \
                    break;                                                                                                \
                case 32:                                                                                                  \
                    ((uint64_t *)V(vd))[ei] = OP((uint64_t)((uint32_t *)V(vs2))[ei], (uint64_t)((uint32_t *)V(vs1))[ei]); \
                    break;                                                                                                \
                default:                                                                                                  \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                            \
                    break;                                                                                                \
            }                                                                                                             \
        }                                                                                                                 \
    }

#define VOP_SIGNED_WVV(NAME, OP)                                                                                     \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1)                 \
    {                                                                                                                \
        const target_ulong eew = env->vsew;                                                                          \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);                                  \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {                            \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                               \
        }                                                                                                            \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                              \
            TEST_MASK(ei)                                                                                            \
            switch(eew) {                                                                                            \
                case 8:                                                                                              \
                    ((int16_t *)V(vd))[ei] = OP((int16_t)((int8_t *)V(vs2))[ei], (int16_t)((int8_t *)V(vs1))[ei]);   \
                    break;                                                                                           \
                case 16:                                                                                             \
                    ((int32_t *)V(vd))[ei] = OP((int32_t)((int16_t *)V(vs2))[ei], (int32_t)((int16_t *)V(vs1))[ei]); \
                    break;                                                                                           \
                case 32:                                                                                             \
                    ((int64_t *)V(vd))[ei] = OP((int64_t)((int32_t *)V(vs2))[ei], (int64_t)((int32_t *)V(vs1))[ei]); \
                    break;                                                                                           \
                default:                                                                                             \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                       \
                    break;                                                                                           \
            }                                                                                                        \
        }                                                                                                            \
    }

#define VOP_SIGNED_UNSIGNED_WVV(NAME, OP)                                                                              \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1)                   \
    {                                                                                                                  \
        const target_ulong eew = env->vsew;                                                                            \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);                                    \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {                              \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                                 \
        }                                                                                                              \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                                \
            TEST_MASK(ei)                                                                                              \
            switch(eew) {                                                                                              \
                case 8:                                                                                                \
                    ((int16_t *)V(vd))[ei] = OP((int16_t)((int8_t *)V(vs2))[ei], (uint16_t)((uint8_t *)V(vs1))[ei]);   \
                    break;                                                                                             \
                case 16:                                                                                               \
                    ((int32_t *)V(vd))[ei] = OP((int32_t)((int16_t *)V(vs2))[ei], (uint32_t)((uint16_t *)V(vs1))[ei]); \
                    break;                                                                                             \
                case 32:                                                                                               \
                    ((int64_t *)V(vd))[ei] = OP((int64_t)((int32_t *)V(vs2))[ei], (uint64_t)((uint32_t *)V(vs1))[ei]); \
                    break;                                                                                             \
                default:                                                                                               \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                         \
                    break;                                                                                             \
            }                                                                                                          \
        }                                                                                                              \
    }

#define VOP_UNSIGNED_WWX(NAME, OP)                                                                      \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, target_long imm) \
    {                                                                                                   \
        const target_ulong eew = env->vsew;                                                             \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);                     \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID_EEW(vs2, eew << 1)) {                       \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                  \
        }                                                                                               \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                 \
            TEST_MASK(ei)                                                                               \
            switch(eew) {                                                                               \
                case 8:                                                                                 \
                    ((uint16_t *)V(vd))[ei] = OP(((uint16_t *)V(vs2))[ei], (uint8_t)((int8_t)imm));     \
                    break;                                                                              \
                case 16:                                                                                \
                    ((uint32_t *)V(vd))[ei] = OP(((uint32_t *)V(vs2))[ei], (uint16_t)((int16_t)imm));   \
                    break;                                                                              \
                case 32:                                                                                \
                    ((uint64_t *)V(vd))[ei] = OP(((uint64_t *)V(vs2))[ei], (uint32_t)((int32_t)imm));   \
                    break;                                                                              \
                default:                                                                                \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                          \
                    break;                                                                              \
            }                                                                                           \
        }                                                                                               \
    }

#define VOP_SIGNED_WWX(NAME, OP)                                                                        \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, target_long imm) \
    {                                                                                                   \
        const target_ulong eew = env->vsew;                                                             \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);                     \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID_EEW(vs2, eew << 1)) {                       \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                  \
        }                                                                                               \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                 \
            TEST_MASK(ei)                                                                               \
            switch(eew) {                                                                               \
                case 8:                                                                                 \
                    ((int16_t *)V(vd))[ei] = OP(((int16_t *)V(vs2))[ei], (int8_t)(imm));                \
                    break;                                                                              \
                case 16:                                                                                \
                    ((int32_t *)V(vd))[ei] = OP(((int32_t *)V(vs2))[ei], (int16_t)(imm));               \
                    break;                                                                              \
                case 32:                                                                                \
                    ((int64_t *)V(vd))[ei] = OP(((int64_t *)V(vs2))[ei], (int32_t)(imm));               \
                    break;                                                                              \
                default:                                                                                \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                          \
                    break;                                                                              \
            }                                                                                           \
        }                                                                                               \
    }

#define VOP_UNSIGNED_WWV(NAME, OP)                                                                              \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1)            \
    {                                                                                                           \
        const target_ulong eew = env->vsew;                                                                     \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);                             \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID_EEW(vs2, eew << 1) || V_IDX_INVALID(vs1)) {         \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                          \
        }                                                                                                       \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                         \
            TEST_MASK(ei)                                                                                       \
            switch(eew) {                                                                                       \
                case 8:                                                                                         \
                    ((uint16_t *)V(vd))[ei] = OP(((uint16_t *)V(vs2))[ei], (uint16_t)((uint8_t *)V(vs1))[ei]);  \
                    break;                                                                                      \
                case 16:                                                                                        \
                    ((uint32_t *)V(vd))[ei] = OP(((uint32_t *)V(vs2))[ei], (uint32_t)((uint16_t *)V(vs1))[ei]); \
                    break;                                                                                      \
                case 32:                                                                                        \
                    ((uint64_t *)V(vd))[ei] = OP(((uint64_t *)V(vs2))[ei], (uint64_t)((uint32_t *)V(vs1))[ei]); \
                    break;                                                                                      \
                default:                                                                                        \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                  \
                    break;                                                                                      \
            }                                                                                                   \
        }                                                                                                       \
    }

#define VOP_SIGNED_WWV(NAME, OP)                                                                            \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1)        \
    {                                                                                                       \
        const target_ulong eew = env->vsew;                                                                 \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);                         \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID_EEW(vs2, eew << 1) || V_IDX_INVALID(vs1)) {     \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                      \
        }                                                                                                   \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                     \
            TEST_MASK(ei)                                                                                   \
            switch(eew) {                                                                                   \
                case 8:                                                                                     \
                    ((int16_t *)V(vd))[ei] = OP(((int16_t *)V(vs2))[ei], (int16_t)((int8_t *)V(vs1))[ei]);  \
                    break;                                                                                  \
                case 16:                                                                                    \
                    ((int32_t *)V(vd))[ei] = OP(((int32_t *)V(vs2))[ei], (int32_t)((int16_t *)V(vs1))[ei]); \
                    break;                                                                                  \
                case 32:                                                                                    \
                    ((int64_t *)V(vd))[ei] = OP(((int64_t *)V(vs2))[ei], (int64_t)((int32_t *)V(vs1))[ei]); \
                    break;                                                                                  \
                default:                                                                                    \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                              \
                    break;                                                                                  \
            }                                                                                               \
        }                                                                                                   \
    }

#define VOP_UNSIGNED_VWX(NAME, OP)                                                                              \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, target_long imm)         \
    {                                                                                                           \
        const target_ulong eew = env->vsew;                                                                     \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);                             \
        if(V_IDX_INVALID(vd) || V_IDX_INVALID_EEW(vs2, eew << 1)) {                                             \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                          \
        }                                                                                                       \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                         \
            TEST_MASK(ei)                                                                                       \
            switch(eew) {                                                                                       \
                case 8:                                                                                         \
                    ((uint8_t *)V(vd))[ei] = OP(((uint16_t *)V(vs2))[ei], (uint16_t)((uint8_t)(int8_t)imm));    \
                    break;                                                                                      \
                case 16:                                                                                        \
                    ((uint16_t *)V(vd))[ei] = OP(((uint32_t *)V(vs2))[ei], (uint32_t)((uint16_t)(int16_t)imm)); \
                    break;                                                                                      \
                case 32:                                                                                        \
                    ((uint32_t *)V(vd))[ei] = OP(((uint64_t *)V(vs2))[ei], (uint64_t)((uint32_t)(int32_t)imm)); \
                    break;                                                                                      \
                default:                                                                                        \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                  \
                    break;                                                                                      \
            }                                                                                                   \
        }                                                                                                       \
    }

#define VOP_SIGNED_VWX(NAME, OP)                                                                        \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, target_long imm) \
    {                                                                                                   \
        const target_ulong eew = env->vsew;                                                             \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);                     \
        if(V_IDX_INVALID(vd) || V_IDX_INVALID_EEW(vs2, eew << 1)) {                                     \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                  \
        }                                                                                               \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                 \
            TEST_MASK(ei)                                                                               \
            switch(eew) {                                                                               \
                case 8:                                                                                 \
                    ((int8_t *)V(vd))[ei] = OP(((int16_t *)V(vs2))[ei], (int16_t)(imm));                \
                    break;                                                                              \
                case 16:                                                                                \
                    ((int16_t *)V(vd))[ei] = OP(((int32_t *)V(vs2))[ei], (int32_t)(imm));               \
                    break;                                                                              \
                case 32:                                                                                \
                    ((int32_t *)V(vd))[ei] = OP(((int64_t *)V(vs2))[ei], (int64_t)(imm));               \
                    break;                                                                              \
                default:                                                                                \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                          \
                    break;                                                                              \
            }                                                                                           \
        }                                                                                               \
    }

#define VOP_UNSIGNED_VWV(NAME, OP)                                                                              \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1)            \
    {                                                                                                           \
        const target_ulong eew = env->vsew;                                                                     \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);                             \
        if(V_IDX_INVALID(vd) || V_IDX_INVALID_EEW(vs2, eew << 1) || V_IDX_INVALID(vs1)) {                       \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                          \
        }                                                                                                       \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                         \
            TEST_MASK(ei)                                                                                       \
            switch(eew) {                                                                                       \
                case 8:                                                                                         \
                    ((uint8_t *)V(vd))[ei] = OP(((uint16_t *)V(vs2))[ei], (uint16_t)((uint8_t *)V(vs1))[ei]);   \
                    break;                                                                                      \
                case 16:                                                                                        \
                    ((uint16_t *)V(vd))[ei] = OP(((uint32_t *)V(vs2))[ei], (uint32_t)((uint16_t *)V(vs1))[ei]); \
                    break;                                                                                      \
                case 32:                                                                                        \
                    ((uint32_t *)V(vd))[ei] = OP(((uint64_t *)V(vs2))[ei], (uint64_t)((uint32_t *)V(vs1))[ei]); \
                    break;                                                                                      \
                default:                                                                                        \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                  \
                    break;                                                                                      \
            }                                                                                                   \
        }                                                                                                       \
    }

#define VOP_SIGNED_VWV(NAME, OP)                                                                            \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1)        \
    {                                                                                                       \
        const target_ulong eew = env->vsew;                                                                 \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);                         \
        if(V_IDX_INVALID(vd) || V_IDX_INVALID_EEW(vs2, eew << 1) || V_IDX_INVALID(vs1)) {                   \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                      \
        }                                                                                                   \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                     \
            TEST_MASK(ei)                                                                                   \
            switch(eew) {                                                                                   \
                case 8:                                                                                     \
                    ((int8_t *)V(vd))[ei] = OP(((int16_t *)V(vs2))[ei], (int16_t)((int8_t *)V(vs1))[ei]);   \
                    break;                                                                                  \
                case 16:                                                                                    \
                    ((int16_t *)V(vd))[ei] = OP(((int32_t *)V(vs2))[ei], (int32_t)((int16_t *)V(vs1))[ei]); \
                    break;                                                                                  \
                case 32:                                                                                    \
                    ((int32_t *)V(vd))[ei] = OP(((int64_t *)V(vs2))[ei], (int64_t)((int32_t *)V(vs1))[ei]); \
                    break;                                                                                  \
                default:                                                                                    \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                              \
                    break;                                                                                  \
            }                                                                                               \
        }                                                                                                   \
    }

#define MS_VL_MASK ((env->vl - ei > 0x7) ? 0 : (0xff << (env->vl & 0x7)))
#ifdef MASKED
#define MS_MASK()      (MS_VL_MASK | ~V(0)[(ei >> 3)])
#define MS_TEST_MASK() (~mask & (1 << (ei & 0x7)))
#else
#define MS_MASK()      MS_VL_MASK
#define MS_TEST_MASK() (true)
#endif

#define VMOP_LOOP_PREFIX() \
    if(!(ei & 0x7)) {      \
        mask = MS_MASK();  \
    }
#define VMOP_LOOP_SUFFIX()                         \
    if(!((ei + 1) & 0x7) || (ei + 1) >= env->vl) { \
        V(vd)[(ei >> 3)] &= mask;                  \
        V(vd)[(ei >> 3)] |= value;                 \
        value = 0;                                 \
    }

#define VMOP_UNSIGNED_VX(NAME, OP)                                                                       \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, target_ulong imm) \
    {                                                                                                    \
        const target_ulong eew = env->vsew;                                                              \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);                           \
        if(V_IDX_INVALID_EEW(vd, 8) || V_IDX_INVALID(vs2)) {                                             \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                   \
        }                                                                                                \
        uint8_t mask = 0, value = 0;                                                                     \
        for(int ei = 0; ei < env->vl; ++ei) {                                                            \
            VMOP_LOOP_PREFIX();                                                                          \
            if(MS_TEST_MASK()) {                                                                         \
                switch(eew) {                                                                            \
                    case 8:                                                                              \
                        value |= OP(((uint8_t *)V(vs2))[ei], ((uint8_t)imm)) << (ei & 0x7);              \
                        break;                                                                           \
                    case 16:                                                                             \
                        value |= OP(((uint16_t *)V(vs2))[ei], ((uint16_t)imm)) << (ei & 0x7);            \
                        break;                                                                           \
                    case 32:                                                                             \
                        value |= OP(((uint32_t *)V(vs2))[ei], ((uint32_t)imm)) << (ei & 0x7);            \
                        break;                                                                           \
                    case 64:                                                                             \
                        value |= OP(((uint64_t *)V(vs2))[ei], ((uint64_t)imm)) << (ei & 0x7);            \
                        break;                                                                           \
                    default:                                                                             \
                        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                       \
                        break;                                                                           \
                }                                                                                        \
            }                                                                                            \
            VMOP_LOOP_SUFFIX();                                                                          \
        }                                                                                                \
    }

#define VMOP_SIGNED_VX(NAME, OP)                                                                        \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, target_long imm) \
    {                                                                                                   \
        const target_ulong eew = env->vsew;                                                             \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);                          \
        if(V_IDX_INVALID_EEW(vd, 8) || V_IDX_INVALID(vs2)) {                                            \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                  \
        }                                                                                               \
        uint8_t mask = 0, value = 0;                                                                    \
        for(int ei = 0; ei < env->vl; ++ei) {                                                           \
            VMOP_LOOP_PREFIX();                                                                         \
            if(MS_TEST_MASK()) {                                                                        \
                switch(eew) {                                                                           \
                    case 8:                                                                             \
                        value |= OP(((int8_t *)V(vs2))[ei], ((int8_t)imm)) << (ei & 0x7);               \
                        break;                                                                          \
                    case 16:                                                                            \
                        value |= OP(((int16_t *)V(vs2))[ei], ((int16_t)imm)) << (ei & 0x7);             \
                        break;                                                                          \
                    case 32:                                                                            \
                        value |= OP(((int32_t *)V(vs2))[ei], ((int32_t)imm)) << (ei & 0x7);             \
                        break;                                                                          \
                    case 64:                                                                            \
                        value |= OP(((int64_t *)V(vs2))[ei], ((int64_t)imm)) << (ei & 0x7);             \
                        break;                                                                          \
                    default:                                                                            \
                        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                      \
                        break;                                                                          \
                }                                                                                       \
            }                                                                                           \
            VMOP_LOOP_SUFFIX();                                                                         \
        }                                                                                               \
    }

#define VMOP_UNSIGNED_VV(NAME, OP)                                                                     \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1)   \
    {                                                                                                  \
        const target_ulong eew = env->vsew;                                                            \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);                         \
        if(V_IDX_INVALID_EEW(vd, 8) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {                     \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                 \
        }                                                                                              \
        uint8_t mask = 0, value = 0;                                                                   \
        for(int ei = 0; ei < env->vl; ++ei) {                                                          \
            VMOP_LOOP_PREFIX();                                                                        \
            if(MS_TEST_MASK()) {                                                                       \
                switch(eew) {                                                                          \
                    case 8:                                                                            \
                        value |= OP(((uint8_t *)V(vs2))[ei], ((uint8_t *)V(vs1))[ei]) << (ei & 0x7);   \
                        break;                                                                         \
                    case 16:                                                                           \
                        value |= OP(((uint16_t *)V(vs2))[ei], ((uint16_t *)V(vs1))[ei]) << (ei & 0x7); \
                        break;                                                                         \
                    case 32:                                                                           \
                        value |= OP(((uint32_t *)V(vs2))[ei], ((uint32_t *)V(vs1))[ei]) << (ei & 0x7); \
                        break;                                                                         \
                    case 64:                                                                           \
                        value |= OP(((uint64_t *)V(vs2))[ei], ((uint64_t *)V(vs1))[ei]) << (ei & 0x7); \
                        break;                                                                         \
                    default:                                                                           \
                        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                     \
                        break;                                                                         \
                }                                                                                      \
            }                                                                                          \
            VMOP_LOOP_SUFFIX();                                                                        \
        }                                                                                              \
    }

#define VMOP_SIGNED_VV(NAME, OP)                                                                     \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1) \
    {                                                                                                \
        const target_ulong eew = env->vsew;                                                          \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);                       \
        if(V_IDX_INVALID_EEW(vd, 8) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {                   \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                               \
        }                                                                                            \
        uint8_t mask = 0, value = 0;                                                                 \
        for(int ei = 0; ei < env->vl; ++ei) {                                                        \
            VMOP_LOOP_PREFIX();                                                                      \
            if(MS_TEST_MASK()) {                                                                     \
                switch(eew) {                                                                        \
                    case 8:                                                                          \
                        value |= OP(((int8_t *)V(vs2))[ei], ((int8_t *)V(vs1))[ei]) << (ei & 0x7);   \
                        break;                                                                       \
                    case 16:                                                                         \
                        value |= OP(((int16_t *)V(vs2))[ei], ((int16_t *)V(vs1))[ei]) << (ei & 0x7); \
                        break;                                                                       \
                    case 32:                                                                         \
                        value |= OP(((int32_t *)V(vs2))[ei], ((int32_t *)V(vs1))[ei]) << (ei & 0x7); \
                        break;                                                                       \
                    case 64:                                                                         \
                        value |= OP(((int64_t *)V(vs2))[ei], ((int64_t *)V(vs1))[ei]) << (ei & 0x7); \
                        break;                                                                       \
                    default:                                                                         \
                        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                   \
                        break;                                                                       \
                }                                                                                    \
            }                                                                                        \
            VMOP_LOOP_SUFFIX();                                                                      \
        }                                                                                            \
    }

#define V3OP_SIGNED_VVX(NAME, OP)                                                                                 \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, target_long imm)           \
    {                                                                                                             \
        const target_ulong eew = env->vsew;                                                                       \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);                                    \
        if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2)) {                                                             \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                            \
        }                                                                                                         \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                           \
            TEST_MASK(ei)                                                                                         \
            switch(eew) {                                                                                         \
                case 8:                                                                                           \
                    ((int8_t *)V(vd))[ei] = OP(((int8_t *)V(vd))[ei], ((int8_t *)V(vs2))[ei], ((int8_t)imm));     \
                    break;                                                                                        \
                case 16:                                                                                          \
                    ((int16_t *)V(vd))[ei] = OP(((int16_t *)V(vd))[ei], ((int16_t *)V(vs2))[ei], ((int16_t)imm)); \
                    break;                                                                                        \
                case 32:                                                                                          \
                    ((int32_t *)V(vd))[ei] = OP(((int32_t *)V(vd))[ei], ((int32_t *)V(vs2))[ei], ((int32_t)imm)); \
                    break;                                                                                        \
                case 64:                                                                                          \
                    ((int64_t *)V(vd))[ei] = OP(((int64_t *)V(vd))[ei], ((int64_t *)V(vs2))[ei], ((int64_t)imm)); \
                    break;                                                                                        \
                default:                                                                                          \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                    \
                    break;                                                                                        \
            }                                                                                                     \
        }                                                                                                         \
    }

#define V3OP_SIGNED_VVV(NAME, OP)                                                                                          \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1)                       \
    {                                                                                                                      \
        const target_ulong eew = env->vsew;                                                                                \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);                                             \
        if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {                                                \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                                     \
        }                                                                                                                  \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                                    \
            TEST_MASK(ei)                                                                                                  \
            switch(eew) {                                                                                                  \
                case 8:                                                                                                    \
                    ((int8_t *)V(vd))[ei] = OP(((int8_t *)V(vd))[ei], ((int8_t *)V(vs2))[ei], ((int8_t *)V(vs1))[ei]);     \
                    break;                                                                                                 \
                case 16:                                                                                                   \
                    ((int16_t *)V(vd))[ei] = OP(((int16_t *)V(vd))[ei], ((int16_t *)V(vs2))[ei], ((int16_t *)V(vs1))[ei]); \
                    break;                                                                                                 \
                case 32:                                                                                                   \
                    ((int32_t *)V(vd))[ei] = OP(((int32_t *)V(vd))[ei], ((int32_t *)V(vs2))[ei], ((int32_t *)V(vs1))[ei]); \
                    break;                                                                                                 \
                case 64:                                                                                                   \
                    ((int64_t *)V(vd))[ei] = OP(((int64_t *)V(vd))[ei], ((int64_t *)V(vs2))[ei], ((int64_t *)V(vs1))[ei]); \
                    break;                                                                                                 \
                default:                                                                                                   \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                             \
                    break;                                                                                                 \
            }                                                                                                              \
        }                                                                                                                  \
    }

#define V3OP_UNSIGNED_WVX(NAME, OP)                                                                                 \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, target_ulong imm)            \
    {                                                                                                               \
        const target_ulong eew = env->vsew;                                                                         \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);                                 \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID(vs2)) {                                                 \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                              \
        }                                                                                                           \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                             \
            TEST_MASK(ei)                                                                                           \
            switch(eew) {                                                                                           \
                case 8:                                                                                             \
                    ((uint16_t *)V(vd))[ei] =                                                                       \
                        OP(((uint16_t *)V(vd))[ei], (uint16_t)((uint8_t *)V(vs2))[ei], (uint16_t)((uint8_t)imm));   \
                    break;                                                                                          \
                case 16:                                                                                            \
                    ((uint32_t *)V(vd))[ei] =                                                                       \
                        OP(((uint32_t *)V(vd))[ei], (uint32_t)((uint16_t *)V(vs2))[ei], (uint32_t)((uint16_t)imm)); \
                    break;                                                                                          \
                case 32:                                                                                            \
                    ((uint64_t *)V(vd))[ei] =                                                                       \
                        OP(((uint64_t *)V(vd))[ei], (uint64_t)((uint32_t *)V(vs2))[ei], (uint64_t)((uint32_t)imm)); \
                    break;                                                                                          \
                default:                                                                                            \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                      \
                    break;                                                                                          \
            }                                                                                                       \
        }                                                                                                           \
    }

#define V3OP_SIGNED_WVX(NAME, OP)                                                                              \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, target_ulong imm)       \
    {                                                                                                          \
        const target_ulong eew = env->vsew;                                                                    \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);                            \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID(vs2)) {                                            \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                         \
        }                                                                                                      \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                        \
            TEST_MASK(ei)                                                                                      \
            switch(eew) {                                                                                      \
                case 8:                                                                                        \
                    ((int16_t *)V(vd))[ei] =                                                                   \
                        OP(((int16_t *)V(vd))[ei], (int16_t)((int8_t *)V(vs2))[ei], (int16_t)((int8_t)imm));   \
                    break;                                                                                     \
                case 16:                                                                                       \
                    ((int32_t *)V(vd))[ei] =                                                                   \
                        OP(((int32_t *)V(vd))[ei], (int32_t)((int16_t *)V(vs2))[ei], (int32_t)((int16_t)imm)); \
                    break;                                                                                     \
                case 32:                                                                                       \
                    ((int64_t *)V(vd))[ei] =                                                                   \
                        OP(((int64_t *)V(vd))[ei], (int64_t)((int32_t *)V(vs2))[ei], (int64_t)((int32_t)imm)); \
                    break;                                                                                     \
                default:                                                                                       \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                 \
                    break;                                                                                     \
            }                                                                                                  \
        }                                                                                                      \
    }

#define V3OP_UNSIGNED_SIGNED_WVX(NAME, OP)                                                                       \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, target_ulong imm)         \
    {                                                                                                            \
        const target_ulong eew = env->vsew;                                                                      \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);                              \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID(vs2)) {                                              \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                           \
        }                                                                                                        \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                          \
            TEST_MASK(ei)                                                                                        \
            switch(eew) {                                                                                        \
                case 8:                                                                                          \
                    ((int16_t *)V(vd))[ei] =                                                                     \
                        OP(((int16_t *)V(vd))[ei], (uint16_t)((uint8_t *)V(vs2))[ei], (int16_t)((int8_t)imm));   \
                    break;                                                                                       \
                case 16:                                                                                         \
                    ((int32_t *)V(vd))[ei] =                                                                     \
                        OP(((int32_t *)V(vd))[ei], (uint32_t)((uint16_t *)V(vs2))[ei], (int32_t)((int16_t)imm)); \
                    break;                                                                                       \
                case 32:                                                                                         \
                    ((int64_t *)V(vd))[ei] =                                                                     \
                        OP(((int64_t *)V(vd))[ei], (uint64_t)((uint32_t *)V(vs2))[ei], (int64_t)((int32_t)imm)); \
                    break;                                                                                       \
                default:                                                                                         \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                   \
                    break;                                                                                       \
            }                                                                                                    \
        }                                                                                                        \
    }

#define V3OP_SIGNED_UNSIGNED_WVX(NAME, OP)                                                                       \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, target_ulong imm)         \
    {                                                                                                            \
        const target_ulong eew = env->vsew;                                                                      \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);                              \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID(vs2)) {                                              \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                           \
        }                                                                                                        \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                          \
            TEST_MASK(ei)                                                                                        \
            switch(eew) {                                                                                        \
                case 8:                                                                                          \
                    ((int16_t *)V(vd))[ei] =                                                                     \
                        OP(((int16_t *)V(vd))[ei], (int16_t)((int8_t *)V(vs2))[ei], (uint16_t)((uint8_t)imm));   \
                    break;                                                                                       \
                case 16:                                                                                         \
                    ((int32_t *)V(vd))[ei] =                                                                     \
                        OP(((int32_t *)V(vd))[ei], (int32_t)((int16_t *)V(vs2))[ei], (uint32_t)((uint16_t)imm)); \
                    break;                                                                                       \
                case 32:                                                                                         \
                    ((int64_t *)V(vd))[ei] =                                                                     \
                        OP(((int64_t *)V(vd))[ei], (int64_t)((int32_t *)V(vs2))[ei], (uint64_t)((uint32_t)imm)); \
                    break;                                                                                       \
                default:                                                                                         \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                   \
                    break;                                                                                       \
            }                                                                                                    \
        }                                                                                                        \
    }

#define V3OP_UNSIGNED_WVV(NAME, OP)                                                                                          \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1)                         \
    {                                                                                                                        \
        const target_ulong eew = env->vsew;                                                                                  \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);                                          \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {                                    \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                                       \
        }                                                                                                                    \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                                      \
            TEST_MASK(ei)                                                                                                    \
            switch(eew) {                                                                                                    \
                case 8:                                                                                                      \
                    ((uint16_t *)V(vd))[ei] =                                                                                \
                        OP(((uint16_t *)V(vd))[ei], (uint16_t)((uint8_t *)V(vs2))[ei], (uint16_t)((uint8_t *)V(vs1))[ei]);   \
                    break;                                                                                                   \
                case 16:                                                                                                     \
                    ((uint32_t *)V(vd))[ei] =                                                                                \
                        OP(((uint32_t *)V(vd))[ei], (uint32_t)((uint16_t *)V(vs2))[ei], (uint32_t)((uint16_t *)V(vs1))[ei]); \
                    break;                                                                                                   \
                case 32:                                                                                                     \
                    ((uint64_t *)V(vd))[ei] =                                                                                \
                        OP(((uint64_t *)V(vd))[ei], (uint64_t)((uint32_t *)V(vs2))[ei], (uint64_t)((uint32_t *)V(vs1))[ei]); \
                    break;                                                                                                   \
                default:                                                                                                     \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                               \
                    break;                                                                                                   \
            }                                                                                                                \
        }                                                                                                                    \
    }

#define V3OP_SIGNED_WVV(NAME, OP)                                                                                       \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1)                    \
    {                                                                                                                   \
        const target_ulong eew = env->vsew;                                                                             \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);                                     \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {                               \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                                  \
        }                                                                                                               \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                                 \
            TEST_MASK(ei)                                                                                               \
            switch(eew) {                                                                                               \
                case 8:                                                                                                 \
                    ((int16_t *)V(vd))[ei] =                                                                            \
                        OP(((int16_t *)V(vd))[ei], (int16_t)((int8_t *)V(vs2))[ei], (int16_t)((int8_t *)V(vs1))[ei]);   \
                    break;                                                                                              \
                case 16:                                                                                                \
                    ((int32_t *)V(vd))[ei] =                                                                            \
                        OP(((int32_t *)V(vd))[ei], (int32_t)((int16_t *)V(vs2))[ei], (int32_t)((int16_t *)V(vs1))[ei]); \
                    break;                                                                                              \
                case 32:                                                                                                \
                    ((int64_t *)V(vd))[ei] =                                                                            \
                        OP(((int64_t *)V(vd))[ei], (int64_t)((int32_t *)V(vs2))[ei], (int64_t)((int32_t *)V(vs1))[ei]); \
                    break;                                                                                              \
                default:                                                                                                \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                          \
                    break;                                                                                              \
            }                                                                                                           \
        }                                                                                                               \
    }

#define V3OP_UNSIGNED_SIGNED_WVV(NAME, OP)                                                                                \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1)                      \
    {                                                                                                                     \
        const target_ulong eew = env->vsew;                                                                               \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);                                       \
        if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {                                 \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                                    \
        }                                                                                                                 \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                                                   \
            TEST_MASK(ei)                                                                                                 \
            switch(eew) {                                                                                                 \
                case 8:                                                                                                   \
                    ((int16_t *)V(vd))[ei] =                                                                              \
                        OP(((int16_t *)V(vd))[ei], (uint16_t)((uint8_t *)V(vs2))[ei], (int16_t)((int8_t *)V(vs1))[ei]);   \
                    break;                                                                                                \
                case 16:                                                                                                  \
                    ((int32_t *)V(vd))[ei] =                                                                              \
                        OP(((int32_t *)V(vd))[ei], (uint32_t)((uint16_t *)V(vs2))[ei], (int32_t)((int16_t *)V(vs1))[ei]); \
                    break;                                                                                                \
                case 32:                                                                                                  \
                    ((int64_t *)V(vd))[ei] =                                                                              \
                        OP(((int64_t *)V(vd))[ei], (uint64_t)((uint32_t *)V(vs2))[ei], (int64_t)((int32_t *)V(vs1))[ei]); \
                    break;                                                                                                \
                default:                                                                                                  \
                    raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                                            \
                    break;                                                                                                \
            }                                                                                                             \
        }                                                                                                                 \
    }

#define VOP_RED_UNSIGNED_VVV(NAME, OP)                                                               \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1) \
    {                                                                                                \
        const target_ulong eew = env->vsew;                                                          \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);                       \
        if(V_IDX_INVALID(vs2)) {                                                                     \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                               \
        }                                                                                            \
        if(env->vl == 0) {                                                                           \
            return;                                                                                  \
        }                                                                                            \
        uint64_t acc = 0;                                                                            \
        switch(eew) {                                                                                \
            case 8:                                                                                  \
                acc = ((uint8_t *)V(vs1))[0];                                                        \
                break;                                                                               \
            case 16:                                                                                 \
                acc = ((uint16_t *)V(vs1))[0];                                                       \
                break;                                                                               \
            case 32:                                                                                 \
                acc = ((uint32_t *)V(vs1))[0];                                                       \
                break;                                                                               \
            case 64:                                                                                 \
                acc = ((uint64_t *)V(vs1))[0];                                                       \
                break;                                                                               \
            default:                                                                                 \
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                           \
                return;                                                                              \
        }                                                                                            \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                              \
            TEST_MASK(ei)                                                                            \
            switch(eew) {                                                                            \
                case 8:                                                                              \
                    acc = OP((uint8_t)acc, ((uint8_t *)V(vs2))[ei]);                                 \
                    break;                                                                           \
                case 16:                                                                             \
                    acc = OP((uint16_t)acc, ((uint16_t *)V(vs2))[ei]);                               \
                    break;                                                                           \
                case 32:                                                                             \
                    acc = OP((uint32_t)acc, ((uint32_t *)V(vs2))[ei]);                               \
                    break;                                                                           \
                case 64:                                                                             \
                    acc = OP((uint64_t)acc, ((uint64_t *)V(vs2))[ei]);                               \
                    break;                                                                           \
            }                                                                                        \
        }                                                                                            \
        switch(eew) {                                                                                \
            case 8:                                                                                  \
                ((uint8_t *)V(vd))[0] = acc;                                                         \
                break;                                                                               \
            case 16:                                                                                 \
                ((uint16_t *)V(vd))[0] = acc;                                                        \
                break;                                                                               \
            case 32:                                                                                 \
                ((uint32_t *)V(vd))[0] = acc;                                                        \
                break;                                                                               \
            case 64:                                                                                 \
                ((uint64_t *)V(vd))[0] = acc;                                                        \
                break;                                                                               \
        }                                                                                            \
    }

#define VOP_RED_SIGNED_VVV(NAME, OP)                                                                 \
    void glue(glue(helper_, NAME), POSTFIX)(CPUState * env, uint32_t vd, uint32_t vs2, uint32_t vs1) \
    {                                                                                                \
        const target_ulong eew = env->vsew;                                                          \
        ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);                       \
        if(V_IDX_INVALID(vs2)) {                                                                     \
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                               \
        }                                                                                            \
        if(env->vl == 0) {                                                                           \
            return;                                                                                  \
        }                                                                                            \
        int64_t acc = 0;                                                                             \
        switch(eew) {                                                                                \
            case 8:                                                                                  \
                acc = ((int8_t *)V(vs1))[0];                                                         \
                break;                                                                               \
            case 16:                                                                                 \
                acc = ((int16_t *)V(vs1))[0];                                                        \
                break;                                                                               \
            case 32:                                                                                 \
                acc = ((int32_t *)V(vs1))[0];                                                        \
                break;                                                                               \
            case 64:                                                                                 \
                acc = ((int64_t *)V(vs1))[0];                                                        \
                break;                                                                               \
            default:                                                                                 \
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);                           \
                return;                                                                              \
        }                                                                                            \
        for(int ei = env->vstart; ei < env->vl; ++ei) {                                              \
            TEST_MASK(ei)                                                                            \
            switch(eew) {                                                                            \
                case 8:                                                                              \
                    acc = OP((int8_t)acc, ((int8_t *)V(vs2))[ei]);                                   \
                    break;                                                                           \
                case 16:                                                                             \
                    acc = OP((int16_t)acc, ((int16_t *)V(vs2))[ei]);                                 \
                    break;                                                                           \
                case 32:                                                                             \
                    acc = OP((int32_t)acc, ((int32_t *)V(vs2))[ei]);                                 \
                    break;                                                                           \
                case 64:                                                                             \
                    acc = OP((int64_t)acc, ((int64_t *)V(vs2))[ei]);                                 \
                    break;                                                                           \
            }                                                                                        \
        }                                                                                            \
        switch(eew) {                                                                                \
            case 8:                                                                                  \
                ((int8_t *)V(vd))[0] = acc;                                                          \
                break;                                                                               \
            case 16:                                                                                 \
                ((int16_t *)V(vd))[0] = acc;                                                         \
                break;                                                                               \
            case 32:                                                                                 \
                ((int32_t *)V(vd))[0] = acc;                                                         \
                break;                                                                               \
            case 64:                                                                                 \
                ((int64_t *)V(vd))[0] = acc;                                                         \
                break;                                                                               \
        }                                                                                            \
    }

#define OP_ADD(A, B)   ((A) + (B))
#define OP_SUB(A, B)   ((A) - (B))
#define OP_RSUB(A, B)  (-(A) + (B))
#define OP_MUL(A, B)   ((A) * (B))
#define OP_AND(A, B)   ((A) & (B))
#define OP_OR(A, B)    ((A) | (B))
#define OP_XOR(A, B)   ((A) ^ (B))
#define OP_EQ(A, B)    ((A) == (B))
#define OP_NE(A, B)    ((A) != (B))
#define OP_LT(A, B)    ((A) < (B))
#define OP_LE(A, B)    ((A) <= (B))
#define OP_GT(A, B)    ((A) > (B))
#define OP_SHR(A, B)   ((A) >> ((B) & ((sizeof(A) << 3) - 1)))
#define OP_SHL(A, B)   ((A) << ((B) & ((sizeof(A) << 3) - 1)))
#define LBITS(A, BITS) ((A) & (((typeof(A))1 << (BITS)) - 1))
#define HBITS(A, BITS) (((A) >> (BITS)) & (((typeof(A))1 << (BITS)) - 1))

//  Karatsuba algorithm's approach for getting upper half of multiplication result
//  ab = a1*b1 * 2^N + (a0*b1 + a1*b0) * 2^(N/2) + a0*b0
//  ab >> N = a1*b1 + (a0*b1)_1 + (a1*b0)_1 + OVERFLOW((a0*b1)_0 + (a1*b0)_0 + (a0*b0)_1)
//       ab_3   |    ab_2   |    ab_1   |    ab_0
//    (a1*b1)_1 | (a1*b1)_0 |     0     |     0
//  +     0     | (a0*b1)_1 | (a0*b1)_0 |     0
//  +     0     | (a1*b0)_1 | (a1*b0)_0 |     0
//  +     0     |     0     | (a0*b0)_1 | (a0*b0)_0

//  In some cases the B operand is unsigned, so a common type is type of the B operand.

#define OP_MULHU(A, B)                                                                                            \
    ({                                                                                                            \
        typeof(B) a = A;                                                                                          \
        typeof(B) b = B;                                                                                          \
        typeof(a) a1b1 = HBITS(a, sizeof(a) << 2) * HBITS(b, sizeof(b) << 2);                                     \
        typeof(a) a1b0 = HBITS(a, sizeof(a) << 2) * LBITS(b, sizeof(b) << 2);                                     \
        typeof(a) a0b1 = LBITS(a, sizeof(a) << 2) * HBITS(b, sizeof(b) << 2);                                     \
        typeof(a) a0b0 = LBITS(a, sizeof(a) << 2) * LBITS(b, sizeof(b) << 2);                                     \
        typeof(a) ab_1 = HBITS(a0b0, sizeof(a) << 2) + LBITS(a1b0, sizeof(a) << 2) + LBITS(a0b1, sizeof(a) << 2); \
        a1b1 + HBITS(a1b0, sizeof(a) << 2) + HBITS(a0b1, sizeof(a) << 2) + HBITS(ab_1, sizeof(a) << 2);           \
    })

#define OP_NEG_MULHU(A, B)                                                                                        \
    ({                                                                                                            \
        typeof(B) a = A;                                                                                          \
        typeof(B) b = B;                                                                                          \
        typeof(a) a1b1 = HBITS(a, sizeof(a) << 2) * HBITS(b, sizeof(b) << 2);                                     \
        typeof(a) a1b0 = HBITS(a, sizeof(a) << 2) * LBITS(b, sizeof(b) << 2);                                     \
        typeof(a) a0b1 = LBITS(a, sizeof(a) << 2) * HBITS(b, sizeof(b) << 2);                                     \
        typeof(a) a0b0 = LBITS(a, sizeof(a) << 2) * LBITS(b, sizeof(b) << 2);                                     \
        typeof(a) ab_1 = HBITS(a0b0, sizeof(a) << 2) + LBITS(a1b0, sizeof(a) << 2) + LBITS(a0b1, sizeof(a) << 2); \
        typeof(a) ab_0 = LBITS(a0b0, sizeof(a) << 2);                                                             \
        ~(a1b1 + HBITS(a1b0, sizeof(a) << 2) + HBITS(a0b1, sizeof(a) << 2) + HBITS(ab_1, sizeof(a) << 2)) +       \
            (LBITS(ab_1, sizeof(a) << 2) == 0 && ab_0 == 0);                                                      \
    })

#define OP_MULH(A, B)                         \
    ({                                        \
        typeof(A) a = A;                      \
        typeof(a) b = B;                      \
        typeof(a) abs_a = (a < 0 ? -a : a);   \
        typeof(a) abs_b = (b < 0 ? -b : b);   \
        typeof(a) res;                        \
        if((a < 0) != (b < 0)) {              \
            res = OP_NEG_MULHU(abs_a, abs_b); \
        } else {                              \
            res = OP_MULHU(abs_a, abs_b);     \
        }                                     \
        res;                                  \
    })

#define OP_MULHSU(A, B)                \
    ({                                 \
        typeof(A) aa = A;              \
        typeof(aa) res;                \
        if(aa < 0) {                   \
            aa = -aa;                  \
            res = OP_NEG_MULHU(aa, B); \
        } else {                       \
            res = OP_MULHU(aa, B);     \
        }                              \
        res;                           \
    })

#define OP_MIN(A, B)     \
    ({                   \
        typeof(A) a = A; \
        typeof(a) b = B; \
        a > b ? b : a;   \
    })
#define OP_MAX(A, B)     \
    ({                   \
        typeof(A) a = A; \
        typeof(a) b = B; \
        a < b ? b : a;   \
    })

#define OP_GATHER(A, B)                                     \
    ({                                                      \
        size_t idx = B;                                     \
        idx >= env->vlmax ? 0 : ((typeof(A) *)V(vs2))[idx]; \
    })

#define OP_MACC(ACC, A, B)  ((B) * (A) + (ACC))
#define OP_NMSAC(ACC, A, B) (-((B) * (A)) + (ACC))
#define OP_MADD(ACC, A, B)  ((B) * (ACC) + (A))
#define OP_NMSUB(ACC, A, B) (-((B) * (ACC)) + (A))

#define SMAX(TYPE) (~(((TYPE)0) | (((TYPE)1) << ((sizeof(TYPE) << 3) - 1))))

#define VOP_SADDU(A, B)                                     \
    ({                                                      \
        typeof(A) a = A;                                    \
        typeof(a) b = B;                                    \
        typeof(a) ab = a + b;                               \
        bool sat = ab < a;                                  \
        env->vxsat |= sat;                                  \
        env->vcsr = (env->vcsr & ~VCSR_VXSAT) | env->vxsat; \
        sat ? ~0 : ab;                                      \
    })

#define VOP_SADD(A, B)                                      \
    ({                                                      \
        typeof(A) a = A;                                    \
        typeof(a) b = B;                                    \
        typeof(a) sat_value = SMAX(typeof(a)) + (a < 0);    \
        bool sat = (a < 0) != (b > sat_value - a);          \
        env->vxsat |= sat;                                  \
        env->vcsr = (env->vcsr & ~VCSR_VXSAT) | env->vxsat; \
        sat ? sat_value : a + b;                            \
    })

#define VOP_SSUBU(A, B)                                     \
    ({                                                      \
        typeof(A) a = A;                                    \
        typeof(a) b = B;                                    \
        bool sat = a < b;                                   \
        env->vxsat |= sat;                                  \
        env->vcsr = (env->vcsr & ~VCSR_VXSAT) | env->vxsat; \
        sat ? 0 : a - b;                                    \
    })

#define VOP_SSUB(A, B)                                      \
    ({                                                      \
        typeof(A) a = A;                                    \
        typeof(a) b = B;                                    \
        typeof(a) result = a - b;                           \
        a = SMAX(typeof(a)) + (a < 0);                      \
        bool sat = ((a ^ b) & (a ^ result)) < 0;            \
        env->vxsat |= sat;                                  \
        env->vcsr = (env->vcsr & ~VCSR_VXSAT) | env->vxsat; \
        sat ? a : result;                                   \
    })

VOP_UNSIGNED_VVX(vadd_ivi, OP_ADD)
VOP_UNSIGNED_VVX(vrsub_ivi, OP_RSUB)
VOP_UNSIGNED_VVX(vmulhu_mvx, OP_MULHU)
VOP_UNSIGNED_VVX(vminu_ivi, OP_MIN)
VOP_UNSIGNED_VVX(vmaxu_ivi, OP_MAX)
VOP_UNSIGNED_VVX(vsll_ivi, OP_SHL)
VOP_UNSIGNED_VVX(vsrl_ivi, OP_SHR)
VOP_UNSIGNED_VVX(vsaddu_ivi, VOP_SADDU)
VOP_UNSIGNED_VVX(vssubu_ivi, VOP_SSUBU)

VOP_SIGNED_VVX(vmul_mvx, OP_MUL)
VOP_SIGNED_VVX(vmulh_mvx, OP_MULH)
VOP_SIGNED_VVX(vmin_ivi, OP_MIN)
VOP_SIGNED_VVX(vmax_ivi, OP_MAX)
VOP_SIGNED_VVX(vand_ivi, OP_AND)
VOP_SIGNED_VVX(vor_ivi, OP_OR)
VOP_SIGNED_VVX(vxor_ivi, OP_XOR)
VOP_SIGNED_VVX(vsra_ivi, OP_SHR)
VOP_SIGNED_VVX(vsadd_ivi, VOP_SADD)
VOP_SIGNED_VVX(vssub_ivi, VOP_SSUB)

VOP_SIGNED_UNSIGNED_VVX(vmulhsu_mvx, OP_MULHSU)

VOP_UNSIGNED_VVV(vadd_ivv, OP_ADD)
VOP_UNSIGNED_VVV(vsub_ivv, OP_SUB)
VOP_UNSIGNED_VVV(vmulhu_mvv, OP_MULHU)
VOP_UNSIGNED_VVV(vminu_ivv, OP_MIN)
VOP_UNSIGNED_VVV(vmaxu_ivv, OP_MAX)
VOP_UNSIGNED_VVV(vrgather_ivv, OP_GATHER)
VOP_UNSIGNED_VVV(vand_ivv, OP_AND)
VOP_UNSIGNED_VVV(vor_ivv, OP_OR)
VOP_UNSIGNED_VVV(vxor_ivv, OP_XOR)
VOP_UNSIGNED_VVV(vsll_ivv, OP_SHL)
VOP_UNSIGNED_VVV(vsrl_ivv, OP_SHR)
VOP_UNSIGNED_VVV(vsaddu_ivv, VOP_SADDU)
VOP_UNSIGNED_VVV(vssubu_ivv, VOP_SSUBU)

VOP_SIGNED_VVV(vmul_mvv, OP_MUL)
VOP_SIGNED_VVV(vmulh_mvv, OP_MULH)
VOP_SIGNED_VVV(vmin_ivv, OP_MIN)
VOP_SIGNED_VVV(vmax_ivv, OP_MAX)
VOP_SIGNED_VVV(vsra_ivv, OP_SHR)
VOP_SIGNED_VVV(vsadd_ivv, VOP_SADD)
VOP_SIGNED_VVV(vssub_ivv, VOP_SSUB)

VOP_SIGNED_UNSIGNED_VVV(vmulhsu_mvv, OP_MULHSU)

VOP_UNSIGNED_WVX(vwaddu_mvx, OP_ADD)
VOP_UNSIGNED_WVX(vwsubu_mvx, OP_SUB)
VOP_UNSIGNED_WVX(vwmulu_mvx, OP_MUL)

VOP_SIGNED_WVX(vwadd_mvx, OP_ADD)
VOP_SIGNED_WVX(vwsub_mvx, OP_SUB)
VOP_SIGNED_WVX(vwmul_mvx, OP_MUL)

VOP_SIGNED_UNSIGNED_WVX(vwmulsu_mvx, OP_MUL)

VOP_UNSIGNED_WVV(vwaddu_mvv, OP_ADD)
VOP_UNSIGNED_WVV(vwsubu_mvv, OP_SUB)
VOP_UNSIGNED_WVV(vwmulu_mvv, OP_MUL)

VOP_SIGNED_WVV(vwadd_mvv, OP_ADD)
VOP_SIGNED_WVV(vwsub_mvv, OP_SUB)
VOP_SIGNED_WVV(vwmul_mvv, OP_MUL)

VOP_SIGNED_UNSIGNED_WVV(vwmulsu_mvv, OP_MUL)

VOP_UNSIGNED_WWX(vwaddu_mwx, OP_ADD)
VOP_UNSIGNED_WWX(vwsubu_mwx, OP_SUB)

VOP_SIGNED_WWX(vwadd_mwx, OP_ADD)
VOP_SIGNED_WWX(vwsub_mwx, OP_SUB)

VOP_UNSIGNED_WWV(vwaddu_mwv, OP_ADD)
VOP_UNSIGNED_WWV(vwsubu_mwv, OP_SUB)

VOP_SIGNED_WWV(vwadd_mwv, OP_ADD)
VOP_SIGNED_WWV(vwsub_mwv, OP_SUB)

VOP_UNSIGNED_VWX(vnsrl_ivi, OP_SHR)

VOP_SIGNED_VWX(vnsra_ivi, OP_SHR)

VOP_UNSIGNED_VWV(vnsrl_ivv, OP_SHR)

VOP_SIGNED_VWV(vnsra_ivv, OP_SHR)

VMOP_SIGNED_VV(vmslt_ivv, OP_LT)
VMOP_SIGNED_VV(vmsle_ivv, OP_LE)

VMOP_UNSIGNED_VV(vmseq_ivv, OP_EQ)
VMOP_UNSIGNED_VV(vmsne_ivv, OP_NE)
VMOP_UNSIGNED_VV(vmsltu_ivv, OP_LT)
VMOP_UNSIGNED_VV(vmsleu_ivv, OP_LE)

VMOP_SIGNED_VX(vmslt_ivi, OP_LT)
VMOP_SIGNED_VX(vmsle_ivi, OP_LE)
VMOP_SIGNED_VX(vmsgt_ivi, OP_GT)

VMOP_UNSIGNED_VX(vmseq_ivi, OP_EQ)
VMOP_UNSIGNED_VX(vmsne_ivi, OP_NE)
VMOP_UNSIGNED_VX(vmsltu_ivi, OP_LT)
VMOP_UNSIGNED_VX(vmsleu_ivi, OP_LE)
VMOP_UNSIGNED_VX(vmsgtu_ivi, OP_GT)

V3OP_SIGNED_VVX(vmacc_mvx, OP_MACC)
V3OP_SIGNED_VVX(vnmsac_mvx, OP_NMSAC)
V3OP_SIGNED_VVX(vmadd_mvx, OP_MADD)
V3OP_SIGNED_VVX(vnmsub_mvx, OP_NMSUB)

V3OP_SIGNED_VVV(vmacc_mvv, OP_MACC)
V3OP_SIGNED_VVV(vnmsac_mvv, OP_NMSAC)
V3OP_SIGNED_VVV(vmadd_mvv, OP_MADD)
V3OP_SIGNED_VVV(vnmsub_mvv, OP_NMSUB)

V3OP_SIGNED_WVX(vwmacc_mvx, OP_MACC)

V3OP_UNSIGNED_WVX(vwmaccu_mvx, OP_MACC)

V3OP_SIGNED_WVV(vwmacc_mvv, OP_MACC)

V3OP_UNSIGNED_WVV(vwmaccu_mvv, OP_MACC)

V3OP_UNSIGNED_SIGNED_WVX(vwmaccsu_mvx, OP_MACC)

V3OP_SIGNED_UNSIGNED_WVX(vwmaccus_mvx, OP_MACC)

V3OP_UNSIGNED_SIGNED_WVV(vwmaccsu_mvv, OP_MACC)

VOP_RED_UNSIGNED_VVV(vredmaxu_vs, OP_MAX)
VOP_RED_UNSIGNED_VVV(vredminu_vs, OP_MIN)

VOP_RED_SIGNED_VVV(vredsum_vs, OP_ADD)
VOP_RED_SIGNED_VVV(vredmax_vs, OP_MAX)
VOP_RED_SIGNED_VVV(vredmin_vs, OP_MIN)
VOP_RED_SIGNED_VVV(vredand_vs, OP_AND)
VOP_RED_SIGNED_VVV(vredor_vs, OP_OR)
VOP_RED_SIGNED_VVV(vredxor_vs, OP_XOR)

void glue(helper_vwredsumu_ivv, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, int32_t vs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);
    if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID(vs2) || V_IDX_INVALID_EEW(vs1, eew << 1)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    if(env->vl == 0) {
        return;
    }
    uint64_t acc = 0;
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                acc += ((uint8_t *)V(vs2))[ei];
                break;
            case 16:
                acc += ((uint16_t *)V(vs2))[ei];
                break;
            case 32:
                acc += ((uint32_t *)V(vs2))[ei];
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
    switch(eew) {
        case 8:
            ((uint16_t *)V(vd))[0] = acc + ((uint16_t *)V(vs1))[0];
            break;
        case 16:
            ((uint32_t *)V(vd))[0] = acc + ((uint32_t *)V(vs1))[0];
            break;
        case 32:
            ((uint64_t *)V(vd))[0] = acc + ((uint64_t *)V(vs1))[0];
            break;
        default:
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
            break;
    }
}

void glue(helper_vwredsum_ivv, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, int32_t vs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);
    if(V_IDX_INVALID_EEW(vd, eew << 1) || V_IDX_INVALID(vs2) || V_IDX_INVALID_EEW(vs1, eew << 1)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    if(env->vl == 0) {
        return;
    }
    int64_t acc = 0;
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                acc += ((int8_t *)V(vs2))[ei];
                break;
            case 16:
                acc += ((int16_t *)V(vs2))[ei];
                break;
            case 32:
                acc += ((int32_t *)V(vs2))[ei];
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
    switch(eew) {
        case 8:
            ((int16_t *)V(vd))[0] = acc + ((int16_t *)V(vs1))[0];
            break;
        case 16:
            ((int32_t *)V(vd))[0] = acc + ((int32_t *)V(vs1))[0];
            break;
        case 32:
            ((int64_t *)V(vd))[0] = acc + ((int64_t *)V(vs1))[0];
            break;
        default:
            raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
            break;
    }
}

void glue(helper_vnclipu_ivv, POSTFIX)(CPUState *env, uint32_t vd, uint32_t vs2, uint32_t vs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID_EEW(vs2, eew << 1) || V_IDX_INVALID(vs1)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    const uint16_t v1_mask = (eew << 1) - 1;
    const uint8_t rm = env->vxrm & 0b11;
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                ((uint8_t *)V(vd))[ei] = clipto_u8(roundoff_u16(((uint16_t *)V(vs2))[ei], ((uint8_t *)V(vs1))[ei] & v1_mask, rm));
                break;
            case 16:
                ((uint16_t *)V(vd))[ei] =
                    clipto_u16(roundoff_u32(((uint32_t *)V(vs2))[ei], ((uint16_t *)V(vs1))[ei] & v1_mask, rm));
                break;
            case 32:
                ((uint32_t *)V(vd))[ei] =
                    clipto_u32(roundoff_u64(((uint64_t *)V(vs2))[ei], ((uint32_t *)V(vs1))[ei] & v1_mask, rm));
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vnclipu_ivi, POSTFIX)(CPUState *env, uint32_t vd, uint32_t vs2, target_ulong rs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID_EEW(vs2, eew << 1)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    const uint16_t shift = rs1 & ((eew << 1) - 1);
    const uint8_t rm = env->vxrm & 0b11;
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                ((uint8_t *)V(vd))[ei] = clipto_u8(roundoff_u16(((uint16_t *)V(vs2))[ei], shift, rm));
                break;
            case 16:
                ((uint16_t *)V(vd))[ei] = clipto_u16(roundoff_u32(((uint32_t *)V(vs2))[ei], shift, rm));
                break;
            case 32:
                ((uint32_t *)V(vd))[ei] = clipto_u32(roundoff_u64(((uint64_t *)V(vs2))[ei], shift, rm));
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vnclip_ivv, POSTFIX)(CPUState *env, uint32_t vd, uint32_t vs2, uint32_t vs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID_EEW(vs2, eew << 1) || V_IDX_INVALID(vs1)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    const uint16_t v1_mask = (eew << 1) - 1;
    const uint8_t rm = env->vxrm & 0b11;
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                ((int8_t *)V(vd))[ei] = clipto_i8(roundoff_i16(((int16_t *)V(vs2))[ei], ((uint8_t *)V(vs1))[ei] & v1_mask, rm));
                break;
            case 16:
                ((int16_t *)V(vd))[ei] =
                    clipto_i16(roundoff_i32(((int32_t *)V(vs2))[ei], ((uint16_t *)V(vs1))[ei] & v1_mask, rm));
                break;
            case 32:
                ((int32_t *)V(vd))[ei] =
                    clipto_i32(roundoff_i64(((int64_t *)V(vs2))[ei], ((uint32_t *)V(vs1))[ei] & v1_mask, rm));
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vnclip_ivi, POSTFIX)(CPUState *env, uint32_t vd, uint32_t vs2, target_ulong rs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew << 1);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID_EEW(vs2, eew << 1)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    const uint16_t shift = rs1 & ((eew << 1) - 1);
    const uint8_t rm = env->vxrm & 0b11;
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                ((int8_t *)V(vd))[ei] = clipto_i8(roundoff_i16(((int16_t *)V(vs2))[ei], shift, rm));
                break;
            case 16:
                ((int16_t *)V(vd))[ei] = clipto_i16(roundoff_i32(((int32_t *)V(vs2))[ei], shift, rm));
                break;
            case 32:
                ((int32_t *)V(vd))[ei] = clipto_i32(roundoff_i64(((int64_t *)V(vs2))[ei], shift, rm));
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vslideup_ivi, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, target_ulong rs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    const int start = rs1 > env->vlmax ? env->vlmax : (rs1 > env->vstart ? rs1 : env->vstart);
    for(int ei = start; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                ((int8_t *)V(vd))[ei] = ((int8_t *)V(vs2))[ei - rs1];
                break;
            case 16:
                ((int16_t *)V(vd))[ei] = ((int16_t *)V(vs2))[ei - rs1];
                break;
            case 32:
                ((int32_t *)V(vd))[ei] = ((int32_t *)V(vs2))[ei - rs1];
                break;
            case 64:
                ((int64_t *)V(vd))[ei] = ((int64_t *)V(vs2))[ei - rs1];
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vslidedown_ivi, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, target_ulong rs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    const int src_max = rs1 > env->vlmax ? 0 : (env->vl < env->vlmax - rs1 ? env->vl : env->vlmax - rs1);
    int ei = env->vstart;
    for(; ei < src_max; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                ((int8_t *)V(vd))[ei] = ((int8_t *)V(vs2))[ei + rs1];
                break;
            case 16:
                ((int16_t *)V(vd))[ei] = ((int16_t *)V(vs2))[ei + rs1];
                break;
            case 32:
                ((int32_t *)V(vd))[ei] = ((int32_t *)V(vs2))[ei + rs1];
                break;
            case 64:
                ((int64_t *)V(vd))[ei] = ((int64_t *)V(vs2))[ei + rs1];
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
    for(; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                ((int8_t *)V(vd))[ei] = 0;
                break;
            case 16:
                ((int16_t *)V(vd))[ei] = 0;
                break;
            case 32:
                ((int32_t *)V(vd))[ei] = 0;
                break;
            case 64:
                ((int64_t *)V(vd))[ei] = 0;
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vslide1up, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, target_long rs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
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
            case 8:
                ((int8_t *)V(vd))[0] = rs1;
                break;
            case 16:
                ((int16_t *)V(vd))[0] = rs1;
                break;
            case 32:
                ((int32_t *)V(vd))[0] = rs1;
                break;
            case 64:
                ((int64_t *)V(vd))[0] = rs1;
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
    for(int ei = (env->vstart ? env->vstart : 1); ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                ((int8_t *)V(vd))[ei] = ((int8_t *)V(vs2))[ei - 1];
                break;
            case 16:
                ((int16_t *)V(vd))[ei] = ((int16_t *)V(vs2))[ei - 1];
                break;
            case 32:
                ((int32_t *)V(vd))[ei] = ((int32_t *)V(vs2))[ei - 1];
                break;
            case 64:
                ((int64_t *)V(vd))[ei] = ((int64_t *)V(vs2))[ei - 1];
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vslide1down, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, target_long rs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
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
            case 8:
                ((int8_t *)V(vd))[ei] = ((int8_t *)V(vs2))[ei + 1];
                break;
            case 16:
                ((int16_t *)V(vd))[ei] = ((int16_t *)V(vs2))[ei + 1];
                break;
            case 32:
                ((int32_t *)V(vd))[ei] = ((int32_t *)V(vs2))[ei + 1];
                break;
            case 64:
                ((int64_t *)V(vd))[ei] = ((int64_t *)V(vs2))[ei + 1];
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
            case 8:
                ((int8_t *)V(vd))[src_max] = rs1;
                break;
            case 16:
                ((int16_t *)V(vd))[src_max] = rs1;
                break;
            case 32:
                ((int32_t *)V(vd))[src_max] = rs1;
                break;
            case 64:
                ((int64_t *)V(vd))[src_max] = rs1;
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
#ifdef MASKED
    }
#endif
}

void glue(helper_vrgather_ivi, POSTFIX)(CPUState *env, uint32_t vd, uint32_t vs2, target_long imm)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                ((int8_t *)V(vd))[ei] = imm >= env->vlmax ? 0 : ((int8_t *)V(vs2))[imm];
                break;
            case 16:
                ((int16_t *)V(vd))[ei] = imm >= env->vlmax ? 0 : ((int16_t *)V(vs2))[imm];
                break;
            case 32:
                ((int32_t *)V(vd))[ei] = imm >= env->vlmax ? 0 : ((int32_t *)V(vs2))[imm];
                break;
            case 64:
                ((int64_t *)V(vd))[ei] = imm >= env->vlmax ? 0 : ((int64_t *)V(vs2))[imm];
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vrgatherei16_ivv, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, int32_t vs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2) || V_IDX_INVALID_EEW(vs1, 16)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        uint16_t idx = ((uint16_t *)V(vs1))[ei];
        switch(eew) {
            case 8:
                ((int8_t *)V(vd))[ei] = idx >= env->vlmax ? 0 : ((int8_t *)V(vs2))[idx];
                break;
            case 16:
                ((int16_t *)V(vd))[ei] = idx >= env->vlmax ? 0 : ((int16_t *)V(vs2))[idx];
                break;
            case 32:
                ((int32_t *)V(vd))[ei] = idx >= env->vlmax ? 0 : ((int32_t *)V(vs2))[idx];
                break;
            case 64:
                ((int64_t *)V(vd))[ei] = idx >= env->vlmax ? 0 : ((int64_t *)V(vs2))[idx];
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vzext_vf2, POSTFIX)(CPUState *env, uint32_t vd, uint32_t vs2)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(eew < 16 || V_IDX_INVALID(vd) || V_IDX_INVALID_EMUL(vs2, eew >> 1)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }

    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 16:
                ((uint16_t *)V(vd))[ei] = ((uint8_t *)V(vs2))[ei];
                break;
            case 32:
                ((uint32_t *)V(vd))[ei] = ((uint16_t *)V(vs2))[ei];
                break;
            case 64:
                ((uint64_t *)V(vd))[ei] = ((uint32_t *)V(vs2))[ei];
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vsext_vf2, POSTFIX)(CPUState *env, uint32_t vd, uint32_t vs2)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(eew < 16 || V_IDX_INVALID(vd) || V_IDX_INVALID_EMUL(vs2, eew >> 1)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }

    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 16:
                ((int16_t *)V(vd))[ei] = ((int8_t *)V(vs2))[ei];
                break;
            case 32:
                ((int32_t *)V(vd))[ei] = ((int16_t *)V(vs2))[ei];
                break;
            case 64:
                ((int64_t *)V(vd))[ei] = ((int32_t *)V(vs2))[ei];
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vzext_vf4, POSTFIX)(CPUState *env, uint32_t vd, uint32_t vs2)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(eew < 32 || V_IDX_INVALID(vd) || V_IDX_INVALID_EMUL(vs2, eew >> 2)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }

    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 32:
                ((uint32_t *)V(vd))[ei] = ((uint8_t *)V(vs2))[ei];
                break;
            case 64:
                ((uint64_t *)V(vd))[ei] = ((uint16_t *)V(vs2))[ei];
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vsext_vf4, POSTFIX)(CPUState *env, uint32_t vd, uint32_t vs2)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(eew < 32 || V_IDX_INVALID(vd) || V_IDX_INVALID_EMUL(vs2, eew >> 2)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }

    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 32:
                ((int32_t *)V(vd))[ei] = ((int8_t *)V(vs2))[ei];
                break;
            case 64:
                ((int64_t *)V(vd))[ei] = ((int16_t *)V(vs2))[ei];
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vzext_vf8, POSTFIX)(CPUState *env, uint32_t vd, uint32_t vs2)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(eew < 64 || V_IDX_INVALID(vd) || V_IDX_INVALID_EMUL(vs2, eew >> 3)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }

    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 64:
                ((uint64_t *)V(vd))[ei] = ((uint8_t *)V(vs2))[ei];
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vsext_vf8, POSTFIX)(CPUState *env, uint32_t vd, uint32_t vs2)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(eew < 64 || V_IDX_INVALID(vd) || V_IDX_INVALID_EMUL(vs2, eew >> 3)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }

    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 64:
                ((int64_t *)V(vd))[ei] = ((int8_t *)V(vs2))[ei];
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vdivu_mvv, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, int32_t vs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                ((uint8_t *)V(vd))[ei] = divu_8(((uint8_t *)V(vs2))[ei], ((uint8_t *)V(vs1))[ei]);
                break;
            case 16:
                ((uint16_t *)V(vd))[ei] = divu_16(((uint16_t *)V(vs2))[ei], ((uint16_t *)V(vs1))[ei]);
                break;
            case 32:
                ((uint32_t *)V(vd))[ei] = divu_32(((uint32_t *)V(vs2))[ei], ((uint32_t *)V(vs1))[ei]);
                break;
            case 64:
                ((uint64_t *)V(vd))[ei] = divu_64(((uint64_t *)V(vs2))[ei], ((uint64_t *)V(vs1))[ei]);
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vdiv_mvv, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, int32_t vs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                ((int8_t *)V(vd))[ei] = div_8(((int8_t *)V(vs2))[ei], ((int8_t *)V(vs1))[ei]);
                break;
            case 16:
                ((int16_t *)V(vd))[ei] = div_16(((int16_t *)V(vs2))[ei], ((int16_t *)V(vs1))[ei]);
                break;
            case 32:
                ((int32_t *)V(vd))[ei] = div_32(((int32_t *)V(vs2))[ei], ((int32_t *)V(vs1))[ei]);
                break;
            case 64:
                ((int64_t *)V(vd))[ei] = div_64(((int64_t *)V(vs2))[ei], ((int64_t *)V(vs1))[ei]);
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vremu_mvv, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, int32_t vs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                ((uint8_t *)V(vd))[ei] = remu_8(((uint8_t *)V(vs2))[ei], ((uint8_t *)V(vs1))[ei]);
                break;
            case 16:
                ((uint16_t *)V(vd))[ei] = remu_16(((uint16_t *)V(vs2))[ei], ((uint16_t *)V(vs1))[ei]);
                break;
            case 32:
                ((uint32_t *)V(vd))[ei] = remu_32(((uint32_t *)V(vs2))[ei], ((uint32_t *)V(vs1))[ei]);
                break;
            case 64:
                ((uint64_t *)V(vd))[ei] = remu_64(((uint64_t *)V(vs2))[ei], ((uint64_t *)V(vs1))[ei]);
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vrem_mvv, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, int32_t vs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                ((int8_t *)V(vd))[ei] = rem_8(((int8_t *)V(vs2))[ei], ((int8_t *)V(vs1))[ei]);
                break;
            case 16:
                ((int16_t *)V(vd))[ei] = rem_16(((int16_t *)V(vs2))[ei], ((int16_t *)V(vs1))[ei]);
                break;
            case 32:
                ((int32_t *)V(vd))[ei] = rem_32(((int32_t *)V(vs2))[ei], ((int32_t *)V(vs1))[ei]);
                break;
            case 64:
                ((int64_t *)V(vd))[ei] = rem_64(((int64_t *)V(vs2))[ei], ((int64_t *)V(vs1))[ei]);
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vdivu_mvx, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, target_long rs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                ((uint8_t *)V(vd))[ei] = divu_8(((uint8_t *)V(vs2))[ei], (int8_t)rs1);
                break;
            case 16:
                ((uint16_t *)V(vd))[ei] = divu_16(((uint16_t *)V(vs2))[ei], (int16_t)rs1);
                break;
            case 32:
                ((uint32_t *)V(vd))[ei] = divu_32(((uint32_t *)V(vs2))[ei], (int32_t)rs1);
                break;
            case 64:
                ((uint64_t *)V(vd))[ei] = divu_64(((uint64_t *)V(vs2))[ei], (int64_t)rs1);
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vdiv_mvx, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, target_long rs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                ((int8_t *)V(vd))[ei] = div_8(((int8_t *)V(vs2))[ei], rs1);
                break;
            case 16:
                ((int16_t *)V(vd))[ei] = div_16(((int16_t *)V(vs2))[ei], rs1);
                break;
            case 32:
                ((int32_t *)V(vd))[ei] = div_32(((int32_t *)V(vs2))[ei], rs1);
                break;
            case 64:
                ((int64_t *)V(vd))[ei] = div_64(((int64_t *)V(vs2))[ei], rs1);
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vremu_mvx, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, target_long rs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                ((uint8_t *)V(vd))[ei] = remu_8(((uint8_t *)V(vs2))[ei], (int8_t)rs1);
                break;
            case 16:
                ((uint16_t *)V(vd))[ei] = remu_16(((uint16_t *)V(vs2))[ei], (int16_t)rs1);
                break;
            case 32:
                ((uint32_t *)V(vd))[ei] = remu_32(((uint32_t *)V(vs2))[ei], (int32_t)rs1);
                break;
            case 64:
                ((uint64_t *)V(vd))[ei] = remu_64(((uint64_t *)V(vs2))[ei], (int64_t)rs1);
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vrem_mvx, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, target_long rs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                ((int8_t *)V(vd))[ei] = rem_8(((int8_t *)V(vs2))[ei], rs1);
                break;
            case 16:
                ((int16_t *)V(vd))[ei] = rem_16(((int16_t *)V(vs2))[ei], rs1);
                break;
            case 32:
                ((int32_t *)V(vd))[ei] = rem_32(((int32_t *)V(vs2))[ei], rs1);
                break;
            case 64:
                ((int64_t *)V(vd))[ei] = rem_64(((int64_t *)V(vs2))[ei], rs1);
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vaadd_mvv, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, int32_t vs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    const uint8_t rm = env->vxrm & 0b11;
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8: {
                int16_t a = ((int8_t *)V(vs2))[ei];
                int16_t b = ((int8_t *)V(vs1))[ei];
                ((int8_t *)V(vd))[ei] = roundoff_i16(a + b, 1, rm);
                break;
            }
            case 16: {
                int32_t a = ((int16_t *)V(vs2))[ei];
                int32_t b = ((int16_t *)V(vs1))[ei];
                ((int16_t *)V(vd))[ei] = roundoff_i32(a + b, 1, rm);
                break;
            }
            case 32: {
                int64_t a = ((int32_t *)V(vs2))[ei];
                int64_t b = ((int32_t *)V(vs1))[ei];
                ((int32_t *)V(vd))[ei] = roundoff_i64(a + b, 1, rm);
                break;
            }
            case 64: {
                __int128_t a = ((int64_t *)V(vs2))[ei];
                __int128_t b = ((int64_t *)V(vs1))[ei];
                ((int64_t *)V(vd))[ei] = roundoff_i128(a + b, 1, rm);
                break;
            }
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vaadd_mvx, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, target_long rs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    const uint8_t rm = env->vxrm & 0b11;
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8: {
                int16_t a = ((int8_t *)V(vs2))[ei];
                ((int8_t *)V(vd))[ei] = roundoff_i16(a + (int16_t)((int8_t)(rs1 & UINT8_MAX)), 1, rm);
                break;
            }
            case 16: {
                int32_t a = ((int16_t *)V(vs2))[ei];
                ((int16_t *)V(vd))[ei] = roundoff_i32(a + (int32_t)((int16_t)(rs1 & UINT16_MAX)), 1, rm);
                break;
            }
            case 32: {
                int64_t a = ((int32_t *)V(vs2))[ei];
                ((int32_t *)V(vd))[ei] = roundoff_i64(a + (int64_t)((int32_t)(rs1 & UINT32_MAX)), 1, rm);
                break;
            }
            case 64: {
                __int128_t a = ((int64_t *)V(vs2))[ei];
                ((int64_t *)V(vd))[ei] = roundoff_i128(a + (__int128_t)((int64_t)(rs1 & UINT64_MAX)), 1, rm);
                break;
            }
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vaaddu_mvv, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, int32_t vs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    const uint8_t rm = env->vxrm & 0b11;
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8: {
                uint16_t a = ((uint8_t *)V(vs2))[ei];
                uint16_t b = ((uint8_t *)V(vs1))[ei];
                ((uint8_t *)V(vd))[ei] = roundoff_u16(a + b, 1, rm);
                break;
            }
            case 16: {
                uint32_t a = ((uint16_t *)V(vs2))[ei];
                uint32_t b = ((uint16_t *)V(vs1))[ei];
                ((uint16_t *)V(vd))[ei] = roundoff_u32(a + b, 1, rm);
                break;
            }
            case 32: {
                uint64_t a = ((uint32_t *)V(vs2))[ei];
                uint64_t b = ((uint32_t *)V(vs1))[ei];
                ((uint32_t *)V(vd))[ei] = roundoff_u64(a + b, 1, rm);
                break;
            }
            case 64: {
                __uint128_t a = ((uint64_t *)V(vs2))[ei];
                __uint128_t b = ((uint64_t *)V(vs1))[ei];
                ((uint64_t *)V(vd))[ei] = roundoff_u128(a + b, 1, rm);
                break;
            }
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vaaddu_mvx, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, target_long rs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    const uint8_t rm = env->vxrm & 0b11;
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8: {
                uint16_t a = ((uint8_t *)V(vs2))[ei];
                ((uint8_t *)V(vd))[ei] = roundoff_u16(a + (uint16_t)((uint8_t)(rs1 & UINT8_MAX)), 1, rm);
                break;
            }
            case 16: {
                uint32_t a = ((uint16_t *)V(vs2))[ei];
                ((uint16_t *)V(vd))[ei] = roundoff_u32(a + (uint32_t)((uint16_t)(rs1 & UINT16_MAX)), 1, rm);
                break;
            }
            case 32: {
                uint64_t a = ((uint32_t *)V(vs2))[ei];
                ((uint32_t *)V(vd))[ei] = roundoff_u64(a + (uint64_t)((uint32_t)(rs1 & UINT32_MAX)), 1, rm);
                break;
            }
            case 64: {
                __uint128_t a = ((uint64_t *)V(vs2))[ei];
                ((uint64_t *)V(vd))[ei] = roundoff_u128(a + (__uint128_t)((uint64_t)(rs1 & UINT64_MAX)), 1, rm);
                break;
            }
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vasub_mvv, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, int32_t vs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    const uint8_t rm = env->vxrm & 0b11;
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8: {
                int16_t a = ((int8_t *)V(vs2))[ei];
                int16_t b = ((int8_t *)V(vs1))[ei];
                ((int8_t *)V(vd))[ei] = roundoff_i16(a - b, 1, rm);
                break;
            }
            case 16: {
                int32_t a = ((int16_t *)V(vs2))[ei];
                int32_t b = ((int16_t *)V(vs1))[ei];
                ((int16_t *)V(vd))[ei] = roundoff_i32(a - b, 1, rm);
                break;
            }
            case 32: {
                int64_t a = ((int32_t *)V(vs2))[ei];
                int64_t b = ((int32_t *)V(vs1))[ei];
                ((int32_t *)V(vd))[ei] = roundoff_i64(a - b, 1, rm);
                break;
            }
            case 64: {
                __int128_t a = ((int64_t *)V(vs2))[ei];
                __int128_t b = ((int64_t *)V(vs1))[ei];
                ((int64_t *)V(vd))[ei] = roundoff_i128(a - b, 1, rm);
                break;
            }
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vasub_mvx, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, target_long rs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    const uint8_t rm = env->vxrm & 0b11;
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8: {
                int16_t a = ((int8_t *)V(vs2))[ei];
                ((int8_t *)V(vd))[ei] = roundoff_i16(a - (int16_t)((int8_t)(rs1 & UINT8_MAX)), 1, rm);
                break;
            }
            case 16: {
                int32_t a = ((int16_t *)V(vs2))[ei];
                ((int16_t *)V(vd))[ei] = roundoff_i32(a - (int32_t)((int16_t)(rs1 & UINT16_MAX)), 1, rm);
                break;
            }
            case 32: {
                int64_t a = ((int32_t *)V(vs2))[ei];
                ((int32_t *)V(vd))[ei] = roundoff_i64(a - (int64_t)((int32_t)(rs1 & UINT32_MAX)), 1, rm);
                break;
            }
            case 64: {
                __int128_t a = ((int64_t *)V(vs2))[ei];
                ((int64_t *)V(vd))[ei] = roundoff_i128(a - (__int128_t)((int64_t)(rs1 & UINT64_MAX)), 1, rm);
                break;
            }
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vasubu_mvv, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, int32_t vs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    const uint8_t rm = env->vxrm & 0b11;
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8: {
                uint16_t a = ((uint8_t *)V(vs2))[ei];
                uint16_t b = ((uint8_t *)V(vs1))[ei];
                ((uint8_t *)V(vd))[ei] = roundoff_u16(a - b, 1, rm);
                break;
            }
            case 16: {
                uint32_t a = ((uint16_t *)V(vs2))[ei];
                uint32_t b = ((uint16_t *)V(vs1))[ei];
                ((uint16_t *)V(vd))[ei] = roundoff_u32(a - b, 1, rm);
                break;
            }
            case 32: {
                uint64_t a = ((uint32_t *)V(vs2))[ei];
                uint64_t b = ((uint32_t *)V(vs1))[ei];
                ((uint32_t *)V(vd))[ei] = roundoff_u64(a - b, 1, rm);
                break;
            }
            case 64: {
                __uint128_t a = ((uint64_t *)V(vs2))[ei];
                __uint128_t b = ((uint64_t *)V(vs1))[ei];
                ((uint64_t *)V(vd))[ei] = roundoff_u128(a - b, 1, rm);
                break;
            }
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vasubu_mvx, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, target_long rs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    const uint8_t rm = env->vxrm & 0b11;
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8: {
                uint16_t a = ((uint8_t *)V(vs2))[ei];
                ((uint8_t *)V(vd))[ei] = roundoff_u16(a - (uint16_t)((uint8_t)(rs1 & UINT8_MAX)), 1, rm);
                break;
            }
            case 16: {
                uint32_t a = ((uint16_t *)V(vs2))[ei];
                ((uint16_t *)V(vd))[ei] = roundoff_u32(a - (uint32_t)((uint16_t)(rs1 & UINT16_MAX)), 1, rm);
                break;
            }
            case 32: {
                uint64_t a = ((uint32_t *)V(vs2))[ei];
                ((uint32_t *)V(vd))[ei] = roundoff_u64(a - (uint64_t)((uint32_t)(rs1 & UINT32_MAX)), 1, rm);
                break;
            }
            case 64: {
                __uint128_t a = ((uint64_t *)V(vs2))[ei];
                ((uint64_t *)V(vd))[ei] = roundoff_u128(a - (__uint128_t)((uint64_t)(rs1 & UINT64_MAX)), 1, rm);
                break;
            }
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vsmul_ivv, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, int32_t vs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if((eew == 64 && !riscv_has_ext(env, RISCV_FEATURE_RVV)) || V_IDX_INVALID(vd) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    const uint8_t rm = env->vxrm & 0b11;
    const uint16_t shift = eew - 1;
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8: {
                int16_t a = ((int8_t *)V(vs2))[ei];
                int16_t b = ((int8_t *)V(vs1))[ei];
                ((int8_t *)V(vd))[ei] = clipto_i8(roundoff_i16(a * b, shift, rm));
                break;
            }
            case 16: {
                int32_t a = ((int16_t *)V(vs2))[ei];
                int32_t b = ((int16_t *)V(vs1))[ei];
                ((int16_t *)V(vd))[ei] = clipto_i16(roundoff_i32(a * b, shift, rm));
                break;
            }
            case 32: {
                int64_t a = ((int32_t *)V(vs2))[ei];
                int64_t b = ((int32_t *)V(vs1))[ei];
                ((int32_t *)V(vd))[ei] = clipto_i32(roundoff_i64(a * b, shift, rm));
                break;
            }
            case 64: {
                __int128_t a = ((int64_t *)V(vs2))[ei];
                __int128_t b = ((int64_t *)V(vs1))[ei];
                ((int64_t *)V(vd))[ei] = clipto_i64(roundoff_i128(a * b, shift, rm));
                break;
            }
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vsmul_ivx, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, target_long rs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if((eew == 64 && !riscv_has_ext(env, RISCV_FEATURE_RVV)) || V_IDX_INVALID(vd) || V_IDX_INVALID(vs2)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    const uint8_t rm = env->vxrm & 0b11;
    const uint16_t shift = eew - 1;
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8: {
                int16_t a = ((int8_t *)V(vs2))[ei];
                ((int8_t *)V(vd))[ei] = clipto_i8(roundoff_i16(a * (int16_t)((int8_t)(rs1 & UINT8_MAX)), shift, rm));
                break;
            }
            case 16: {
                int32_t a = ((int16_t *)V(vs2))[ei];
                ((int16_t *)V(vd))[ei] = clipto_i16(roundoff_i32(a * (int32_t)((int16_t)(rs1 & UINT16_MAX)), shift, rm));
                break;
            }
            case 32: {
                int64_t a = ((int32_t *)V(vs2))[ei];
                ((int32_t *)V(vd))[ei] = clipto_i32(roundoff_i64(a * (int64_t)((int32_t)(rs1 & UINT32_MAX)), shift, rm));
                break;
            }
            case 64: {
                __int128_t a = ((int64_t *)V(vs2))[ei];
                ((int64_t *)V(vd))[ei] = clipto_i64(roundoff_i128(a * (__int128_t)((int64_t)(rs1 & UINT64_MAX)), shift, rm));
                break;
            }
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vssrl_ivv, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, int32_t vs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    const uint8_t rm = env->vxrm & 0b11;
    const uint16_t mask = eew - 1;
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                ((uint8_t *)V(vd))[ei] = roundoff_u8(((uint8_t *)V(vs2))[ei], ((uint8_t *)V(vs1))[ei] & mask, rm);
                break;
            case 16:
                ((uint16_t *)V(vd))[ei] = roundoff_u16(((uint16_t *)V(vs2))[ei], ((uint16_t *)V(vs1))[ei] & mask, rm);
                break;
            case 32:
                ((uint32_t *)V(vd))[ei] = roundoff_u32(((uint32_t *)V(vs2))[ei], ((uint32_t *)V(vs1))[ei] & mask, rm);
                break;
            case 64:
                ((uint64_t *)V(vd))[ei] = roundoff_u64(((uint64_t *)V(vs2))[ei], ((uint64_t *)V(vs1))[ei] & mask, rm);
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vssrl_ivi, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, target_ulong rs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    const uint8_t rm = env->vxrm & 0b11;
    const uint16_t shift = rs1 & (eew - 1);
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                ((uint8_t *)V(vd))[ei] = roundoff_u8(((uint8_t *)V(vs2))[ei], shift, rm);
                break;
            case 16:
                ((uint16_t *)V(vd))[ei] = roundoff_u16(((uint16_t *)V(vs2))[ei], shift, rm);
                break;
            case 32:
                ((uint32_t *)V(vd))[ei] = roundoff_u32(((uint32_t *)V(vs2))[ei], shift, rm);
                break;
            case 64:
                ((uint64_t *)V(vd))[ei] = roundoff_u64(((uint64_t *)V(vs2))[ei], shift, rm);
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vssra_ivv, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, int32_t vs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2) || V_IDX_INVALID(vs1)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    const uint8_t rm = env->vxrm & 0b11;
    const uint16_t mask = eew - 1;
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                ((int8_t *)V(vd))[ei] = roundoff_i8(((int8_t *)V(vs2))[ei], ((int8_t *)V(vs1))[ei] & mask, rm);
                break;
            case 16:
                ((int16_t *)V(vd))[ei] = roundoff_i16(((int16_t *)V(vs2))[ei], ((int16_t *)V(vs1))[ei] & mask, rm);
                break;
            case 32:
                ((int32_t *)V(vd))[ei] = roundoff_i32(((int32_t *)V(vs2))[ei], ((int32_t *)V(vs1))[ei] & mask, rm);
                break;
            case 64:
                ((int64_t *)V(vd))[ei] = roundoff_i64(((int64_t *)V(vs2))[ei], ((int64_t *)V(vs1))[ei] & mask, rm);
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_vssra_ivi, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2, target_ulong rs1)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || V_IDX_INVALID(vs2)) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    const uint8_t rm = env->vxrm & 0b11;
    const uint16_t shift = rs1 & (eew - 1);
    for(int ei = env->vstart; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                ((int8_t *)V(vd))[ei] = roundoff_i8(((int8_t *)V(vs2))[ei], shift, rm);
                break;
            case 16:
                ((int16_t *)V(vd))[ei] = roundoff_i16(((int16_t *)V(vs2))[ei], shift, rm);
                break;
            case 32:
                ((int32_t *)V(vd))[ei] = roundoff_i32(((int32_t *)V(vs2))[ei], shift, rm);
                break;
            case 64:
                ((int64_t *)V(vd))[ei] = roundoff_i64(((int64_t *)V(vs2))[ei], shift, rm);
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

void glue(helper_viota, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || env->vstart) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    uint64_t cnt = 0;
    for(int ei = 0; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                ((uint8_t *)V(vd))[ei] = cnt;
                break;
            case 16:
                ((uint16_t *)V(vd))[ei] = cnt;
                break;
            case 32:
                ((uint32_t *)V(vd))[ei] = cnt;
                break;
            case 64:
                ((uint64_t *)V(vd))[ei] = cnt;
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
        cnt += !!(V(vs2)[ei >> 3] & (1 << (ei & 0x7)));
    }
}

void glue(helper_vid, POSTFIX)(CPUState *env, uint32_t vd, int32_t vs2)
{
    const target_ulong eew = env->vsew;
    ensure_vector_embedded_extension_for_eew_or_raise_exception(cpu, eew);
    if(V_IDX_INVALID(vd) || env->vstart) {
        raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
    }
    for(int ei = 0; ei < env->vl; ++ei) {
        TEST_MASK(ei)
        switch(eew) {
            case 8:
                ((uint8_t *)V(vd))[ei] = ei;
                break;
            case 16:
                ((uint16_t *)V(vd))[ei] = ei;
                break;
            case 32:
                ((uint32_t *)V(vd))[ei] = ei;
                break;
            case 64:
                ((uint64_t *)V(vd))[ei] = ei;
                break;
            default:
                raise_exception_and_sync_pc(env, RISCV_EXCP_ILLEGAL_INST);
                break;
        }
    }
}

#endif

#undef IS_ELEMENT_ACTIVE
#undef TEST_MASK
#undef MS_MASK
#undef MS_TEST_MASK
#undef VMOP_LOOP_PREFIX

#undef SHIFT
#undef DATA_TYPE
#undef DATA_TYPE_MAX
#undef DATA_TYPE_DOUBLED
#undef DATA_STYPE
#undef DATA_STYPE_MAX
#undef DATA_STYPE_MIN
#undef DATA_STYPE_DOUBLED
#undef BITS
#undef SUFFIX
#undef USUFFIX
#undef DATA_SIZE
#undef MASKED
#undef POSTFIX
