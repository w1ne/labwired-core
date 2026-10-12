/*
 *  PowerPC emulation for qemu: main translation routines.
 *
 *  Copyright (c) 2003-2007 Jocelyn Mayer
 *  Copyright (C) 2011 Freescale Semiconductor, Inc.
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
#include <stdarg.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <inttypes.h>

#include "arch_callbacks.h"

#include "cpu.h"
#include "host-utils.h"

#include "tb-helper.h"

#include "debug.h"

/* Include definitions for instructions classes and implementations flags */

/*****************************************************************************/
/* Code translation helpers                                                  */

static TCGv cpu_gpr[32];
static TCGv cpu_gprh[32];
static TCGv_i64 cpu_fpr[32];
static TCGv_i64 cpu_avrh[32], cpu_avrl[32];
static TCGv_i32 cpu_crf[8];
static TCGv cpu_nip;
static TCGv cpu_msr;
static TCGv cpu_ctr;
static TCGv cpu_lr;
#if defined(TARGET_PPC64)
static TCGv cpu_cfar;
#endif
static TCGv cpu_xer;
static TCGv cpu_reserve;
static TCGv_i32 cpu_fpscr;
static TCGv_i32 cpu_access_type;

/* internal defines */
void translate_init(void)
{
    int i;
    char *p;
    size_t cpu_reg_names_size;

    static char cpu_reg_names[10 * 3 + 22 * 4         /* GPR */
                              + 10 * 4 + 22 * 5       /* SPE GPRh */
                              + 10 * 4 + 22 * 5       /* FPR */
                              + 2 * (10 * 6 + 22 * 7) /* AVRh, AVRl */
                              + 8 * 5 /* CRF */];

    p = cpu_reg_names;
    cpu_reg_names_size = sizeof(cpu_reg_names);

    for(i = 0; i < 8; i++) {
        snprintf(p, cpu_reg_names_size, "crf%d", i);
        cpu_crf[i] = tcg_global_mem_new_i32(TCG_AREG0, offsetof(CPUState, crf[i]), p);
        p += 5;
        cpu_reg_names_size -= 5;
    }

    for(i = 0; i < 32; i++) {
        snprintf(p, cpu_reg_names_size, "r%d", i);
        cpu_gpr[i] = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, gpr[i]), p);
        p += (i < 10) ? 3 : 4;
        cpu_reg_names_size -= (i < 10) ? 3 : 4;
        snprintf(p, cpu_reg_names_size, "r%dH", i);
        cpu_gprh[i] = tcg_global_mem_new_i32(TCG_AREG0, offsetof(CPUState, gprh[i]), p);
        p += (i < 10) ? 4 : 5;
        cpu_reg_names_size -= (i < 10) ? 4 : 5;

        snprintf(p, cpu_reg_names_size, "fp%d", i);
        cpu_fpr[i] = tcg_global_mem_new_i64(TCG_AREG0, offsetof(CPUState, fpr[i]), p);
        p += (i < 10) ? 4 : 5;
        cpu_reg_names_size -= (i < 10) ? 4 : 5;

        snprintf(p, cpu_reg_names_size, "avr%dH", i);
#ifdef HOST_WORDS_BIGENDIAN
        cpu_avrh[i] = tcg_global_mem_new_i64(TCG_AREG0, offsetof(CPUState, avr[i].u64[0]), p);
#else
        cpu_avrh[i] = tcg_global_mem_new_i64(TCG_AREG0, offsetof(CPUState, avr[i].u64[1]), p);
#endif
        p += (i < 10) ? 6 : 7;
        cpu_reg_names_size -= (i < 10) ? 6 : 7;

        snprintf(p, cpu_reg_names_size, "avr%dL", i);
#ifdef HOST_WORDS_BIGENDIAN
        cpu_avrl[i] = tcg_global_mem_new_i64(TCG_AREG0, offsetof(CPUState, avr[i].u64[1]), p);
#else
        cpu_avrl[i] = tcg_global_mem_new_i64(TCG_AREG0, offsetof(CPUState, avr[i].u64[0]), p);
#endif
        p += (i < 10) ? 6 : 7;
        cpu_reg_names_size -= (i < 10) ? 6 : 7;
    }

    cpu_nip = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, nip), "nip");

    cpu_msr = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, msr), "msr");

    cpu_ctr = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, ctr), "ctr");

    cpu_lr = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, lr), "lr");

#if defined(TARGET_PPC64)
    cpu_cfar = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, cfar), "cfar");
#endif

    cpu_xer = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, xer), "xer");

    cpu_reserve = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, reserve_addr), "reserve_addr");

    cpu_fpscr = tcg_global_mem_new_i32(TCG_AREG0, offsetof(CPUState, fpscr), "fpscr");

    cpu_access_type = tcg_global_mem_new_i32(TCG_AREG0, offsetof(CPUState, access_type), "access_type");
}

struct opc_handler_t {
    /* invalid bits for instruction 1 (Rc(opcode) == 0) */
    uint32_t inval1;
    /* invalid bits for instruction 2 (Rc(opcode) == 1) */
    uint32_t inval2;
    /* instruction type */
    uint64_t type;
    /* extended instruction type */
    uint64_t type2;
    /* instruction length in bytes */
    uint8_t length;
    /* handler */
    void (*handler)(DisasContext *s);
};

static inline void gen_reset_fpstatus(void)
{
    gen_helper_reset_fpstatus();
}

static inline void gen_compute_fprf(TCGv_i64 arg, int set_fprf, int set_rc)
{
    TCGv_i32 t0 = tcg_temp_new_i32();

    if(set_fprf != 0) {
        /* This case might be optimized later */
        tcg_gen_movi_i32(t0, 1);
        gen_helper_compute_fprf(t0, arg, t0);
        if(unlikely(set_rc)) {
            tcg_gen_mov_i32(cpu_crf[1], t0);
        }
        gen_helper_float_check_status();
    } else if(unlikely(set_rc)) {
        /* We always need to compute fpcc */
        tcg_gen_movi_i32(t0, 0);
        gen_helper_compute_fprf(t0, arg, t0);
        tcg_gen_mov_i32(cpu_crf[1], t0);
    }

    tcg_temp_free_i32(t0);
}

static inline void gen_set_access_type(DisasContext *s, int access_type)
{
    if(s->access_type != access_type) {
        tcg_gen_movi_i32(cpu_access_type, access_type);
        s->access_type = access_type;
    }
}

static inline void gen_update_nip(DisasContext *s, target_ulong nip)
{
#if defined(TARGET_PPC64)
    if(s->sf_mode) {
        tcg_gen_movi_tl(cpu_nip, nip);
    } else
#endif
        tcg_gen_movi_tl(cpu_nip, (uint32_t)nip);
}

void gen_sync_pc(DisasContext *dc)
{
    tcg_gen_movi_tl(cpu_nip, dc->base.pc);
}

static inline void gen_exception_err(DisasContext *s, uint32_t excp, uint32_t error)
{
    TCGv_i32 t0, t1;
    if(s->exception == POWERPC_EXCP_NONE) {
        gen_update_nip(s, s->base.pc);
    }
    t0 = tcg_const_i32(excp);
    t1 = tcg_const_i32(error);
    gen_helper_raise_exception_err(t0, t1);
    tcg_temp_free_i32(t0);
    tcg_temp_free_i32(t1);
    s->exception = (excp);
}

static inline void gen_exception(DisasContext *s, uint32_t excp)
{
    TCGv_i32 t0;
    if(s->exception == POWERPC_EXCP_NONE) {
        gen_update_nip(s, s->base.pc);
    }
    t0 = tcg_const_i32(excp);
    gen_helper_raise_exception(t0);
    tcg_temp_free_i32(t0);
    s->exception = (excp);
}

static inline void gen_debug_exception(DisasContext *s)
{
    TCGv_i32 t0;

    if((s->exception != POWERPC_EXCP_BRANCH) && (s->exception != POWERPC_EXCP_SYNC)) {
        gen_update_nip(s, s->base.pc);
    }
    t0 = tcg_const_i32(EXCP_DEBUG);
    gen_helper_raise_exception(t0);
    tcg_temp_free_i32(t0);
}

static inline void gen_inval_exception(DisasContext *s, uint32_t error)
{
    gen_exception_err(s, POWERPC_EXCP_PROGRAM, POWERPC_EXCP_INVAL | error);
}

/* Stop translation */
static inline void gen_stop_exception(DisasContext *s)
{
    gen_update_nip(s, s->base.pc);
    s->exception = POWERPC_EXCP_STOP;
}

/* No need to update nip here, as execution flow will change */
static inline void gen_sync_exception(DisasContext *s)
{
    s->exception = POWERPC_EXCP_SYNC;
}

//  Standard PPC helper macros
#define GEN_HANDLER(name, opc1, opc2, opc3, inval, type) GEN_OPCODE(name, opc1, opc2, opc3, inval, type, PPC_NONE, 4)

#define GEN_HANDLER_E(name, opc1, opc2, opc3, inval, type, type2) GEN_OPCODE(name, opc1, opc2, opc3, inval, type, type2, 4)

#define GEN_HANDLER2(name, onam, opc1, opc2, opc3, inval, type) \
    GEN_OPCODE2(name, onam, opc1, opc2, opc3, inval, type, PPC_NONE, 4)

#define GEN_HANDLER2_E(name, onam, opc1, opc2, opc3, inval, type, type2) \
    GEN_OPCODE2(name, onam, opc1, opc2, opc3, inval, type, type2, 4)

typedef struct opcode_t {
    unsigned char opc1, opc2, opc3;
#if HOST_LONG_BITS == 64 /* Explicitly align to 64 bits */
    unsigned char pad[5];
#else
    unsigned char pad[1];
#endif
    opc_handler_t handler;
    const char *oname;
} opcode_t;

/*****************************************************************************/
/***                           Instruction decoding                        ***/
#define EXTRACT_HELPER(name, shift, nb)                 \
    static inline uint32_t name(uint32_t opcode)        \
    {                                                   \
        return (opcode >> (shift)) & ((1 << (nb)) - 1); \
    }

#define EXTRACT_SHELPER(name, shift, nb)                           \
    static inline int32_t name(uint32_t opcode)                    \
    {                                                              \
        return (int16_t)((opcode >> (shift)) & ((1 << (nb)) - 1)); \
    }

/* Opcode part 1 */
EXTRACT_HELPER(opc1, 26, 6);
/* Opcode part 2 */
EXTRACT_HELPER(opc2, 1, 5);
/* Opcode part 3 */
EXTRACT_HELPER(opc3, 6, 5);
/* Update Cr0 flags */
EXTRACT_HELPER(Rc, 0, 1);
/* Destination */
EXTRACT_HELPER(rD, 21, 5);
/* Source */
EXTRACT_HELPER(rS, 21, 5);
/* First operand */
EXTRACT_HELPER(rA, 16, 5);
/* Second operand */
EXTRACT_HELPER(rB, 11, 5);
/* Third operand */
EXTRACT_HELPER(rC, 6, 5);
/***                               Get CRn                                 ***/
EXTRACT_HELPER(crfD, 23, 3);
EXTRACT_HELPER(crfS, 18, 3);
EXTRACT_HELPER(crbD, 21, 5);
EXTRACT_HELPER(crbA, 16, 5);
EXTRACT_HELPER(crbB, 11, 5);
/* SPR / TBL */
EXTRACT_HELPER(_SPR, 11, 10);
static inline uint32_t SPR(uint32_t opcode)
{
    uint32_t sprn = _SPR(opcode);

    return ((sprn >> 5) & 0x1F) | ((sprn & 0x1F) << 5);
}
/***                              Get constants                            ***/
/* 16 bits signed immediate value */
EXTRACT_SHELPER(SIMM, 0, 16);
/* 16 bits unsigned immediate value */
EXTRACT_HELPER(UIMM, 0, 16);
/* 5 bits signed immediate value */
EXTRACT_HELPER(SIMM5, 16, 5);
/* 5 bits signed immediate value */
EXTRACT_HELPER(UIMM5, 16, 5);
/* Bit count */
EXTRACT_HELPER(NB, 11, 5);
/* Shift count */
EXTRACT_HELPER(SH, 11, 5);
/* Vector shift count */
EXTRACT_HELPER(VSH, 6, 4);
/* Mask start */
EXTRACT_HELPER(MB, 6, 5);
/* Mask end */
EXTRACT_HELPER(ME, 1, 5);
/* Trap operand */
EXTRACT_HELPER(TO, 21, 5);

EXTRACT_HELPER(CRM, 12, 8);
EXTRACT_HELPER(FM, 17, 8);
EXTRACT_HELPER(SR, 16, 4);
EXTRACT_HELPER(FPIMM, 12, 4);

/***                            Jump target decoding                       ***/
/* Immediate address */
static inline target_ulong LI(uint32_t opcode)
{
    return (opcode >> 0) & 0x03FFFFFC;
}

static inline uint32_t BD(uint32_t opcode)
{
    return (opcode >> 0) & 0xFFFC;
}

EXTRACT_HELPER(BO, 21, 5);
EXTRACT_HELPER(BI, 16, 5);
/* Absolute/relative address */
EXTRACT_HELPER(AA, 1, 1);
/* Link */
EXTRACT_HELPER(LK, 0, 1);

/* Create a mask between <start> and <end> bits */
static inline target_ulong MASK(uint32_t start, uint32_t end)
{
    target_ulong ret;
    if(start == 0) {
        ret = TARGET_ULONG_MAX << ((TARGET_LONG_BITS - 1) - end);
    } else if(end == (TARGET_LONG_BITS - 1)) {
        ret = TARGET_ULONG_MAX >> start;
    } else {
        ret = (TARGET_ULONG_MAX >> (start)) ^ ((TARGET_ULONG_MAX >> (end)) >> 1);
        if(unlikely(start > end)) {
            return ~ret;
        }
    }

    return ret;
}

/*****************************************************************************/
/* PowerPC instructions table                                                */

#define GEN_OPCODE(name, op1, op2, op3, invl, _typ, _typ2, _length) \
    {                                                                             \
    .opc1 = op1,                                                              \
    .opc2 = op2,                                                              \
    .opc3 = op3,                                                              \
    .pad  = { 0, },                                                           \
    .handler = {                                                              \
        .inval1  = invl,                                                      \
        .type = _typ,                                                         \
        .type2 = _typ2,                                                       \
        .handler = &gen_##name,                                               \
        .length = _length,                                                    \
    },                                                                        \
    .oname = stringify(name),                                                 \
}
#define GEN_OPCODE_DUAL(name, op1, op2, op3, invl1, invl2, _typ, _typ2, _length) \
    {                                                                             \
    .opc1 = op1,                                                              \
    .opc2 = op2,                                                              \
    .opc3 = op3,                                                              \
    .pad  = { 0, },                                                           \
    .handler = {                                                              \
        .inval1  = invl1,                                                     \
        .inval2  = invl2,                                                     \
        .type = _typ,                                                         \
        .type2 = _typ2,                                                       \
        .handler = &gen_##name,                                               \
        .length = _length,                                                    \
    },                                                                        \
    .oname = stringify(name),                                                 \
}
#define GEN_OPCODE2(name, onam, op1, op2, op3, invl, _typ, _typ2, _length) \
    {                                                                             \
    .opc1 = op1,                                                              \
    .opc2 = op2,                                                              \
    .opc3 = op3,                                                              \
    .pad  = { 0, },                                                           \
    .handler = {                                                              \
        .inval1  = invl,                                                      \
        .type = _typ,                                                         \
        .type2 = _typ2,                                                       \
        .handler = &gen_##name,                                               \
        .length = _length,                                                    \
    },                                                                        \
    .oname = onam,                                                            \
}

/* SPR load/store helpers */
static inline void gen_load_spr(TCGv t, int reg)
{
    tcg_gen_ld_tl(t, cpu_env, offsetof(CPUState, spr[reg]));
}

static inline void gen_store_spr(int reg, TCGv t)
{
    tcg_gen_st_tl(t, cpu_env, offsetof(CPUState, spr[reg]));
}

/* Invalid instruction */
static void gen_invalid(DisasContext *s)
{
    gen_inval_exception(s, POWERPC_EXCP_INVAL_INVAL);
}

static opc_handler_t invalid_handler = {
    .inval1 = 0xFFFFFFFF,
    .inval2 = 0xFFFFFFFF,
    .type = PPC_NONE,
    .type2 = PPC_NONE,
    .handler = gen_invalid,
    .length = 4,
};

/***                           Integer comparison                          ***/

static inline void gen_op_cmp(TCGv arg0, TCGv arg1, int s, int crf)
{
    int l1, l2, l3;

    tcg_gen_trunc_tl_i32(cpu_crf[crf], cpu_xer);
    tcg_gen_shri_i32(cpu_crf[crf], cpu_crf[crf], XER_SO);
    tcg_gen_andi_i32(cpu_crf[crf], cpu_crf[crf], 1);

    l1 = gen_new_label();
    l2 = gen_new_label();
    l3 = gen_new_label();
    if(s) {
        tcg_gen_brcond_tl(TCG_COND_LT, arg0, arg1, l1);
        tcg_gen_brcond_tl(TCG_COND_GT, arg0, arg1, l2);
    } else {
        tcg_gen_brcond_tl(TCG_COND_LTU, arg0, arg1, l1);
        tcg_gen_brcond_tl(TCG_COND_GTU, arg0, arg1, l2);
    }
    tcg_gen_ori_i32(cpu_crf[crf], cpu_crf[crf], 1 << CRF_EQ);
    tcg_gen_br(l3);
    gen_set_label(l1);
    tcg_gen_ori_i32(cpu_crf[crf], cpu_crf[crf], 1 << CRF_LT);
    tcg_gen_br(l3);
    gen_set_label(l2);
    tcg_gen_ori_i32(cpu_crf[crf], cpu_crf[crf], 1 << CRF_GT);
    gen_set_label(l3);
}

static inline void gen_op_cmpi(TCGv arg0, target_ulong arg1, int s, int crf)
{
    TCGv t0 = tcg_const_local_tl(arg1);
    gen_op_cmp(arg0, t0, s, crf);
    tcg_temp_free(t0);
}

#if defined(TARGET_PPC64)
static inline void gen_op_cmp32(TCGv arg0, TCGv arg1, int s, int crf)
{
    TCGv t0, t1;
    t0 = tcg_temp_local_new();
    t1 = tcg_temp_local_new();
    if(s) {
        tcg_gen_ext32s_tl(t0, arg0);
        tcg_gen_ext32s_tl(t1, arg1);
    } else {
        tcg_gen_ext32u_tl(t0, arg0);
        tcg_gen_ext32u_tl(t1, arg1);
    }
    gen_op_cmp(t0, t1, s, crf);
    tcg_temp_free(t1);
    tcg_temp_free(t0);
}

static inline void gen_op_cmpi32(TCGv arg0, target_ulong arg1, int s, int crf)
{
    TCGv t0 = tcg_const_local_tl(arg1);
    gen_op_cmp32(arg0, t0, s, crf);
    tcg_temp_free(t0);
}
#endif

static inline void gen_set_Rc0(DisasContext *s, TCGv reg)
{
#if defined(TARGET_PPC64)
    if(!(s->sf_mode)) {
        gen_op_cmpi32(reg, 0, 1, 0);
    } else
#endif
        gen_op_cmpi(reg, 0, 1, 0);
}

/* cmp */
static void gen_cmp(DisasContext *s)
{
#if defined(TARGET_PPC64)
    if(!(s->sf_mode && (s->opcode & 0x00200000))) {
        gen_op_cmp32(cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)], 1, crfD(s->opcode));
    } else
#endif
        gen_op_cmp(cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)], 1, crfD(s->opcode));
}

/* cmpi */
static void gen_cmpi(DisasContext *s)
{
#if defined(TARGET_PPC64)
    if(!(s->sf_mode && (s->opcode & 0x00200000))) {
        gen_op_cmpi32(cpu_gpr[rA(s->opcode)], SIMM(s->opcode), 1, crfD(s->opcode));
    } else
#endif
        gen_op_cmpi(cpu_gpr[rA(s->opcode)], SIMM(s->opcode), 1, crfD(s->opcode));
}

/* cmpl */
static void gen_cmpl(DisasContext *s)
{
#if defined(TARGET_PPC64)
    if(!(s->sf_mode && (s->opcode & 0x00200000))) {
        gen_op_cmp32(cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)], 0, crfD(s->opcode));
    } else
#endif
        gen_op_cmp(cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)], 0, crfD(s->opcode));
}

/* cmpli */
static void gen_cmpli(DisasContext *s)
{
#if defined(TARGET_PPC64)
    if(!(s->sf_mode && (s->opcode & 0x00200000))) {
        gen_op_cmpi32(cpu_gpr[rA(s->opcode)], UIMM(s->opcode), 0, crfD(s->opcode));
    } else
#endif
        gen_op_cmpi(cpu_gpr[rA(s->opcode)], UIMM(s->opcode), 0, crfD(s->opcode));
}

/* isel (PowerPC 2.03 specification) */
static void gen_isel(DisasContext *s)
{
    int l1, l2;
    uint32_t bi = rC(s->opcode);
    uint32_t mask;
    TCGv_i32 t0;

    l1 = gen_new_label();
    l2 = gen_new_label();

    mask = 1 << (3 - (bi & 0x03));
    t0 = tcg_temp_new_i32();
    tcg_gen_andi_i32(t0, cpu_crf[bi >> 2], mask);
    tcg_gen_brcondi_i32(TCG_COND_EQ, t0, 0, l1);
    if(rA(s->opcode) == 0) {
        tcg_gen_movi_tl(cpu_gpr[rD(s->opcode)], 0);
    } else {
        tcg_gen_mov_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)]);
    }
    tcg_gen_br(l2);
    gen_set_label(l1);
    tcg_gen_mov_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rB(s->opcode)]);
    gen_set_label(l2);
    tcg_temp_free_i32(t0);
}

/***                           Integer arithmetic                          ***/

static inline void gen_op_arith_compute_ov(DisasContext *s, TCGv arg0, TCGv arg1, TCGv arg2, int sub)
{
    int l1;
    TCGv t0;

    l1 = gen_new_label();
    /* Start with XER OV disabled, the most likely case */
    tcg_gen_andi_tl(cpu_xer, cpu_xer, ~(1 << XER_OV));
    t0 = tcg_temp_local_new();
    tcg_gen_xor_tl(t0, arg0, arg1);
#if defined(TARGET_PPC64)
    if(!s->sf_mode) {
        tcg_gen_ext32s_tl(t0, t0);
    }
#endif
    if(sub) {
        tcg_gen_brcondi_tl(TCG_COND_LT, t0, 0, l1);
    } else {
        tcg_gen_brcondi_tl(TCG_COND_GE, t0, 0, l1);
    }
    tcg_gen_xor_tl(t0, arg1, arg2);
#if defined(TARGET_PPC64)
    if(!s->sf_mode) {
        tcg_gen_ext32s_tl(t0, t0);
    }
#endif
    if(sub) {
        tcg_gen_brcondi_tl(TCG_COND_GE, t0, 0, l1);
    } else {
        tcg_gen_brcondi_tl(TCG_COND_LT, t0, 0, l1);
    }
    tcg_gen_ori_tl(cpu_xer, cpu_xer, (1 << XER_OV) | (1 << XER_SO));
    gen_set_label(l1);
    tcg_temp_free(t0);
}

static inline void gen_op_arith_compute_ca(DisasContext *s, TCGv arg1, TCGv arg2, int sub)
{
    int l1 = gen_new_label();

#if defined(TARGET_PPC64)
    if(!(s->sf_mode)) {
        TCGv t0, t1;
        t0 = tcg_temp_new();
        t1 = tcg_temp_new();

        tcg_gen_ext32u_tl(t0, arg1);
        tcg_gen_ext32u_tl(t1, arg2);
        if(sub) {
            tcg_gen_brcond_tl(TCG_COND_GTU, t0, t1, l1);
        } else {
            tcg_gen_brcond_tl(TCG_COND_GEU, t0, t1, l1);
        }
        tcg_gen_ori_tl(cpu_xer, cpu_xer, 1 << XER_CA);
        gen_set_label(l1);
        tcg_temp_free(t0);
        tcg_temp_free(t1);
    } else
#endif
    {
        if(sub) {
            tcg_gen_brcond_tl(TCG_COND_GTU, arg1, arg2, l1);
        } else {
            tcg_gen_brcond_tl(TCG_COND_GEU, arg1, arg2, l1);
        }
    }
    tcg_gen_ori_tl(cpu_xer, cpu_xer, 1 << XER_CA);
    gen_set_label(l1);
}

/* Common add function */
static inline void gen_op_arith_add(DisasContext *s, TCGv ret, TCGv arg1, TCGv arg2, int add_ca, int compute_ca, int compute_ov)
{
    TCGv t0, t1;

    if((!compute_ca && !compute_ov) || (!TCGV_EQUAL(ret, arg1) && !TCGV_EQUAL(ret, arg2))) {
        t0 = ret;
    } else {
        t0 = tcg_temp_local_new();
    }

    if(add_ca) {
        t1 = tcg_temp_local_new();
        tcg_gen_andi_tl(t1, cpu_xer, (1 << XER_CA));
        tcg_gen_shri_tl(t1, t1, XER_CA);
    } else {
        TCGV_UNUSED(t1);
    }

    if(compute_ca && compute_ov) {
        /* Start with XER CA and OV disabled, the most likely case */
        tcg_gen_andi_tl(cpu_xer, cpu_xer, ~((1 << XER_CA) | (1 << XER_OV)));
    } else if(compute_ca) {
        /* Start with XER CA disabled, the most likely case */
        tcg_gen_andi_tl(cpu_xer, cpu_xer, ~(1 << XER_CA));
    } else if(compute_ov) {
        /* Start with XER OV disabled, the most likely case */
        tcg_gen_andi_tl(cpu_xer, cpu_xer, ~(1 << XER_OV));
    }

    tcg_gen_add_tl(t0, arg1, arg2);

    if(compute_ca) {
        gen_op_arith_compute_ca(s, t0, arg1, 0);
    }
    if(add_ca) {
        tcg_gen_add_tl(t0, t0, t1);
        gen_op_arith_compute_ca(s, t0, t1, 0);
        tcg_temp_free(t1);
    }
    if(compute_ov) {
        gen_op_arith_compute_ov(s, t0, arg1, arg2, 0);
    }

    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, t0);
    }

    if(!TCGV_EQUAL(t0, ret)) {
        tcg_gen_mov_tl(ret, t0);
        tcg_temp_free(t0);
    }
}
/* Add functions with two operands */
#define GEN_INT_ARITH_ADD(name, opc3, add_ca, compute_ca, compute_ov)                                                   \
    static void glue(gen_, name)(DisasContext * s)                                                                      \
    {                                                                                                                   \
        gen_op_arith_add(s, cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)], add_ca, compute_ca, \
                         compute_ov);                                                                                   \
    }
/* Add functions with one operand and one immediate */
#define GEN_INT_ARITH_ADD_CONST(name, opc3, const_val, add_ca, compute_ca, compute_ov)                           \
    static void glue(gen_, name)(DisasContext * s)                                                               \
    {                                                                                                            \
        TCGv t0 = tcg_const_local_tl(const_val);                                                                 \
        gen_op_arith_add(s, cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], t0, add_ca, compute_ca, compute_ov); \
        tcg_temp_free(t0);                                                                                       \
    }

/* add  add.  addo  addo. */
GEN_INT_ARITH_ADD(add, 0x08, 0, 0, 0)
GEN_INT_ARITH_ADD(addo, 0x18, 0, 0, 1)
/* addc  addc.  addco  addco. */
GEN_INT_ARITH_ADD(addc, 0x00, 0, 1, 0)
GEN_INT_ARITH_ADD(addco, 0x10, 0, 1, 1)
/* adde  adde.  addeo  addeo. */
GEN_INT_ARITH_ADD(adde, 0x04, 1, 1, 0)
GEN_INT_ARITH_ADD(addeo, 0x14, 1, 1, 1)
/* addme  addme.  addmeo  addmeo.  */
GEN_INT_ARITH_ADD_CONST(addme, 0x07, -1LL, 1, 1, 0)
GEN_INT_ARITH_ADD_CONST(addmeo, 0x17, -1LL, 1, 1, 1)
/* addze  addze.  addzeo  addzeo.*/
GEN_INT_ARITH_ADD_CONST(addze, 0x06, 0, 1, 1, 0)
GEN_INT_ARITH_ADD_CONST(addzeo, 0x16, 0, 1, 1, 1)
/* addi */
static void gen_addi(DisasContext *s)
{
    target_long simm = SIMM(s->opcode);

    if(rA(s->opcode) == 0) {
        /* li case */
        tcg_gen_movi_tl(cpu_gpr[rD(s->opcode)], simm);
    } else {
        tcg_gen_addi_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], simm);
    }
}
/* addic  addic.*/
static inline void gen_op_addic(DisasContext *s, TCGv ret, TCGv arg1, int compute_Rc0)
{
    target_long simm = SIMM(s->opcode);

    /* Start with XER CA and OV disabled, the most likely case */
    tcg_gen_andi_tl(cpu_xer, cpu_xer, ~(1 << XER_CA));

    if(likely(simm != 0)) {
        TCGv t0 = tcg_temp_local_new();
        tcg_gen_addi_tl(t0, arg1, simm);
        gen_op_arith_compute_ca(s, t0, arg1, 0);
        tcg_gen_mov_tl(ret, t0);
        tcg_temp_free(t0);
    } else {
        tcg_gen_mov_tl(ret, arg1);
    }
    if(compute_Rc0) {
        gen_set_Rc0(s, ret);
    }
}

static void gen_addic(DisasContext *s)
{
    gen_op_addic(s, cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], 0);
}

static void gen_addic_(DisasContext *s)
{
    gen_op_addic(s, cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], 1);
}

/* addis */
static void gen_addis(DisasContext *s)
{
    target_long simm = SIMM(s->opcode);

    if(rA(s->opcode) == 0) {
        /* lis case */
        tcg_gen_movi_tl(cpu_gpr[rD(s->opcode)], simm << 16);
    } else {
        tcg_gen_addi_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], simm << 16);
    }
}

static inline void gen_op_arith_divw(DisasContext *s, TCGv ret, TCGv arg1, TCGv arg2, int sign, int compute_ov)
{
    int l1 = gen_new_label();
    int l2 = gen_new_label();
    TCGv_i32 t0 = tcg_temp_local_new_i32();
    TCGv_i32 t1 = tcg_temp_local_new_i32();

    tcg_gen_trunc_tl_i32(t0, arg1);
    tcg_gen_trunc_tl_i32(t1, arg2);
    tcg_gen_brcondi_i32(TCG_COND_EQ, t1, 0, l1);
    if(sign) {
        int l3 = gen_new_label();
        tcg_gen_brcondi_i32(TCG_COND_NE, t1, -1, l3);
        tcg_gen_brcondi_i32(TCG_COND_EQ, t0, INT32_MIN, l1);
        gen_set_label(l3);
        tcg_gen_div_i32(t0, t0, t1);
    } else {
        tcg_gen_divu_i32(t0, t0, t1);
    }
    if(compute_ov) {
        tcg_gen_andi_tl(cpu_xer, cpu_xer, ~(1 << XER_OV));
    }
    tcg_gen_br(l2);
    gen_set_label(l1);
    if(sign) {
        tcg_gen_sari_i32(t0, t0, 31);
    } else {
        tcg_gen_movi_i32(t0, 0);
    }
    if(compute_ov) {
        tcg_gen_ori_tl(cpu_xer, cpu_xer, (1 << XER_OV) | (1 << XER_SO));
    }
    gen_set_label(l2);
    tcg_gen_extu_i32_tl(ret, t0);
    tcg_temp_free_i32(t0);
    tcg_temp_free_i32(t1);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, ret);
    }
}
/* Div functions */
#define GEN_INT_ARITH_DIVW(name, opc3, sign, compute_ov)                                                                \
    static void glue(gen_, name)(DisasContext * s)                                                                      \
    {                                                                                                                   \
        gen_op_arith_divw(s, cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)], sign, compute_ov); \
    }
/* divwu  divwu.  divwuo  divwuo.   */
GEN_INT_ARITH_DIVW(divwu, 0x0E, 0, 0);
GEN_INT_ARITH_DIVW(divwuo, 0x1E, 0, 1);
/* divw  divw.  divwo  divwo.   */
GEN_INT_ARITH_DIVW(divw, 0x0F, 1, 0);
GEN_INT_ARITH_DIVW(divwo, 0x1F, 1, 1);

#if defined(TARGET_PPC64)
static inline void gen_op_arith_divd(DisasContext *s, TCGv ret, TCGv arg1, TCGv arg2, int sign, int compute_ov)
{
    int l1 = gen_new_label();
    int l2 = gen_new_label();

    tcg_gen_brcondi_i64(TCG_COND_EQ, arg2, 0, l1);
    if(sign) {
        int l3 = gen_new_label();
        tcg_gen_brcondi_i64(TCG_COND_NE, arg2, -1, l3);
        tcg_gen_brcondi_i64(TCG_COND_EQ, arg1, INT64_MIN, l1);
        gen_set_label(l3);
        tcg_gen_div_i64(ret, arg1, arg2);
    } else {
        tcg_gen_divu_i64(ret, arg1, arg2);
    }
    if(compute_ov) {
        tcg_gen_andi_tl(cpu_xer, cpu_xer, ~(1 << XER_OV));
    }
    tcg_gen_br(l2);
    gen_set_label(l1);
    if(sign) {
        tcg_gen_sari_i64(ret, arg1, 63);
    } else {
        tcg_gen_movi_i64(ret, 0);
    }
    if(compute_ov) {
        tcg_gen_ori_tl(cpu_xer, cpu_xer, (1 << XER_OV) | (1 << XER_SO));
    }
    gen_set_label(l2);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, ret);
    }
}
#define GEN_INT_ARITH_DIVD(name, opc3, sign, compute_ov)                                                                \
    static void glue(gen_, name)(DisasContext * s)                                                                      \
    {                                                                                                                   \
        gen_op_arith_divd(s, cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)], sign, compute_ov); \
    }
/* divwu  divwu.  divwuo  divwuo.   */
GEN_INT_ARITH_DIVD(divdu, 0x0E, 0, 0);
GEN_INT_ARITH_DIVD(divduo, 0x1E, 0, 1);
/* divw  divw.  divwo  divwo.   */
GEN_INT_ARITH_DIVD(divd, 0x0F, 1, 0);
GEN_INT_ARITH_DIVD(divdo, 0x1F, 1, 1);
#endif

/* mulhw  mulhw. */
static void gen_mulhw(DisasContext *s)
{
    TCGv_i64 t0, t1;

    t0 = tcg_temp_new_i64();
    t1 = tcg_temp_new_i64();
#if defined(TARGET_PPC64)
    tcg_gen_ext32s_tl(t0, cpu_gpr[rA(s->opcode)]);
    tcg_gen_ext32s_tl(t1, cpu_gpr[rB(s->opcode)]);
    tcg_gen_mul_i64(t0, t0, t1);
    tcg_gen_shri_i64(cpu_gpr[rD(s->opcode)], t0, 32);
#else
    tcg_gen_ext_tl_i64(t0, cpu_gpr[rA(s->opcode)]);
    tcg_gen_ext_tl_i64(t1, cpu_gpr[rB(s->opcode)]);
    tcg_gen_mul_i64(t0, t0, t1);
    tcg_gen_shri_i64(t0, t0, 32);
    tcg_gen_trunc_i64_tl(cpu_gpr[rD(s->opcode)], t0);
#endif
    tcg_temp_free_i64(t0);
    tcg_temp_free_i64(t1);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rD(s->opcode)]);
    }
}

/* mulhwu  mulhwu.  */
static void gen_mulhwu(DisasContext *s)
{
    TCGv_i64 t0, t1;

    t0 = tcg_temp_new_i64();
    t1 = tcg_temp_new_i64();
#if defined(TARGET_PPC64)
    tcg_gen_ext32u_i64(t0, cpu_gpr[rA(s->opcode)]);
    tcg_gen_ext32u_i64(t1, cpu_gpr[rB(s->opcode)]);
    tcg_gen_mul_i64(t0, t0, t1);
    tcg_gen_shri_i64(cpu_gpr[rD(s->opcode)], t0, 32);
#else
    tcg_gen_extu_tl_i64(t0, cpu_gpr[rA(s->opcode)]);
    tcg_gen_extu_tl_i64(t1, cpu_gpr[rB(s->opcode)]);
    tcg_gen_mul_i64(t0, t0, t1);
    tcg_gen_shri_i64(t0, t0, 32);
    tcg_gen_trunc_i64_tl(cpu_gpr[rD(s->opcode)], t0);
#endif
    tcg_temp_free_i64(t0);
    tcg_temp_free_i64(t1);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rD(s->opcode)]);
    }
}

/* mullw  mullw. */
static void gen_mullw(DisasContext *s)
{
    tcg_gen_mul_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)]);
    tcg_gen_ext32s_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rD(s->opcode)]);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rD(s->opcode)]);
    }
}

/* mullwo  mullwo. */
static void gen_mullwo(DisasContext *s)
{
    int l1;
    TCGv_i64 t0, t1;

    t0 = tcg_temp_new_i64();
    t1 = tcg_temp_new_i64();
    l1 = gen_new_label();
    /* Start with XER OV disabled, the most likely case */
    tcg_gen_andi_tl(cpu_xer, cpu_xer, ~(1 << XER_OV));
#if defined(TARGET_PPC64)
    tcg_gen_ext32s_i64(t0, cpu_gpr[rA(s->opcode)]);
    tcg_gen_ext32s_i64(t1, cpu_gpr[rB(s->opcode)]);
#else
    tcg_gen_ext_tl_i64(t0, cpu_gpr[rA(s->opcode)]);
    tcg_gen_ext_tl_i64(t1, cpu_gpr[rB(s->opcode)]);
#endif
    tcg_gen_mul_i64(t0, t0, t1);
#if defined(TARGET_PPC64)
    tcg_gen_ext32s_i64(cpu_gpr[rD(s->opcode)], t0);
    tcg_gen_brcond_i64(TCG_COND_EQ, t0, cpu_gpr[rD(s->opcode)], l1);
#else
    tcg_gen_trunc_i64_tl(cpu_gpr[rD(s->opcode)], t0);
    tcg_gen_ext32s_i64(t1, t0);
    tcg_gen_brcond_i64(TCG_COND_EQ, t0, t1, l1);
#endif
    tcg_gen_ori_tl(cpu_xer, cpu_xer, (1 << XER_OV) | (1 << XER_SO));
    gen_set_label(l1);
    tcg_temp_free_i64(t0);
    tcg_temp_free_i64(t1);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rD(s->opcode)]);
    }
}

/* mulli */
static void gen_mulli(DisasContext *s)
{
    tcg_gen_muli_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], SIMM(s->opcode));
}
#if defined(TARGET_PPC64)
#define GEN_INT_ARITH_MUL_HELPER(name, opc3)                                                       \
    static void glue(gen_, name)(DisasContext * s)                                                 \
    {                                                                                              \
        gen_helper_##name(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)]); \
        if(unlikely(Rc(s->opcode) != 0))                                                           \
            gen_set_Rc0(s, cpu_gpr[rD(s->opcode)]);                                                \
    }
/* mulhd  mulhd. */
GEN_INT_ARITH_MUL_HELPER(mulhdu, 0x00);
/* mulhdu  mulhdu. */
GEN_INT_ARITH_MUL_HELPER(mulhd, 0x02);

/* mulld  mulld. */
static void gen_mulld(DisasContext *s)
{
    tcg_gen_mul_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)]);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rD(s->opcode)]);
    }
}
/* mulldo  mulldo. */
GEN_INT_ARITH_MUL_HELPER(mulldo, 0x17);
#endif
/* neg neg. nego nego. */
static inline void gen_op_arith_neg(DisasContext *s, TCGv ret, TCGv arg1, int ov_check)
{
    int l1 = gen_new_label();
    int l2 = gen_new_label();
    TCGv t0 = tcg_temp_local_new();
#if defined(TARGET_PPC64)
    if(s->sf_mode) {
        tcg_gen_mov_tl(t0, arg1);
        tcg_gen_brcondi_tl(TCG_COND_EQ, t0, INT64_MIN, l1);
    } else
#endif
    {
        tcg_gen_ext32s_tl(t0, arg1);
        tcg_gen_brcondi_tl(TCG_COND_EQ, t0, INT32_MIN, l1);
    }
    tcg_gen_neg_tl(ret, arg1);
    if(ov_check) {
        tcg_gen_andi_tl(cpu_xer, cpu_xer, ~(1 << XER_OV));
    }
    tcg_gen_br(l2);
    gen_set_label(l1);
    tcg_gen_mov_tl(ret, t0);
    if(ov_check) {
        tcg_gen_ori_tl(cpu_xer, cpu_xer, (1 << XER_OV) | (1 << XER_SO));
    }
    gen_set_label(l2);
    tcg_temp_free(t0);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, ret);
    }
}

static void gen_neg(DisasContext *s)
{
    gen_op_arith_neg(s, cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], 0);
}

static void gen_nego(DisasContext *s)
{
    gen_op_arith_neg(s, cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], 1);
}

/* Common subf function */
static inline void gen_op_arith_subf(DisasContext *s, TCGv ret, TCGv arg1, TCGv arg2, int add_ca, int compute_ca, int compute_ov)
{
    TCGv t0, t1;

    if((!compute_ca && !compute_ov) || (!TCGV_EQUAL(ret, arg1) && !TCGV_EQUAL(ret, arg2))) {
        t0 = ret;
    } else {
        t0 = tcg_temp_local_new();
    }

    if(add_ca) {
        t1 = tcg_temp_local_new();
        tcg_gen_andi_tl(t1, cpu_xer, (1 << XER_CA));
        tcg_gen_shri_tl(t1, t1, XER_CA);
    } else {
        TCGV_UNUSED(t1);
    }

    if(compute_ca && compute_ov) {
        /* Start with XER CA and OV disabled, the most likely case */
        tcg_gen_andi_tl(cpu_xer, cpu_xer, ~((1 << XER_CA) | (1 << XER_OV)));
    } else if(compute_ca) {
        /* Start with XER CA disabled, the most likely case */
        tcg_gen_andi_tl(cpu_xer, cpu_xer, ~(1 << XER_CA));
    } else if(compute_ov) {
        /* Start with XER OV disabled, the most likely case */
        tcg_gen_andi_tl(cpu_xer, cpu_xer, ~(1 << XER_OV));
    }

    if(add_ca) {
        tcg_gen_not_tl(t0, arg1);
        tcg_gen_add_tl(t0, t0, arg2);
        gen_op_arith_compute_ca(s, t0, arg2, 0);
        tcg_gen_add_tl(t0, t0, t1);
        gen_op_arith_compute_ca(s, t0, t1, 0);
        tcg_temp_free(t1);
    } else {
        tcg_gen_sub_tl(t0, arg2, arg1);
        if(compute_ca) {
            gen_op_arith_compute_ca(s, t0, arg2, 1);
        }
    }
    if(compute_ov) {
        gen_op_arith_compute_ov(s, t0, arg1, arg2, 1);
    }

    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, t0);
    }

    if(!TCGV_EQUAL(t0, ret)) {
        tcg_gen_mov_tl(ret, t0);
        tcg_temp_free(t0);
    }
}
/* Sub functions with Two operands functions */
#define GEN_INT_ARITH_SUBF(name, opc3, add_ca, compute_ca, compute_ov)                                                   \
    static void glue(gen_, name)(DisasContext * s)                                                                       \
    {                                                                                                                    \
        gen_op_arith_subf(s, cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)], add_ca, compute_ca, \
                          compute_ov);                                                                                   \
    }
/* Sub functions with one operand and one immediate */
#define GEN_INT_ARITH_SUBF_CONST(name, opc3, const_val, add_ca, compute_ca, compute_ov)                           \
    static void glue(gen_, name)(DisasContext * s)                                                                \
    {                                                                                                             \
        TCGv t0 = tcg_const_local_tl(const_val);                                                                  \
        gen_op_arith_subf(s, cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], t0, add_ca, compute_ca, compute_ov); \
        tcg_temp_free(t0);                                                                                        \
    }
/* subf  subf.  subfo  subfo. */
GEN_INT_ARITH_SUBF(subf, 0x01, 0, 0, 0)
GEN_INT_ARITH_SUBF(subfo, 0x11, 0, 0, 1)
/* subfc  subfc.  subfco  subfco. */
GEN_INT_ARITH_SUBF(subfc, 0x00, 0, 1, 0)
GEN_INT_ARITH_SUBF(subfco, 0x10, 0, 1, 1)
/* subfe  subfe.  subfeo  subfo. */
GEN_INT_ARITH_SUBF(subfe, 0x04, 1, 1, 0)
GEN_INT_ARITH_SUBF(subfeo, 0x14, 1, 1, 1)
/* subfme  subfme.  subfmeo  subfmeo.  */
GEN_INT_ARITH_SUBF_CONST(subfme, 0x07, -1LL, 1, 1, 0)
GEN_INT_ARITH_SUBF_CONST(subfmeo, 0x17, -1LL, 1, 1, 1)
/* subfze  subfze.  subfzeo  subfzeo.*/
GEN_INT_ARITH_SUBF_CONST(subfze, 0x06, 0, 1, 1, 0)
GEN_INT_ARITH_SUBF_CONST(subfzeo, 0x16, 0, 1, 1, 1)

/* subfic */
static void gen_subfic(DisasContext *s)
{
    /* Start with XER CA and OV disabled, the most likely case */
    tcg_gen_andi_tl(cpu_xer, cpu_xer, ~(1 << XER_CA));
    TCGv t0 = tcg_temp_local_new();
    TCGv t1 = tcg_const_local_tl(SIMM(s->opcode));
    tcg_gen_sub_tl(t0, t1, cpu_gpr[rA(s->opcode)]);
    gen_op_arith_compute_ca(s, t0, t1, 1);
    tcg_temp_free(t1);
    tcg_gen_mov_tl(cpu_gpr[rD(s->opcode)], t0);
    tcg_temp_free(t0);
}

/***                            Integer logical                            ***/
#define GEN_LOGICAL2(name, tcg_op, opc, type)                                           \
    static void glue(gen_, name)(DisasContext * s)                                      \
    {                                                                                   \
        tcg_op(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)], cpu_gpr[rB(s->opcode)]); \
        if(unlikely(Rc(s->opcode) != 0))                                                \
            gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);                                     \
    }

#define GEN_LOGICAL1(name, tcg_op, opc, type)                   \
    static void glue(gen_, name)(DisasContext * s)              \
    {                                                           \
        tcg_op(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)]); \
        if(unlikely(Rc(s->opcode) != 0))                        \
            gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);             \
    }

/* and & and. */
GEN_LOGICAL2(and, tcg_gen_and_tl, 0x00, PPC_INTEGER);
/* andc & andc. */
GEN_LOGICAL2(andc, tcg_gen_andc_tl, 0x01, PPC_INTEGER);

/* andi. */
static void gen_andi_(DisasContext *s)
{
    tcg_gen_andi_tl(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)], UIMM(s->opcode));
    gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
}

/* andis. */
static void gen_andis_(DisasContext *s)
{
    tcg_gen_andi_tl(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)], UIMM(s->opcode) << 16);
    gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
}

/* cntlzw */
static void gen_cntlzw(DisasContext *s)
{
    gen_helper_cntlzw(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)]);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}
/* eqv & eqv. */
GEN_LOGICAL2(eqv, tcg_gen_eqv_tl, 0x08, PPC_INTEGER);
/* extsb & extsb. */
GEN_LOGICAL1(extsb, tcg_gen_ext8s_tl, 0x1D, PPC_INTEGER);
/* extsh & extsh. */
GEN_LOGICAL1(extsh, tcg_gen_ext16s_tl, 0x1C, PPC_INTEGER);
/* nand & nand. */
GEN_LOGICAL2(nand, tcg_gen_nand_tl, 0x0E, PPC_INTEGER);
/* nor & nor. */
GEN_LOGICAL2(nor, tcg_gen_nor_tl, 0x03, PPC_INTEGER);

/* or & or. */
static void gen_or(DisasContext *s)
{
    int rs, ra, rb;

    rs = rS(s->opcode);
    ra = rA(s->opcode);
    rb = rB(s->opcode);
    /* Optimisation for mr. ri case */
    if(rs != ra || rs != rb) {
        if(rs != rb) {
            tcg_gen_or_tl(cpu_gpr[ra], cpu_gpr[rs], cpu_gpr[rb]);
        } else {
            tcg_gen_mov_tl(cpu_gpr[ra], cpu_gpr[rs]);
        }
        if(unlikely(Rc(s->opcode) != 0)) {
            gen_set_Rc0(s, cpu_gpr[ra]);
        }
    } else if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rs]);

#if defined(TARGET_PPC64)
    } else {
        int prio = 0;

        switch(rs) {
            case 1:
                /* Set process priority to low */
                prio = 2;
                break;
            case 6:
                /* Set process priority to medium-low */
                prio = 3;
                break;
            case 2:
                /* Set process priority to normal */
                prio = 4;
                break;
            case 31:
                if(s->base.mem_idx > 0) {
                    /* Set process priority to very low */
                    prio = 1;
                }
                break;
            case 5:
                if(s->base.mem_idx > 0) {
                    /* Set process priority to medium-hight */
                    prio = 5;
                }
                break;
            case 3:
                if(s->base.mem_idx > 0) {
                    /* Set process priority to high */
                    prio = 6;
                }
                break;
            case 7:
                if(s->base.mem_idx > 1) {
                    /* Set process priority to very high */
                    prio = 7;
                }
                break;
            default:
                /* nop */
                break;
        }
        if(prio) {
            TCGv t0 = tcg_temp_new();
            gen_load_spr(t0, SPR_PPR);
            tcg_gen_andi_tl(t0, t0, ~0x001C000000000000ULL);
            tcg_gen_ori_tl(t0, t0, ((uint64_t)prio) << 50);
            gen_store_spr(SPR_PPR, t0);
            tcg_temp_free(t0);
        }
#endif
    }
}
/* orc & orc. */
GEN_LOGICAL2(orc, tcg_gen_orc_tl, 0x0C, PPC_INTEGER);

/* xor & xor. */
static void gen_xor(DisasContext *s)
{
    /* Optimisation for "set to zero" case */
    if(rS(s->opcode) != rB(s->opcode)) {
        tcg_gen_xor_tl(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)], cpu_gpr[rB(s->opcode)]);
    } else {
        tcg_gen_movi_tl(cpu_gpr[rA(s->opcode)], 0);
    }
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* ori */
static void gen_ori(DisasContext *s)
{
    target_ulong uimm = UIMM(s->opcode);

    if(rS(s->opcode) == rA(s->opcode) && uimm == 0) {
        /* NOP */
        /* XXX: should handle special NOPs for POWER series */
        return;
    }
    tcg_gen_ori_tl(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)], uimm);
}

/* oris */
static void gen_oris(DisasContext *s)
{
    target_ulong uimm = UIMM(s->opcode);

    if(rS(s->opcode) == rA(s->opcode) && uimm == 0) {
        /* NOP */
        return;
    }
    tcg_gen_ori_tl(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)], uimm << 16);
}

/* xori */
static void gen_xori(DisasContext *s)
{
    target_ulong uimm = UIMM(s->opcode);

    if(rS(s->opcode) == rA(s->opcode) && uimm == 0) {
        /* NOP */
        return;
    }
    tcg_gen_xori_tl(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)], uimm);
}

/* xoris */
static void gen_xoris(DisasContext *s)
{
    target_ulong uimm = UIMM(s->opcode);

    if(rS(s->opcode) == rA(s->opcode) && uimm == 0) {
        /* NOP */
        return;
    }
    tcg_gen_xori_tl(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)], uimm << 16);
}

/* popcntb : PowerPC 2.03 specification */
static void gen_popcntb(DisasContext *s)
{
    gen_helper_popcntb(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)]);
}

static void gen_popcntw(DisasContext *s)
{
    gen_helper_popcntw(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)]);
}

#if defined(TARGET_PPC64)

static void gen_popcntd(DisasContext *s)
{
    gen_helper_popcntd(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)]);
}

/* extsw & extsw. */
GEN_LOGICAL1(extsw, tcg_gen_ext32s_tl, 0x1E, PPC_64B);

/* cntlzd */
static void gen_cntlzd(DisasContext *s)
{
    gen_helper_cntlzd(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)]);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}
#endif

/***                             Integer rotate                            ***/

/* rlwimi & rlwimi. */
static void gen_rlwimi(DisasContext *s)
{
    uint32_t mb, me, sh;

    mb = MB(s->opcode);
    me = ME(s->opcode);
    sh = SH(s->opcode);
    if(likely(sh == 0 && mb == 0 && me == 31)) {
        tcg_gen_ext32u_tl(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)]);
    } else {
        target_ulong mask;
        TCGv t1;
        TCGv t0 = tcg_temp_new();
#if defined(TARGET_PPC64)
        TCGv_i32 t2 = tcg_temp_new_i32();
        tcg_gen_trunc_i64_i32(t2, cpu_gpr[rS(s->opcode)]);
        tcg_gen_rotli_i32(t2, t2, sh);
        tcg_gen_extu_i32_i64(t0, t2);
        tcg_temp_free_i32(t2);
#else
        tcg_gen_rotli_i32(t0, cpu_gpr[rS(s->opcode)], sh);
#endif
#if defined(TARGET_PPC64)
        mb += 32;
        me += 32;
#endif
        mask = MASK(mb, me);
        t1 = tcg_temp_new();
        tcg_gen_andi_tl(t0, t0, mask);
        tcg_gen_andi_tl(t1, cpu_gpr[rA(s->opcode)], ~mask);
        tcg_gen_or_tl(cpu_gpr[rA(s->opcode)], t0, t1);
        tcg_temp_free(t0);
        tcg_temp_free(t1);
    }
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* rlwinm & rlwinm. */
static void gen_rlwinm(DisasContext *s)
{
    uint32_t mb, me, sh;

    sh = SH(s->opcode);
    mb = MB(s->opcode);
    me = ME(s->opcode);

    if(likely(mb == 0 && me == (31 - sh))) {
        if(likely(sh == 0)) {
            tcg_gen_ext32u_tl(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)]);
        } else {
            TCGv t0 = tcg_temp_new();
            tcg_gen_ext32u_tl(t0, cpu_gpr[rS(s->opcode)]);
            tcg_gen_shli_tl(t0, t0, sh);
            tcg_gen_ext32u_tl(cpu_gpr[rA(s->opcode)], t0);
            tcg_temp_free(t0);
        }
    } else if(likely(sh != 0 && me == 31 && sh == (32 - mb))) {
        TCGv t0 = tcg_temp_new();
        tcg_gen_ext32u_tl(t0, cpu_gpr[rS(s->opcode)]);
        tcg_gen_shri_tl(t0, t0, mb);
        tcg_gen_ext32u_tl(cpu_gpr[rA(s->opcode)], t0);
        tcg_temp_free(t0);
    } else {
        TCGv t0 = tcg_temp_new();
#if defined(TARGET_PPC64)
        TCGv_i32 t1 = tcg_temp_new_i32();
        tcg_gen_trunc_i64_i32(t1, cpu_gpr[rS(s->opcode)]);
        tcg_gen_rotli_i32(t1, t1, sh);
        tcg_gen_extu_i32_i64(t0, t1);
        tcg_temp_free_i32(t1);
#else
        tcg_gen_rotli_i32(t0, cpu_gpr[rS(s->opcode)], sh);
#endif
#if defined(TARGET_PPC64)
        mb += 32;
        me += 32;
#endif
        tcg_gen_andi_tl(cpu_gpr[rA(s->opcode)], t0, MASK(mb, me));
        tcg_temp_free(t0);
    }
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* rlwnm & rlwnm. */
static void gen_rlwnm(DisasContext *s)
{
    uint32_t mb, me;
    TCGv t0;
#if defined(TARGET_PPC64)
    TCGv_i32 t1, t2;
#endif

    mb = MB(s->opcode);
    me = ME(s->opcode);
    t0 = tcg_temp_new();
    tcg_gen_andi_tl(t0, cpu_gpr[rB(s->opcode)], 0x1f);
#if defined(TARGET_PPC64)
    t1 = tcg_temp_new_i32();
    t2 = tcg_temp_new_i32();
    tcg_gen_trunc_i64_i32(t1, cpu_gpr[rS(s->opcode)]);
    tcg_gen_trunc_i64_i32(t2, t0);
    tcg_gen_rotl_i32(t1, t1, t2);
    tcg_gen_extu_i32_i64(t0, t1);
    tcg_temp_free_i32(t1);
    tcg_temp_free_i32(t2);
#else
    tcg_gen_rotl_i32(t0, cpu_gpr[rS(s->opcode)], t0);
#endif
    if(unlikely(mb != 0 || me != 31)) {
#if defined(TARGET_PPC64)
        mb += 32;
        me += 32;
#endif
        tcg_gen_andi_tl(cpu_gpr[rA(s->opcode)], t0, MASK(mb, me));
    } else {
        tcg_gen_mov_tl(cpu_gpr[rA(s->opcode)], t0);
    }
    tcg_temp_free(t0);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

#if defined(TARGET_PPC64)

#define GEN_PPC64_R2(name, opc1, opc2)                \
    static void glue(gen_, name##0)(DisasContext * s) \
    {                                                 \
        gen_##name(s, 0);                             \
    }                                                 \
                                                      \
    static void glue(gen_, name##1)(DisasContext * s) \
    {                                                 \
        gen_##name(s, 1);                             \
    }

#define GEN_PPC64_R4(name, opc1, opc2)                \
    static void glue(gen_, name##0)(DisasContext * s) \
    {                                                 \
        gen_##name(s, 0, 0);                          \
    }                                                 \
                                                      \
    static void glue(gen_, name##1)(DisasContext * s) \
    {                                                 \
        gen_##name(s, 0, 1);                          \
    }                                                 \
                                                      \
    static void glue(gen_, name##2)(DisasContext * s) \
    {                                                 \
        gen_##name(s, 1, 0);                          \
    }                                                 \
                                                      \
    static void glue(gen_, name##3)(DisasContext * s) \
    {                                                 \
        gen_##name(s, 1, 1);                          \
    }

static inline void gen_rldnm(DisasContext *s, uint32_t mb, uint32_t me)
{
    TCGv t0;

    mb = MB(s->opcode);
    me = ME(s->opcode);
    t0 = tcg_temp_new();
    tcg_gen_andi_tl(t0, cpu_gpr[rB(s->opcode)], 0x3f);
    tcg_gen_rotl_tl(t0, cpu_gpr[rS(s->opcode)], t0);
    if(unlikely(mb != 0 || me != 63)) {
        tcg_gen_andi_tl(cpu_gpr[rA(s->opcode)], t0, MASK(mb, me));
        tcg_gen_andi_tl(cpu_gpr[rA(s->opcode)], t0, MASK(mb, me));
    } else {
        tcg_gen_mov_tl(cpu_gpr[rA(s->opcode)], t0);
        tcg_gen_mov_tl(cpu_gpr[rA(s->opcode)], t0);
    }
    tcg_temp_free(t0);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* rldcl - rldcl. */
static inline void gen_rldcl(DisasContext *s, int mbn)
{
    uint32_t mb;

    mb = MB(s->opcode) | (mbn << 5);
    gen_rldnm(s, mb, 63);
}

static inline void gen_rldinm(DisasContext *s, uint32_t mb, uint32_t me, uint32_t sh)
{
    if(likely(sh != 0 && mb == 0 && me == (63 - sh))) {
        tcg_gen_shli_tl(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)], sh);
    } else if(likely(sh != 0 && me == 63 && sh == (64 - mb))) {
        tcg_gen_shri_tl(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)], mb);
    } else {
        TCGv t0 = tcg_temp_new();
        tcg_gen_rotli_tl(t0, cpu_gpr[rS(s->opcode)], sh);
        if(likely(mb == 0 && me == 63)) {
            tcg_gen_mov_tl(cpu_gpr[rA(s->opcode)], t0);
        } else {
            tcg_gen_andi_tl(cpu_gpr[rA(s->opcode)], t0, MASK(mb, me));
        }
        tcg_temp_free(t0);
    }
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}
/* rldicl - rldicl. */
static inline void gen_rldicl(DisasContext *s, int mbn, int shn)
{
    uint32_t sh, mb;

    sh = SH(s->opcode) | (shn << 5);
    mb = MB(s->opcode) | (mbn << 5);
    gen_rldinm(s, mb, 63, sh);
}
/* rldicr - rldicr. */
static inline void gen_rldicr(DisasContext *s, int men, int shn)
{
    uint32_t sh, me;

    sh = SH(s->opcode) | (shn << 5);
    me = MB(s->opcode) | (men << 5);
    gen_rldinm(s, 0, me, sh);
}
/* rldic - rldic. */
static inline void gen_rldic(DisasContext *s, int mbn, int shn)
{
    uint32_t sh, mb;

    sh = SH(s->opcode) | (shn << 5);
    mb = MB(s->opcode) | (mbn << 5);
    gen_rldinm(s, mb, 63 - sh, sh);
}
/* rldcr - rldcr. */
static inline void gen_rldcr(DisasContext *s, int men)
{
    uint32_t me;

    me = MB(s->opcode) | (men << 5);
    gen_rldnm(s, 0, me);
}
/* rldimi - rldimi. */
static inline void gen_rldimi(DisasContext *s, int mbn, int shn)
{
    uint32_t sh, mb, me;

    sh = SH(s->opcode) | (shn << 5);
    mb = MB(s->opcode) | (mbn << 5);
    me = 63 - sh;
    if(unlikely(sh == 0 && mb == 0)) {
        tcg_gen_mov_tl(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)]);
    } else {
        TCGv t0, t1;
        target_ulong mask;

        t0 = tcg_temp_new();
        tcg_gen_rotli_tl(t0, cpu_gpr[rS(s->opcode)], sh);
        t1 = tcg_temp_new();
        mask = MASK(mb, me);
        tcg_gen_andi_tl(t0, t0, mask);
        tcg_gen_andi_tl(t1, cpu_gpr[rA(s->opcode)], ~mask);
        tcg_gen_or_tl(cpu_gpr[rA(s->opcode)], t0, t1);
        tcg_temp_free(t0);
        tcg_temp_free(t1);
    }
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

GEN_PPC64_R4(rldicl, 0x1E, 0x00);
GEN_PPC64_R4(rldicr, 0x1E, 0x02);
GEN_PPC64_R4(rldic, 0x1E, 0x04);
GEN_PPC64_R2(rldcl, 0x1E, 0x08);
GEN_PPC64_R2(rldcr, 0x1E, 0x09);
GEN_PPC64_R4(rldimi, 0x1E, 0x06);
#endif

/***                             Integer shift                             ***/

/* slw & slw. */
static void gen_slw(DisasContext *s)
{
    TCGv t0, t1;

    t0 = tcg_temp_new();
    /* AND rS with a mask that is 0 when rB >= 0x20 */
#if defined(TARGET_PPC64)
    tcg_gen_shli_tl(t0, cpu_gpr[rB(s->opcode)], 0x3a);
    tcg_gen_sari_tl(t0, t0, 0x3f);
#else
    tcg_gen_shli_tl(t0, cpu_gpr[rB(s->opcode)], 0x1a);
    tcg_gen_sari_tl(t0, t0, 0x1f);
#endif
    tcg_gen_andc_tl(t0, cpu_gpr[rS(s->opcode)], t0);
    t1 = tcg_temp_new();
    tcg_gen_andi_tl(t1, cpu_gpr[rB(s->opcode)], 0x1f);
    tcg_gen_shl_tl(cpu_gpr[rA(s->opcode)], t0, t1);
    tcg_temp_free(t1);
    tcg_temp_free(t0);
    tcg_gen_ext32u_tl(cpu_gpr[rA(s->opcode)], cpu_gpr[rA(s->opcode)]);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* sraw & sraw. */
static void gen_sraw(DisasContext *s)
{
    gen_helper_sraw(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)], cpu_gpr[rB(s->opcode)]);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* srawi & srawi. */
static void gen_srawi(DisasContext *s)
{
    int sh = SH(s->opcode);
    if(sh != 0) {
        int l1, l2;
        TCGv t0;
        l1 = gen_new_label();
        l2 = gen_new_label();
        t0 = tcg_temp_local_new();
        tcg_gen_ext32s_tl(t0, cpu_gpr[rS(s->opcode)]);
        tcg_gen_brcondi_tl(TCG_COND_GE, t0, 0, l1);
        tcg_gen_andi_tl(t0, cpu_gpr[rS(s->opcode)], (1ULL << sh) - 1);
        tcg_gen_brcondi_tl(TCG_COND_EQ, t0, 0, l1);
        tcg_gen_ori_tl(cpu_xer, cpu_xer, 1 << XER_CA);
        tcg_gen_br(l2);
        gen_set_label(l1);
        tcg_gen_andi_tl(cpu_xer, cpu_xer, ~(1 << XER_CA));
        gen_set_label(l2);
        tcg_gen_ext32s_tl(t0, cpu_gpr[rS(s->opcode)]);
        tcg_gen_sari_tl(cpu_gpr[rA(s->opcode)], t0, sh);
        tcg_temp_free(t0);
    } else {
        tcg_gen_mov_tl(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)]);
        tcg_gen_andi_tl(cpu_xer, cpu_xer, ~(1 << XER_CA));
    }
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* srw & srw. */
static void gen_srw(DisasContext *s)
{
    TCGv t0, t1;

    t0 = tcg_temp_new();
    /* AND rS with a mask that is 0 when rB >= 0x20 */
#if defined(TARGET_PPC64)
    tcg_gen_shli_tl(t0, cpu_gpr[rB(s->opcode)], 0x3a);
    tcg_gen_sari_tl(t0, t0, 0x3f);
#else
    tcg_gen_shli_tl(t0, cpu_gpr[rB(s->opcode)], 0x1a);
    tcg_gen_sari_tl(t0, t0, 0x1f);
#endif
    tcg_gen_andc_tl(t0, cpu_gpr[rS(s->opcode)], t0);
    tcg_gen_ext32u_tl(t0, t0);
    t1 = tcg_temp_new();
    tcg_gen_andi_tl(t1, cpu_gpr[rB(s->opcode)], 0x1f);
    tcg_gen_shr_tl(cpu_gpr[rA(s->opcode)], t0, t1);
    tcg_temp_free(t1);
    tcg_temp_free(t0);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

#if defined(TARGET_PPC64)
/* sld & sld. */
static void gen_sld(DisasContext *s)
{
    TCGv t0, t1;

    t0 = tcg_temp_new();
    /* AND rS with a mask that is 0 when rB >= 0x40 */
    tcg_gen_shli_tl(t0, cpu_gpr[rB(s->opcode)], 0x39);
    tcg_gen_sari_tl(t0, t0, 0x3f);
    tcg_gen_andc_tl(t0, cpu_gpr[rS(s->opcode)], t0);
    t1 = tcg_temp_new();
    tcg_gen_andi_tl(t1, cpu_gpr[rB(s->opcode)], 0x3f);
    tcg_gen_shl_tl(cpu_gpr[rA(s->opcode)], t0, t1);
    tcg_temp_free(t1);
    tcg_temp_free(t0);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* srad & srad. */
static void gen_srad(DisasContext *s)
{
    gen_helper_srad(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)], cpu_gpr[rB(s->opcode)]);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}
/* sradi & sradi. */
static inline void gen_sradi(DisasContext *s, int n)
{
    int sh = SH(s->opcode) + (n << 5);
    if(sh != 0) {
        int l1, l2;
        TCGv t0;
        l1 = gen_new_label();
        l2 = gen_new_label();
        t0 = tcg_temp_local_new();
        tcg_gen_brcondi_tl(TCG_COND_GE, cpu_gpr[rS(s->opcode)], 0, l1);
        tcg_gen_andi_tl(t0, cpu_gpr[rS(s->opcode)], (1ULL << sh) - 1);
        tcg_gen_brcondi_tl(TCG_COND_EQ, t0, 0, l1);
        tcg_gen_ori_tl(cpu_xer, cpu_xer, 1 << XER_CA);
        tcg_gen_br(l2);
        gen_set_label(l1);
        tcg_gen_andi_tl(cpu_xer, cpu_xer, ~(1 << XER_CA));
        gen_set_label(l2);
        tcg_temp_free(t0);
        tcg_gen_sari_tl(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)], sh);
    } else {
        tcg_gen_mov_tl(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)]);
        tcg_gen_andi_tl(cpu_xer, cpu_xer, ~(1 << XER_CA));
    }
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

static void gen_sradi0(DisasContext *s)
{
    gen_sradi(s, 0);
}

static void gen_sradi1(DisasContext *s)
{
    gen_sradi(s, 1);
}

/* srd & srd. */
static void gen_srd(DisasContext *s)
{
    TCGv t0, t1;

    t0 = tcg_temp_new();
    /* AND rS with a mask that is 0 when rB >= 0x40 */
    tcg_gen_shli_tl(t0, cpu_gpr[rB(s->opcode)], 0x39);
    tcg_gen_sari_tl(t0, t0, 0x3f);
    tcg_gen_andc_tl(t0, cpu_gpr[rS(s->opcode)], t0);
    t1 = tcg_temp_new();
    tcg_gen_andi_tl(t1, cpu_gpr[rB(s->opcode)], 0x3f);
    tcg_gen_shr_tl(cpu_gpr[rA(s->opcode)], t0, t1);
    tcg_temp_free(t1);
    tcg_temp_free(t0);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}
#endif

/***                       Floating-Point arithmetic                       ***/
#define _GEN_FLOAT_ACB(name, op, op1, op2, isfloat, set_fprf, type)                                                       \
    static void gen_f##name(DisasContext *s)                                                                              \
    {                                                                                                                     \
        if(unlikely(!s->fpu_enabled)) {                                                                                   \
            gen_exception(s, POWERPC_EXCP_FPU);                                                                           \
            return;                                                                                                       \
        }                                                                                                                 \
        /* NIP cannot be restored if the memory exception comes from an helper */                                         \
        gen_update_nip(s, s->base.pc - 4);                                                                                \
        gen_reset_fpstatus();                                                                                             \
        gen_helper_f##op(cpu_fpr[rD(s->opcode)], cpu_fpr[rA(s->opcode)], cpu_fpr[rC(s->opcode)], cpu_fpr[rB(s->opcode)]); \
        if(isfloat) {                                                                                                     \
            gen_helper_frsp(cpu_fpr[rD(s->opcode)], cpu_fpr[rD(s->opcode)]);                                              \
        }                                                                                                                 \
        gen_compute_fprf(cpu_fpr[rD(s->opcode)], set_fprf, Rc(s->opcode) != 0);                                           \
    }

#define GEN_FLOAT_ACB(name, op2, set_fprf, type)              \
    _GEN_FLOAT_ACB(name, name, 0x3F, op2, 0, set_fprf, type); \
    _GEN_FLOAT_ACB(name##s, name, 0x3B, op2, 1, set_fprf, type);

#define _GEN_FLOAT_AB(name, op, op1, op2, inval, isfloat, set_fprf, type)                         \
    static void gen_f##name(DisasContext *s)                                                      \
    {                                                                                             \
        if(unlikely(!s->fpu_enabled)) {                                                           \
            gen_exception(s, POWERPC_EXCP_FPU);                                                   \
            return;                                                                               \
        }                                                                                         \
        /* NIP cannot be restored if the memory exception comes from an helper */                 \
        gen_update_nip(s, s->base.pc - 4);                                                        \
        gen_reset_fpstatus();                                                                     \
        gen_helper_f##op(cpu_fpr[rD(s->opcode)], cpu_fpr[rA(s->opcode)], cpu_fpr[rB(s->opcode)]); \
        if(isfloat) {                                                                             \
            gen_helper_frsp(cpu_fpr[rD(s->opcode)], cpu_fpr[rD(s->opcode)]);                      \
        }                                                                                         \
        gen_compute_fprf(cpu_fpr[rD(s->opcode)], set_fprf, Rc(s->opcode) != 0);                   \
    }
#define GEN_FLOAT_AB(name, op2, inval, set_fprf, type)              \
    _GEN_FLOAT_AB(name, name, 0x3F, op2, inval, 0, set_fprf, type); \
    _GEN_FLOAT_AB(name##s, name, 0x3B, op2, inval, 1, set_fprf, type);

#define _GEN_FLOAT_AC(name, op, op1, op2, inval, isfloat, set_fprf, type)                         \
    static void gen_f##name(DisasContext *s)                                                      \
    {                                                                                             \
        if(unlikely(!s->fpu_enabled)) {                                                           \
            gen_exception(s, POWERPC_EXCP_FPU);                                                   \
            return;                                                                               \
        }                                                                                         \
        /* NIP cannot be restored if the memory exception comes from an helper */                 \
        gen_update_nip(s, s->base.pc - 4);                                                        \
        gen_reset_fpstatus();                                                                     \
        gen_helper_f##op(cpu_fpr[rD(s->opcode)], cpu_fpr[rA(s->opcode)], cpu_fpr[rC(s->opcode)]); \
        if(isfloat) {                                                                             \
            gen_helper_frsp(cpu_fpr[rD(s->opcode)], cpu_fpr[rD(s->opcode)]);                      \
        }                                                                                         \
        gen_compute_fprf(cpu_fpr[rD(s->opcode)], set_fprf, Rc(s->opcode) != 0);                   \
    }
#define GEN_FLOAT_AC(name, op2, inval, set_fprf, type)              \
    _GEN_FLOAT_AC(name, name, 0x3F, op2, inval, 0, set_fprf, type); \
    _GEN_FLOAT_AC(name##s, name, 0x3B, op2, inval, 1, set_fprf, type);

#define GEN_FLOAT_B(name, op2, op3, set_fprf, type)                               \
    static void gen_f##name(DisasContext *s)                                      \
    {                                                                             \
        if(unlikely(!s->fpu_enabled)) {                                           \
            gen_exception(s, POWERPC_EXCP_FPU);                                   \
            return;                                                               \
        }                                                                         \
        /* NIP cannot be restored if the memory exception comes from an helper */ \
        gen_update_nip(s, s->base.pc - 4);                                        \
        gen_reset_fpstatus();                                                     \
        gen_helper_f##name(cpu_fpr[rD(s->opcode)], cpu_fpr[rB(s->opcode)]);       \
        gen_compute_fprf(cpu_fpr[rD(s->opcode)], set_fprf, Rc(s->opcode) != 0);   \
    }

#define GEN_FLOAT_BS(name, op1, op2, set_fprf, type)                              \
    static void gen_f##name(DisasContext *s)                                      \
    {                                                                             \
        if(unlikely(!s->fpu_enabled)) {                                           \
            gen_exception(s, POWERPC_EXCP_FPU);                                   \
            return;                                                               \
        }                                                                         \
        /* NIP cannot be restored if the memory exception comes from an helper */ \
        gen_update_nip(s, s->base.pc - 4);                                        \
        gen_reset_fpstatus();                                                     \
        gen_helper_f##name(cpu_fpr[rD(s->opcode)], cpu_fpr[rB(s->opcode)]);       \
        gen_compute_fprf(cpu_fpr[rD(s->opcode)], set_fprf, Rc(s->opcode) != 0);   \
    }

/* fadd - fadds */
GEN_FLOAT_AB(add, 0x15, 0x000007C0, 1, PPC_FLOAT);
/* fdiv - fdivs */
GEN_FLOAT_AB(div, 0x12, 0x000007C0, 1, PPC_FLOAT);
/* fmul - fmuls */
GEN_FLOAT_AC(mul, 0x19, 0x0000F800, 1, PPC_FLOAT);

/* fre */
GEN_FLOAT_BS(re, 0x3F, 0x18, 1, PPC_FLOAT_EXT);

/* fres */
GEN_FLOAT_BS(res, 0x3B, 0x18, 1, PPC_FLOAT_FRES);

/* frsqrte */
GEN_FLOAT_BS(rsqrte, 0x3F, 0x1A, 1, PPC_FLOAT_FRSQRTE);

/* frsqrtes */
static void gen_frsqrtes(DisasContext *s)
{
    if(unlikely(!s->fpu_enabled)) {
        gen_exception(s, POWERPC_EXCP_FPU);
        return;
    }
    /* NIP cannot be restored if the memory exception comes from an helper */
    gen_update_nip(s, s->base.pc - 4);
    gen_reset_fpstatus();
    gen_helper_frsqrte(cpu_fpr[rD(s->opcode)], cpu_fpr[rB(s->opcode)]);
    gen_helper_frsp(cpu_fpr[rD(s->opcode)], cpu_fpr[rD(s->opcode)]);
    gen_compute_fprf(cpu_fpr[rD(s->opcode)], 1, Rc(s->opcode) != 0);
}

/* fsel */
_GEN_FLOAT_ACB(sel, sel, 0x3F, 0x17, 0, 0, PPC_FLOAT_FSEL);
/* fsub - fsubs */
GEN_FLOAT_AB(sub, 0x14, 0x000007C0, 1, PPC_FLOAT);
/* Optional: */

/* fsqrt */
static void gen_fsqrt(DisasContext *s)
{
    if(unlikely(!s->fpu_enabled)) {
        gen_exception(s, POWERPC_EXCP_FPU);
        return;
    }
    /* NIP cannot be restored if the memory exception comes from an helper */
    gen_update_nip(s, s->base.pc - 4);
    gen_reset_fpstatus();
    gen_helper_fsqrt(cpu_fpr[rD(s->opcode)], cpu_fpr[rB(s->opcode)]);
    gen_compute_fprf(cpu_fpr[rD(s->opcode)], 1, Rc(s->opcode) != 0);
}

static void gen_fsqrts(DisasContext *s)
{
    if(unlikely(!s->fpu_enabled)) {
        gen_exception(s, POWERPC_EXCP_FPU);
        return;
    }
    /* NIP cannot be restored if the memory exception comes from an helper */
    gen_update_nip(s, s->base.pc - 4);
    gen_reset_fpstatus();
    gen_helper_fsqrt(cpu_fpr[rD(s->opcode)], cpu_fpr[rB(s->opcode)]);
    gen_helper_frsp(cpu_fpr[rD(s->opcode)], cpu_fpr[rD(s->opcode)]);
    gen_compute_fprf(cpu_fpr[rD(s->opcode)], 1, Rc(s->opcode) != 0);
}

/***                     Floating-Point multiply-and-add                   ***/
/* fmadd - fmadds */
GEN_FLOAT_ACB(madd, 0x1D, 1, PPC_FLOAT);
/* fmsub - fmsubs */
GEN_FLOAT_ACB(msub, 0x1C, 1, PPC_FLOAT);
/* fnmadd - fnmadds */
GEN_FLOAT_ACB(nmadd, 0x1F, 1, PPC_FLOAT);
/* fnmsub - fnmsubs */
GEN_FLOAT_ACB(nmsub, 0x1E, 1, PPC_FLOAT);

/***                     Floating-Point round & convert                    ***/
/* fctiw */
GEN_FLOAT_B(ctiw, 0x0E, 0x00, 0, PPC_FLOAT);
/* fctiwz */
GEN_FLOAT_B(ctiwz, 0x0F, 0x00, 0, PPC_FLOAT);
/* frsp */
GEN_FLOAT_B(rsp, 0x0C, 0x00, 1, PPC_FLOAT);
#if defined(TARGET_PPC64)
/* fcfid */
GEN_FLOAT_B(cfid, 0x0E, 0x1A, 1, PPC_64B);
/* fctid */
GEN_FLOAT_B(ctid, 0x0E, 0x19, 0, PPC_64B);
/* fctidz */
GEN_FLOAT_B(ctidz, 0x0F, 0x19, 0, PPC_64B);
#endif

/* frin */
GEN_FLOAT_B(rin, 0x08, 0x0C, 1, PPC_FLOAT_EXT);
/* friz */
GEN_FLOAT_B(riz, 0x08, 0x0D, 1, PPC_FLOAT_EXT);
/* frip */
GEN_FLOAT_B(rip, 0x08, 0x0E, 1, PPC_FLOAT_EXT);
/* frim */
GEN_FLOAT_B(rim, 0x08, 0x0F, 1, PPC_FLOAT_EXT);

/***                         Floating-Point compare                        ***/

/* fcmpo */
static void gen_fcmpo(DisasContext *s)
{
    TCGv_i32 crf;
    if(unlikely(!s->fpu_enabled)) {
        gen_exception(s, POWERPC_EXCP_FPU);
        return;
    }
    /* NIP cannot be restored if the memory exception comes from an helper */
    gen_update_nip(s, s->base.pc - 4);
    gen_reset_fpstatus();
    crf = tcg_const_i32(crfD(s->opcode));
    gen_helper_fcmpo(cpu_fpr[rA(s->opcode)], cpu_fpr[rB(s->opcode)], crf);
    tcg_temp_free_i32(crf);
    gen_helper_float_check_status();
}

/* fcmpu */
static void gen_fcmpu(DisasContext *s)
{
    TCGv_i32 crf;
    if(unlikely(!s->fpu_enabled)) {
        gen_exception(s, POWERPC_EXCP_FPU);
        return;
    }
    /* NIP cannot be restored if the memory exception comes from an helper */
    gen_update_nip(s, s->base.pc - 4);
    gen_reset_fpstatus();
    crf = tcg_const_i32(crfD(s->opcode));
    gen_helper_fcmpu(cpu_fpr[rA(s->opcode)], cpu_fpr[rB(s->opcode)], crf);
    tcg_temp_free_i32(crf);
    gen_helper_float_check_status();
}

/***                         Floating-point move                           ***/
/* fabs */
/* XXX: beware that fabs never checks for NaNs nor update FPSCR */
GEN_FLOAT_B(abs, 0x08, 0x08, 0, PPC_FLOAT);

/* fmr  - fmr. */
/* XXX: beware that fmr never checks for NaNs nor update FPSCR */
static void gen_fmr(DisasContext *s)
{
    if(unlikely(!s->fpu_enabled)) {
        gen_exception(s, POWERPC_EXCP_FPU);
        return;
    }
    tcg_gen_mov_i64(cpu_fpr[rD(s->opcode)], cpu_fpr[rB(s->opcode)]);
    gen_compute_fprf(cpu_fpr[rD(s->opcode)], 0, Rc(s->opcode) != 0);
}

/* fnabs */
/* XXX: beware that fnabs never checks for NaNs nor update FPSCR */
GEN_FLOAT_B(nabs, 0x08, 0x04, 0, PPC_FLOAT);
/* fneg */
/* XXX: beware that fneg never checks for NaNs nor update FPSCR */
GEN_FLOAT_B(neg, 0x08, 0x01, 0, PPC_FLOAT);

/***                  Floating-Point status & ctrl register                ***/

/* mcrfs */
static void gen_mcrfs(DisasContext *s)
{
    int bfa;

    if(unlikely(!s->fpu_enabled)) {
        gen_exception(s, POWERPC_EXCP_FPU);
        return;
    }
    bfa = 4 * (7 - crfS(s->opcode));
    tcg_gen_shri_i32(cpu_crf[crfD(s->opcode)], cpu_fpscr, bfa);
    tcg_gen_andi_i32(cpu_crf[crfD(s->opcode)], cpu_crf[crfD(s->opcode)], 0xf);
    tcg_gen_andi_i32(cpu_fpscr, cpu_fpscr, ~(0xF << bfa));
}

/* mffs */
static void gen_mffs(DisasContext *s)
{
    if(unlikely(!s->fpu_enabled)) {
        gen_exception(s, POWERPC_EXCP_FPU);
        return;
    }
    gen_reset_fpstatus();
    tcg_gen_extu_i32_i64(cpu_fpr[rD(s->opcode)], cpu_fpscr);
    gen_compute_fprf(cpu_fpr[rD(s->opcode)], 0, Rc(s->opcode) != 0);
}

/* mtfsb0 */
static void gen_mtfsb0(DisasContext *s)
{
    uint8_t crb;

    if(unlikely(!s->fpu_enabled)) {
        gen_exception(s, POWERPC_EXCP_FPU);
        return;
    }
    crb = 31 - crbD(s->opcode);
    gen_reset_fpstatus();
    if(likely(crb != FPSCR_FEX && crb != FPSCR_VX)) {
        TCGv_i32 t0;
        /* NIP cannot be restored if the memory exception comes from an helper */
        gen_update_nip(s, s->base.pc - 4);
        t0 = tcg_const_i32(crb);
        gen_helper_fpscr_clrbit(t0);
        tcg_temp_free_i32(t0);
    }
    if(unlikely(Rc(s->opcode) != 0)) {
        tcg_gen_shri_i32(cpu_crf[1], cpu_fpscr, FPSCR_OX);
    }
}

/* mtfsb1 */
static void gen_mtfsb1(DisasContext *s)
{
    uint8_t crb;

    if(unlikely(!s->fpu_enabled)) {
        gen_exception(s, POWERPC_EXCP_FPU);
        return;
    }
    crb = 31 - crbD(s->opcode);
    gen_reset_fpstatus();
    /* XXX: we pretend we can only do IEEE floating-point computations */
    if(likely(crb != FPSCR_FEX && crb != FPSCR_VX && crb != FPSCR_NI)) {
        TCGv_i32 t0;
        /* NIP cannot be restored if the memory exception comes from an helper */
        gen_update_nip(s, s->base.pc - 4);
        t0 = tcg_const_i32(crb);
        gen_helper_fpscr_setbit(t0);
        tcg_temp_free_i32(t0);
    }
    if(unlikely(Rc(s->opcode) != 0)) {
        tcg_gen_shri_i32(cpu_crf[1], cpu_fpscr, FPSCR_OX);
    }
    /* We can raise a differed exception */
    gen_helper_float_check_status();
}

/* mtfsf */
static void gen_mtfsf(DisasContext *s)
{
    TCGv_i32 t0;
    int L = s->opcode & 0x02000000;

    if(unlikely(!s->fpu_enabled)) {
        gen_exception(s, POWERPC_EXCP_FPU);
        return;
    }
    /* NIP cannot be restored if the memory exception comes from an helper */
    gen_update_nip(s, s->base.pc - 4);
    gen_reset_fpstatus();
    if(L) {
        t0 = tcg_const_i32(0xff);
    } else {
        t0 = tcg_const_i32(FM(s->opcode));
    }
    gen_helper_store_fpscr(cpu_fpr[rB(s->opcode)], t0);
    tcg_temp_free_i32(t0);
    if(unlikely(Rc(s->opcode) != 0)) {
        tcg_gen_shri_i32(cpu_crf[1], cpu_fpscr, FPSCR_OX);
    }
    /* We can raise a differed exception */
    gen_helper_float_check_status();
}

/* mtfsfi */
static void gen_mtfsfi(DisasContext *s)
{
    int bf, sh;
    TCGv_i64 t0;
    TCGv_i32 t1;

    if(unlikely(!s->fpu_enabled)) {
        gen_exception(s, POWERPC_EXCP_FPU);
        return;
    }
    bf = crbD(s->opcode) >> 2;
    sh = 7 - bf;
    /* NIP cannot be restored if the memory exception comes from an helper */
    gen_update_nip(s, s->base.pc - 4);
    gen_reset_fpstatus();
    t0 = tcg_const_i64(FPIMM(s->opcode) << (4 * sh));
    t1 = tcg_const_i32(1 << sh);
    gen_helper_store_fpscr(t0, t1);
    tcg_temp_free_i64(t0);
    tcg_temp_free_i32(t1);
    if(unlikely(Rc(s->opcode) != 0)) {
        tcg_gen_shri_i32(cpu_crf[1], cpu_fpscr, FPSCR_OX);
    }
    /* We can raise a differed exception */
    gen_helper_float_check_status();
}

/***                           Addressing modes                            ***/
/* Register indirect with immediate index : EA = (rA|0) + SIMM */
static inline void gen_addr_imm_index(DisasContext *s, TCGv EA, target_long maskl)
{
    target_long simm = SIMM(s->opcode);

    simm &= ~maskl;
    if(rA(s->opcode) == 0) {
#if defined(TARGET_PPC64)
        if(!s->sf_mode) {
            tcg_gen_movi_tl(EA, (uint32_t)simm);
        } else
#endif
            tcg_gen_movi_tl(EA, simm);
    } else if(likely(simm != 0)) {
        tcg_gen_addi_tl(EA, cpu_gpr[rA(s->opcode)], simm);
#if defined(TARGET_PPC64)
        if(!s->sf_mode) {
            tcg_gen_ext32u_tl(EA, EA);
        }
#endif
    } else {
#if defined(TARGET_PPC64)
        if(!s->sf_mode) {
            tcg_gen_ext32u_tl(EA, cpu_gpr[rA(s->opcode)]);
        } else
#endif
            tcg_gen_mov_tl(EA, cpu_gpr[rA(s->opcode)]);
    }
}

static inline void gen_addr_reg_index(DisasContext *s, TCGv EA)
{
    if(rA(s->opcode) == 0) {
#if defined(TARGET_PPC64)
        if(!s->sf_mode) {
            tcg_gen_ext32u_tl(EA, cpu_gpr[rB(s->opcode)]);
        } else
#endif
            tcg_gen_mov_tl(EA, cpu_gpr[rB(s->opcode)]);
    } else {
        tcg_gen_add_tl(EA, cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)]);
#if defined(TARGET_PPC64)
        if(!s->sf_mode) {
            tcg_gen_ext32u_tl(EA, EA);
        }
#endif
    }
}

static inline void gen_addr_register(DisasContext *s, TCGv EA)
{
    if(rA(s->opcode) == 0) {
        tcg_gen_movi_tl(EA, 0);
    } else {
#if defined(TARGET_PPC64)
        if(!s->sf_mode) {
            tcg_gen_ext32u_tl(EA, cpu_gpr[rA(s->opcode)]);
        } else
#endif
            tcg_gen_mov_tl(EA, cpu_gpr[rA(s->opcode)]);
    }
}

static inline void gen_addr_add(DisasContext *s, TCGv ret, TCGv arg1, target_long val)
{
    tcg_gen_addi_tl(ret, arg1, val);
#if defined(TARGET_PPC64)
    if(!s->sf_mode) {
        tcg_gen_ext32u_tl(ret, ret);
    }
#endif
}

static inline void gen_check_align(DisasContext *s, TCGv EA, int mask)
{
    int l1 = gen_new_label();
    TCGv t0 = tcg_temp_new();
    TCGv_i32 t1, t2;
    /* NIP cannot be restored if the memory exception comes from an helper */
    gen_update_nip(s, s->base.pc - 4);
    tcg_gen_andi_tl(t0, EA, mask);
    tcg_gen_brcondi_tl(TCG_COND_EQ, t0, 0, l1);
    t1 = tcg_const_i32(POWERPC_EXCP_ALIGN);
    t2 = tcg_const_i32(0);
    gen_helper_raise_exception_err(t1, t2);
    tcg_temp_free_i32(t1);
    tcg_temp_free_i32(t2);
    gen_set_label(l1);
    tcg_temp_free(t0);
}

/***                             Integer load                              ***/
static inline void gen_qemu_ld8u(DisasContext *s, TCGv arg1, TCGv arg2)
{
    gen_sync_pc(s);
    tcg_gen_qemu_ld8u(arg1, arg2, s->base.mem_idx);
}

static inline void gen_qemu_ld16u(DisasContext *s, TCGv arg1, TCGv arg2)
{
    gen_sync_pc(s);
    tcg_gen_qemu_ld16u(arg1, arg2, s->base.mem_idx);
    if(unlikely(s->le_mode)) {
        tcg_gen_bswap16_tl(arg1, arg1);
    }
}

static inline void gen_qemu_ld16s(DisasContext *s, TCGv arg1, TCGv arg2)
{
    gen_sync_pc(s);
    if(unlikely(s->le_mode)) {
        tcg_gen_qemu_ld16u(arg1, arg2, s->base.mem_idx);
        tcg_gen_bswap16_tl(arg1, arg1);
        tcg_gen_ext16s_tl(arg1, arg1);
    } else {
        tcg_gen_qemu_ld16s(arg1, arg2, s->base.mem_idx);
    }
}

static inline void gen_qemu_ld32u(DisasContext *s, TCGv arg1, TCGv arg2)
{
    gen_sync_pc(s);
    tcg_gen_qemu_ld32u(arg1, arg2, s->base.mem_idx);
    if(unlikely(s->le_mode)) {
        tcg_gen_bswap32_tl(arg1, arg1);
    }
}

#if defined(TARGET_PPC64)
static inline void gen_qemu_ld32s(DisasContext *s, TCGv arg1, TCGv arg2)
{
    gen_sync_pc(s);
    if(unlikely(s->le_mode)) {
        tcg_gen_qemu_ld32u(arg1, arg2, s->base.mem_idx);
        tcg_gen_bswap32_tl(arg1, arg1);
        tcg_gen_ext32s_tl(arg1, arg1);
    } else {
        tcg_gen_qemu_ld32s(arg1, arg2, s->base.mem_idx);
    }
}
#endif

static inline void gen_qemu_ld64(DisasContext *s, TCGv_i64 arg1, TCGv arg2)
{
    gen_sync_pc(s);
    tcg_gen_qemu_ld64(arg1, arg2, s->base.mem_idx);
    if(unlikely(s->le_mode)) {
        tcg_gen_bswap64_i64(arg1, arg1);
    }
}

static inline void gen_qemu_st8(DisasContext *s, TCGv arg1, TCGv arg2)
{
    gen_sync_pc(s);
    tcg_gen_qemu_st8(arg1, arg2, s->base.mem_idx);
}

static inline void gen_qemu_st16(DisasContext *s, TCGv arg1, TCGv arg2)
{
    gen_sync_pc(s);
    if(unlikely(s->le_mode)) {
        TCGv t0 = tcg_temp_local_new();
        tcg_gen_ext16u_tl(t0, arg1);
        tcg_gen_bswap16_tl(t0, t0);
        tcg_gen_qemu_st16(t0, arg2, s->base.mem_idx);
        tcg_temp_free(t0);
    } else {
        tcg_gen_qemu_st16(arg1, arg2, s->base.mem_idx);
    }
}

static inline void gen_qemu_st32(DisasContext *s, TCGv arg1, TCGv arg2)
{
    gen_sync_pc(s);
    if(unlikely(s->le_mode)) {
        TCGv t0 = tcg_temp_local_new();
        tcg_gen_ext32u_tl(t0, arg1);
        tcg_gen_bswap32_tl(t0, t0);
        tcg_gen_qemu_st32(t0, arg2, s->base.mem_idx);
        tcg_temp_free(t0);
    } else {
        tcg_gen_qemu_st32(arg1, arg2, s->base.mem_idx);
    }
}

static inline void gen_qemu_st64(DisasContext *s, TCGv_i64 arg1, TCGv arg2)
{
    gen_sync_pc(s);
    if(unlikely(s->le_mode)) {
        TCGv_i64 t0 = tcg_temp_local_new_i64();
        tcg_gen_bswap64_i64(t0, arg1);
        tcg_gen_qemu_st64(t0, arg2, s->base.mem_idx);
        tcg_temp_free_i64(t0);
    } else {
        tcg_gen_qemu_st64(arg1, arg2, s->base.mem_idx);
    }
}

#define GEN_LD(name, ldop, opc, type)                   \
    static void glue(gen_, name)(DisasContext * s)      \
    {                                                   \
        TCGv EA;                                        \
        gen_sync_pc(s);                                 \
        gen_set_access_type(s, ACCESS_INT);             \
        EA = tcg_temp_new();                            \
        gen_addr_imm_index(s, EA, 0);                   \
        gen_qemu_##ldop(s, cpu_gpr[rD(s->opcode)], EA); \
        tcg_temp_free(EA);                              \
    }

#define GEN_LDU(name, ldop, opc, type)                                       \
    static void glue(gen_, name##u)(DisasContext * s)                        \
    {                                                                        \
        TCGv EA;                                                             \
        gen_sync_pc(s);                                                      \
        if(unlikely(rA(s->opcode) == 0 || rA(s->opcode) == rD(s->opcode))) { \
            gen_inval_exception(s, POWERPC_EXCP_INVAL_INVAL);                \
            return;                                                          \
        }                                                                    \
        gen_set_access_type(s, ACCESS_INT);                                  \
        EA = tcg_temp_new();                                                 \
        if(type == PPC_64B)                                                  \
            gen_addr_imm_index(s, EA, 0x03);                                 \
        else                                                                 \
            gen_addr_imm_index(s, EA, 0);                                    \
        gen_qemu_##ldop(s, cpu_gpr[rD(s->opcode)], EA);                      \
        tcg_gen_mov_tl(cpu_gpr[rA(s->opcode)], EA);                          \
        tcg_temp_free(EA);                                                   \
    }

#define GEN_LDUX(name, ldop, opc2, opc3, type)                               \
    static void glue(gen_, name##ux)(DisasContext * s)                       \
    {                                                                        \
        TCGv EA;                                                             \
        gen_sync_pc(s);                                                      \
        if(unlikely(rA(s->opcode) == 0 || rA(s->opcode) == rD(s->opcode))) { \
            gen_inval_exception(s, POWERPC_EXCP_INVAL_INVAL);                \
            return;                                                          \
        }                                                                    \
        gen_set_access_type(s, ACCESS_INT);                                  \
        EA = tcg_temp_new();                                                 \
        gen_addr_reg_index(s, EA);                                           \
        gen_qemu_##ldop(s, cpu_gpr[rD(s->opcode)], EA);                      \
        tcg_gen_mov_tl(cpu_gpr[rA(s->opcode)], EA);                          \
        tcg_temp_free(EA);                                                   \
    }

#define GEN_LDX(name, ldop, opc2, opc3, type)           \
    static void glue(gen_, name##x)(DisasContext * s)   \
    {                                                   \
        TCGv EA;                                        \
        gen_sync_pc(s);                                 \
        gen_set_access_type(s, ACCESS_INT);             \
        EA = tcg_temp_new();                            \
        gen_addr_reg_index(s, EA);                      \
        gen_qemu_##ldop(s, cpu_gpr[rD(s->opcode)], EA); \
        tcg_temp_free(EA);                              \
    }

#define GEN_LDS(name, ldop, op, type)            \
    GEN_LD(name, ldop, op | 0x20, type);         \
    GEN_LDU(name, ldop, op | 0x21, type);        \
    GEN_LDUX(name, ldop, 0x17, op | 0x01, type); \
    GEN_LDX(name, ldop, 0x17, op | 0x00, type)

/* lbz lbzu lbzux lbzx */
GEN_LDS(lbz, ld8u, 0x02, PPC_INTEGER);
/* lha lhau lhaux lhax */
GEN_LDS(lha, ld16s, 0x0A, PPC_INTEGER);
/* lhz lhzu lhzux lhzx */
GEN_LDS(lhz, ld16u, 0x08, PPC_INTEGER);
/* lwz lwzu lwzux lwzx */
GEN_LDS(lwz, ld32u, 0x00, PPC_INTEGER);
#if defined(TARGET_PPC64)
/* lwaux */
GEN_LDUX(lwa, ld32s, 0x15, 0x0B, PPC_64B);
/* lwax */
GEN_LDX(lwa, ld32s, 0x15, 0x0A, PPC_64B);
/* ldux */
GEN_LDUX(ld, ld64, 0x15, 0x01, PPC_64B);
/* ldx */
GEN_LDX(ld, ld64, 0x15, 0x00, PPC_64B);

static void gen_ld(DisasContext *s)
{
    TCGv EA;
    if(Rc(s->opcode)) {
        if(unlikely(rA(s->opcode) == 0 || rA(s->opcode) == rD(s->opcode))) {
            gen_inval_exception(s, POWERPC_EXCP_INVAL_INVAL);
            return;
        }
    }
    gen_set_access_type(s, ACCESS_INT);
    EA = tcg_temp_new();
    gen_addr_imm_index(s, EA, 0x03);
    if(s->opcode & 0x02) {
        /* lwa (lwau is undefined) */
        gen_qemu_ld32s(s, cpu_gpr[rD(s->opcode)], EA);
    } else {
        /* ld - ldu */
        gen_qemu_ld64(s, cpu_gpr[rD(s->opcode)], EA);
    }
    if(Rc(s->opcode)) {
        tcg_gen_mov_tl(cpu_gpr[rA(s->opcode)], EA);
    }
    tcg_temp_free(EA);
}

/* lq */
static void gen_lq(DisasContext *s)
{
    int ra, rd;
    TCGv EA;

    /* Restore CPU state */
    if(unlikely(s->base.mem_idx == 0)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    ra = rA(s->opcode);
    rd = rD(s->opcode);
    if(unlikely((rd & 1) || rd == ra)) {
        gen_inval_exception(s, POWERPC_EXCP_INVAL_INVAL);
        return;
    }
    if(unlikely(s->le_mode)) {
        /* Little-endian mode is not handled */
        gen_exception_err(s, POWERPC_EXCP_ALIGN, POWERPC_EXCP_ALIGN_LE);
        return;
    }
    gen_set_access_type(s, ACCESS_INT);
    EA = tcg_temp_new();
    gen_addr_imm_index(s, EA, 0x0F);
    gen_qemu_ld64(s, cpu_gpr[rd], EA);
    gen_addr_add(s, EA, EA, 8);
    gen_qemu_ld64(s, cpu_gpr[rd + 1], EA);
    tcg_temp_free(EA);
}
#endif

/***                              Integer store                            ***/
#define GEN_ST(name, stop, opc, type)                   \
    static void glue(gen_, name)(DisasContext * s)      \
    {                                                   \
        TCGv EA;                                        \
        gen_sync_pc(s);                                 \
        gen_set_access_type(s, ACCESS_INT);             \
        EA = tcg_temp_local_new();                      \
        gen_addr_imm_index(s, EA, 0);                   \
        gen_qemu_##stop(s, cpu_gpr[rS(s->opcode)], EA); \
        tcg_temp_free(EA);                              \
    }

#define GEN_STU(name, stop, opc, type)                        \
    static void glue(gen_, stop##u)(DisasContext * s)         \
    {                                                         \
        TCGv EA;                                              \
        gen_sync_pc(s);                                       \
        if(unlikely(rA(s->opcode) == 0)) {                    \
            gen_inval_exception(s, POWERPC_EXCP_INVAL_INVAL); \
            return;                                           \
        }                                                     \
        gen_set_access_type(s, ACCESS_INT);                   \
        EA = tcg_temp_local_new();                            \
        if(type == PPC_64B)                                   \
            gen_addr_imm_index(s, EA, 0x03);                  \
        else                                                  \
            gen_addr_imm_index(s, EA, 0);                     \
        gen_qemu_##stop(s, cpu_gpr[rS(s->opcode)], EA);       \
        tcg_gen_mov_tl(cpu_gpr[rA(s->opcode)], EA);           \
        tcg_temp_free(EA);                                    \
    }

#define GEN_STUX(name, stop, opc2, opc3, type)                \
    static void glue(gen_, name##ux)(DisasContext * s)        \
    {                                                         \
        TCGv EA;                                              \
        gen_sync_pc(s);                                       \
        if(unlikely(rA(s->opcode) == 0)) {                    \
            gen_inval_exception(s, POWERPC_EXCP_INVAL_INVAL); \
            return;                                           \
        }                                                     \
        gen_set_access_type(s, ACCESS_INT);                   \
        EA = tcg_temp_local_new();                            \
        gen_addr_reg_index(s, EA);                            \
        gen_qemu_##stop(s, cpu_gpr[rS(s->opcode)], EA);       \
        tcg_gen_mov_tl(cpu_gpr[rA(s->opcode)], EA);           \
        tcg_temp_free(EA);                                    \
    }

#define GEN_STX(name, stop, opc2, opc3, type)           \
    static void glue(gen_, name##x)(DisasContext * s)   \
    {                                                   \
        TCGv EA;                                        \
        gen_sync_pc(s);                                 \
        gen_set_access_type(s, ACCESS_INT);             \
        EA = tcg_temp_local_new();                      \
        gen_addr_reg_index(s, EA);                      \
        gen_qemu_##stop(s, cpu_gpr[rS(s->opcode)], EA); \
        tcg_temp_free(EA);                              \
    }

#define GEN_STS(name, stop, op, type)            \
    GEN_ST(name, stop, op | 0x20, type);         \
    GEN_STU(name, stop, op | 0x21, type);        \
    GEN_STUX(name, stop, 0x17, op | 0x01, type); \
    GEN_STX(name, stop, 0x17, op | 0x00, type)

/* stb stbu stbux stbx */
GEN_STS(stb, st8, 0x06, PPC_INTEGER);
/* sth sthu sthux sthx */
GEN_STS(sth, st16, 0x0C, PPC_INTEGER);
/* stw stwu stwux stwx */
GEN_STS(stw, st32, 0x04, PPC_INTEGER);
#if defined(TARGET_PPC64)
GEN_STUX(std, st64, 0x15, 0x05, PPC_64B);
GEN_STX(std, st64, 0x15, 0x04, PPC_64B);

static void gen_std(DisasContext *s)
{
    int rs;
    TCGv EA;

    rs = rS(s->opcode);
    if((s->opcode & 0x3) == 0x2) {
        /* stq */
        if(unlikely(s->base.mem_idx == 0)) {
            gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
            return;
        }
        if(unlikely(rs & 1)) {
            gen_inval_exception(s, POWERPC_EXCP_INVAL_INVAL);
            return;
        }
        if(unlikely(s->le_mode)) {
            /* Little-endian mode is not handled */
            gen_exception_err(s, POWERPC_EXCP_ALIGN, POWERPC_EXCP_ALIGN_LE);
            return;
        }
        gen_set_access_type(s, ACCESS_INT);
        EA = tcg_temp_local_new();
        gen_addr_imm_index(s, EA, 0x03);
        gen_qemu_st64(s, cpu_gpr[rs], EA);
        gen_addr_add(s, EA, EA, 8);
        gen_qemu_st64(s, cpu_gpr[rs + 1], EA);
        tcg_temp_free(EA);
    } else {
        /* std / stdu */
        if(Rc(s->opcode)) {
            if(unlikely(rA(s->opcode) == 0)) {
                gen_inval_exception(s, POWERPC_EXCP_INVAL_INVAL);
                return;
            }
        }
        gen_set_access_type(s, ACCESS_INT);
        EA = tcg_temp_local_new();
        gen_addr_imm_index(s, EA, 0x03);
        gen_qemu_st64(s, cpu_gpr[rs], EA);
        if(Rc(s->opcode)) {
            tcg_gen_mov_tl(cpu_gpr[rA(s->opcode)], EA);
        }
        tcg_temp_free(EA);
    }
}
#endif
/***                Integer load and store with byte reverse               ***/
/* lhbrx */
static inline void gen_qemu_ld16ur(DisasContext *s, TCGv arg1, TCGv arg2)
{
    tcg_gen_qemu_ld16u(arg1, arg2, s->base.mem_idx);
    if(likely(s->le_mode)) {
        tcg_gen_bswap16_tl(arg1, arg1);
    }
}
GEN_LDX(lhbr, ld16ur, 0x16, 0x18, PPC_INTEGER);

/* lwbrx */
static inline void gen_qemu_ld32ur(DisasContext *s, TCGv arg1, TCGv arg2)
{
    tcg_gen_qemu_ld32u(arg1, arg2, s->base.mem_idx);
    if(likely(!s->le_mode)) {
        tcg_gen_bswap32_tl(arg1, arg1);
    }
}
GEN_LDX(lwbr, ld32ur, 0x16, 0x10, PPC_INTEGER);

/* sthbrx */
static inline void gen_qemu_st16r(DisasContext *s, TCGv arg1, TCGv arg2)
{
    if(likely(!s->le_mode)) {
        TCGv t0 = tcg_temp_local_new();
        tcg_gen_ext16u_tl(t0, arg1);
        tcg_gen_bswap16_tl(t0, t0);
        tcg_gen_qemu_st16(t0, arg2, s->base.mem_idx);
        tcg_temp_free(t0);
    } else {
        tcg_gen_qemu_st16(arg1, arg2, s->base.mem_idx);
    }
}
GEN_STX(sthbr, st16r, 0x16, 0x1C, PPC_INTEGER);

/* stwbrx */
static inline void gen_qemu_st32r(DisasContext *s, TCGv arg1, TCGv arg2)
{
    if(likely(!s->le_mode)) {
        TCGv t0 = tcg_temp_local_new();
        tcg_gen_ext32u_tl(t0, arg1);
        tcg_gen_bswap32_tl(t0, t0);
        tcg_gen_qemu_st32(t0, arg2, s->base.mem_idx);
        tcg_temp_free(t0);
    } else {
        tcg_gen_qemu_st32(arg1, arg2, s->base.mem_idx);
    }
}
GEN_STX(stwbr, st32r, 0x16, 0x14, PPC_INTEGER);

/***                    Integer load and store multiple                    ***/

/* lmw */
static void gen_lmw(DisasContext *s)
{
    TCGv t0;
    TCGv_i32 t1;
    gen_set_access_type(s, ACCESS_INT);
    /* NIP cannot be restored if the memory exception comes from an helper */
    gen_update_nip(s, s->base.pc - 4);
    t0 = tcg_temp_new();
    t1 = tcg_const_i32(rD(s->opcode));
    gen_addr_imm_index(s, t0, 0);
    gen_helper_lmw(t0, t1);
    tcg_temp_free(t0);
    tcg_temp_free_i32(t1);
}

/* stmw */
static void gen_stmw(DisasContext *s)
{
    TCGv t0;
    TCGv_i32 t1;
    gen_set_access_type(s, ACCESS_INT);
    /* NIP cannot be restored if the memory exception comes from an helper */
    gen_update_nip(s, s->base.pc - 4);
    t0 = tcg_temp_new();
    t1 = tcg_const_i32(rS(s->opcode));
    gen_addr_imm_index(s, t0, 0);
    gen_helper_stmw(t0, t1);
    tcg_temp_free(t0);
    tcg_temp_free_i32(t1);
}

/***                    Integer load and store strings                     ***/

/* lswi */
/* PowerPC32 specification says we must generate an exception if
 * rA is in the range of registers to be loaded.
 * In an other hand, IBM says this is valid, but rA won't be loaded.
 * For now, I'll follow the spec...
 */
static void gen_lswi(DisasContext *s)
{
    TCGv t0;
    TCGv_i32 t1, t2;
    int nb = NB(s->opcode);
    int start = rD(s->opcode);
    int ra = rA(s->opcode);
    int nr;

    if(nb == 0) {
        nb = 32;
    }
    nr = nb / 4;
    if(unlikely(((start + nr) > 32 && start <= ra && (start + nr - 32) > ra) ||
                ((start + nr) <= 32 && start <= ra && (start + nr) > ra))) {
        gen_inval_exception(s, POWERPC_EXCP_INVAL_LSWX);
        return;
    }
    gen_set_access_type(s, ACCESS_INT);
    /* NIP cannot be restored if the memory exception comes from an helper */
    gen_update_nip(s, s->base.pc - 4);
    t0 = tcg_temp_new();
    gen_addr_register(s, t0);
    t1 = tcg_const_i32(nb);
    t2 = tcg_const_i32(start);
    gen_helper_lsw(t0, t1, t2);
    tcg_temp_free(t0);
    tcg_temp_free_i32(t1);
    tcg_temp_free_i32(t2);
}

/* lswx */
static void gen_lswx(DisasContext *s)
{
    TCGv t0;
    TCGv_i32 t1, t2, t3;
    gen_set_access_type(s, ACCESS_INT);
    /* NIP cannot be restored if the memory exception comes from an helper */
    gen_update_nip(s, s->base.pc - 4);
    t0 = tcg_temp_new();
    gen_addr_reg_index(s, t0);
    t1 = tcg_const_i32(rD(s->opcode));
    t2 = tcg_const_i32(rA(s->opcode));
    t3 = tcg_const_i32(rB(s->opcode));
    gen_helper_lswx(t0, t1, t2, t3);
    tcg_temp_free(t0);
    tcg_temp_free_i32(t1);
    tcg_temp_free_i32(t2);
    tcg_temp_free_i32(t3);
}

/* stswi */
static void gen_stswi(DisasContext *s)
{
    TCGv t0;
    TCGv_i32 t1, t2;
    int nb = NB(s->opcode);
    gen_set_access_type(s, ACCESS_INT);
    /* NIP cannot be restored if the memory exception comes from an helper */
    gen_update_nip(s, s->base.pc - 4);
    t0 = tcg_temp_new();
    gen_addr_register(s, t0);
    if(nb == 0) {
        nb = 32;
    }
    t1 = tcg_const_i32(nb);
    t2 = tcg_const_i32(rS(s->opcode));
    gen_helper_stsw(t0, t1, t2);
    tcg_temp_free(t0);
    tcg_temp_free_i32(t1);
    tcg_temp_free_i32(t2);
}

/* stswx */
static void gen_stswx(DisasContext *s)
{
    TCGv t0;
    TCGv_i32 t1, t2;
    gen_set_access_type(s, ACCESS_INT);
    /* NIP cannot be restored if the memory exception comes from an helper */
    gen_update_nip(s, s->base.pc - 4);
    t0 = tcg_temp_new();
    gen_addr_reg_index(s, t0);
    t1 = tcg_temp_new_i32();
    tcg_gen_trunc_tl_i32(t1, cpu_xer);
    tcg_gen_andi_i32(t1, t1, 0x7F);
    t2 = tcg_const_i32(rS(s->opcode));
    gen_helper_stsw(t0, t1, t2);
    tcg_temp_free(t0);
    tcg_temp_free_i32(t1);
    tcg_temp_free_i32(t2);
}

/***                        Memory synchronisation                         ***/
/* eieio */
static void gen_eieio(DisasContext *s) { }

/* isync */
static void gen_isync(DisasContext *s)
{
    gen_stop_exception(s);
}

/* lwarx */
static void gen_lwarx(DisasContext *s)
{
    TCGv t0;
    TCGv gpr = cpu_gpr[rD(s->opcode)];
    gen_set_access_type(s, ACCESS_RES);
    t0 = tcg_temp_local_new();
    gen_addr_reg_index(s, t0);
    gen_check_align(s, t0, 0x03);
    gen_qemu_ld32u(s, gpr, t0);
    tcg_gen_mov_tl(cpu_reserve, t0);
    tcg_gen_st_tl(gpr, cpu_env, offsetof(CPUState, reserve_val));
    tcg_temp_free(t0);
}

/* stwcx. */
static void gen_stwcx_(DisasContext *s)
{
    TCGv t0;
    int l1;

    gen_set_access_type(s, ACCESS_RES);
    t0 = tcg_temp_local_new();
    gen_addr_reg_index(s, t0);
    gen_check_align(s, t0, 0x03);

    tcg_gen_trunc_tl_i32(cpu_crf[0], cpu_xer);
    tcg_gen_shri_i32(cpu_crf[0], cpu_crf[0], XER_SO);
    tcg_gen_andi_i32(cpu_crf[0], cpu_crf[0], 1);
    l1 = gen_new_label();
    tcg_gen_brcond_tl(TCG_COND_NE, t0, cpu_reserve, l1);
    tcg_gen_ori_i32(cpu_crf[0], cpu_crf[0], 1 << CRF_EQ);
    gen_qemu_st32(s, cpu_gpr[rS(s->opcode)], t0);
    gen_set_label(l1);
    tcg_gen_movi_tl(cpu_reserve, -1);
    tcg_temp_free(t0);
}

#if defined(TARGET_PPC64)
/* ldarx */
static void gen_ldarx(DisasContext *s)
{
    TCGv t0;
    TCGv gpr = cpu_gpr[rD(s->opcode)];
    gen_set_access_type(s, ACCESS_RES);
    t0 = tcg_temp_local_new();
    gen_addr_reg_index(s, t0);
    gen_check_align(s, t0, 0x07);
    gen_qemu_ld64(s, gpr, t0);
    tcg_gen_mov_tl(cpu_reserve, t0);
    tcg_gen_st_tl(gpr, cpu_env, offsetof(CPUState, reserve_val));
    tcg_temp_free(t0);
}

/* stdcx. */
static void gen_stdcx_(DisasContext *s)
{
    TCGv t0;
    int l1;

    gen_set_access_type(s, ACCESS_RES);
    t0 = tcg_temp_local_new();
    gen_addr_reg_index(s, t0);
    gen_check_align(s, t0, 0x07);
    tcg_gen_trunc_tl_i32(cpu_crf[0], cpu_xer);
    tcg_gen_shri_i32(cpu_crf[0], cpu_crf[0], XER_SO);
    tcg_gen_andi_i32(cpu_crf[0], cpu_crf[0], 1);
    l1 = gen_new_label();
    tcg_gen_brcond_tl(TCG_COND_NE, t0, cpu_reserve, l1);
    tcg_gen_ori_i32(cpu_crf[0], cpu_crf[0], 1 << CRF_EQ);
    gen_qemu_st32(s, cpu_gpr[rS(s->opcode)], t0);
    gen_set_label(l1);
    tcg_gen_movi_tl(cpu_reserve, -1);
    tcg_temp_free(t0);
}
#endif /* defined(TARGET_PPC64) */
/* sync */
static void gen_sync(DisasContext *s) { }

/* wait */
static void gen_wait(DisasContext *s)
{
    TCGv_i32 t0 = tcg_temp_new_i32();
    tcg_gen_st_i32(t0, cpu_env, offsetof(CPUState, wfi));
    tcg_temp_free_i32(t0);
    /* Stop translation, as the CPU is supposed to sleep from now */
    gen_exception_err(s, EXCP_WFI, 1);
}

/***                         Floating-point load                ***/
#define GEN_LDF(name, ldop, opc, type)                  \
    static void glue(gen_, name)(DisasContext * s)      \
    {                                                   \
        TCGv EA;                                        \
        if(unlikely(!s->fpu_enabled)) {                 \
            gen_exception(s, POWERPC_EXCP_FPU);         \
            return;                                     \
        }                                               \
        gen_set_access_type(s, ACCESS_FLOAT);           \
        EA = tcg_temp_new();                            \
        gen_addr_imm_index(s, EA, 0);                   \
        gen_qemu_##ldop(s, cpu_fpr[rD(s->opcode)], EA); \
        tcg_temp_free(EA);                              \
    }

#define GEN_LDUF(name, ldop, opc, type)                       \
    static void glue(gen_, name##u)(DisasContext * s)         \
    {                                                         \
        TCGv EA;                                              \
        if(unlikely(!s->fpu_enabled)) {                       \
            gen_exception(s, POWERPC_EXCP_FPU);               \
            return;                                           \
        }                                                     \
        if(unlikely(rA(s->opcode) == 0)) {                    \
            gen_inval_exception(s, POWERPC_EXCP_INVAL_INVAL); \
            return;                                           \
        }                                                     \
        gen_set_access_type(s, ACCESS_FLOAT);                 \
        EA = tcg_temp_new();                                  \
        gen_addr_imm_index(s, EA, 0);                         \
        gen_qemu_##ldop(s, cpu_fpr[rD(s->opcode)], EA);       \
        tcg_gen_mov_tl(cpu_gpr[rA(s->opcode)], EA);           \
        tcg_temp_free(EA);                                    \
    }

#define GEN_LDUXF(name, ldop, opc, type)                      \
    static void glue(gen_, name##ux)(DisasContext * s)        \
    {                                                         \
        TCGv EA;                                              \
        if(unlikely(!s->fpu_enabled)) {                       \
            gen_exception(s, POWERPC_EXCP_FPU);               \
            return;                                           \
        }                                                     \
        if(unlikely(rA(s->opcode) == 0)) {                    \
            gen_inval_exception(s, POWERPC_EXCP_INVAL_INVAL); \
            return;                                           \
        }                                                     \
        gen_set_access_type(s, ACCESS_FLOAT);                 \
        EA = tcg_temp_new();                                  \
        gen_addr_reg_index(s, EA);                            \
        gen_qemu_##ldop(s, cpu_fpr[rD(s->opcode)], EA);       \
        tcg_gen_mov_tl(cpu_gpr[rA(s->opcode)], EA);           \
        tcg_temp_free(EA);                                    \
    }

#define GEN_LDXF(name, ldop, opc2, opc3, type)          \
    static void glue(gen_, name##x)(DisasContext * s)   \
    {                                                   \
        TCGv EA;                                        \
        if(unlikely(!s->fpu_enabled)) {                 \
            gen_exception(s, POWERPC_EXCP_FPU);         \
            return;                                     \
        }                                               \
        gen_set_access_type(s, ACCESS_FLOAT);           \
        EA = tcg_temp_new();                            \
        gen_addr_reg_index(s, EA);                      \
        gen_qemu_##ldop(s, cpu_fpr[rD(s->opcode)], EA); \
        tcg_temp_free(EA);                              \
    }

#define GEN_LDFS(name, ldop, op, type)      \
    GEN_LDF(name, ldop, op | 0x20, type);   \
    GEN_LDUF(name, ldop, op | 0x21, type);  \
    GEN_LDUXF(name, ldop, op | 0x01, type); \
    GEN_LDXF(name, ldop, 0x17, op | 0x00, type)

static inline void gen_qemu_ld32fs(DisasContext *s, TCGv_i64 arg1, TCGv arg2)
{
    TCGv t0 = tcg_temp_new();
    TCGv_i32 t1 = tcg_temp_new_i32();
    gen_qemu_ld32u(s, t0, arg2);
    tcg_gen_trunc_tl_i32(t1, t0);
    tcg_temp_free(t0);
    gen_helper_float32_to_float64(arg1, t1);
    tcg_temp_free_i32(t1);
}

/* lfd lfdu lfdux lfdx */
GEN_LDFS(lfd, ld64, 0x12, PPC_FLOAT);
/* lfs lfsu lfsux lfsx */
GEN_LDFS(lfs, ld32fs, 0x10, PPC_FLOAT);

/***                         Floating-point store               ***/
#define GEN_STF(name, stop, opc, type)                  \
    static void glue(gen_, name)(DisasContext * s)      \
    {                                                   \
        TCGv EA;                                        \
        if(unlikely(!s->fpu_enabled)) {                 \
            gen_exception(s, POWERPC_EXCP_FPU);         \
            return;                                     \
        }                                               \
        gen_set_access_type(s, ACCESS_FLOAT);           \
        EA = tcg_temp_local_new();                      \
        gen_addr_imm_index(s, EA, 0);                   \
        gen_qemu_##stop(s, cpu_fpr[rS(s->opcode)], EA); \
        tcg_temp_free(EA);                              \
    }

#define GEN_STUF(name, stop, opc, type)                       \
    static void glue(gen_, name##u)(DisasContext * s)         \
    {                                                         \
        TCGv EA;                                              \
        if(unlikely(!s->fpu_enabled)) {                       \
            gen_exception(s, POWERPC_EXCP_FPU);               \
            return;                                           \
        }                                                     \
        if(unlikely(rA(s->opcode) == 0)) {                    \
            gen_inval_exception(s, POWERPC_EXCP_INVAL_INVAL); \
            return;                                           \
        }                                                     \
        gen_set_access_type(s, ACCESS_FLOAT);                 \
        EA = tcg_temp_local_new();                            \
        gen_addr_imm_index(s, EA, 0);                         \
        gen_qemu_##stop(s, cpu_fpr[rS(s->opcode)], EA);       \
        tcg_gen_mov_tl(cpu_gpr[rA(s->opcode)], EA);           \
        tcg_temp_free(EA);                                    \
    }

#define GEN_STUXF(name, stop, opc, type)                      \
    static void glue(gen_, name##ux)(DisasContext * s)        \
    {                                                         \
        TCGv EA;                                              \
        if(unlikely(!s->fpu_enabled)) {                       \
            gen_exception(s, POWERPC_EXCP_FPU);               \
            return;                                           \
        }                                                     \
        if(unlikely(rA(s->opcode) == 0)) {                    \
            gen_inval_exception(s, POWERPC_EXCP_INVAL_INVAL); \
            return;                                           \
        }                                                     \
        gen_set_access_type(s, ACCESS_FLOAT);                 \
        EA = tcg_temp_local_new();                            \
        gen_addr_reg_index(s, EA);                            \
        gen_qemu_##stop(s, cpu_fpr[rS(s->opcode)], EA);       \
        tcg_gen_mov_tl(cpu_gpr[rA(s->opcode)], EA);           \
        tcg_temp_free(EA);                                    \
    }

#define GEN_STXF(name, stop, opc2, opc3, type)          \
    static void glue(gen_, name##x)(DisasContext * s)   \
    {                                                   \
        TCGv EA;                                        \
        if(unlikely(!s->fpu_enabled)) {                 \
            gen_exception(s, POWERPC_EXCP_FPU);         \
            return;                                     \
        }                                               \
        gen_set_access_type(s, ACCESS_FLOAT);           \
        EA = tcg_temp_local_new();                      \
        gen_addr_reg_index(s, EA);                      \
        gen_qemu_##stop(s, cpu_fpr[rS(s->opcode)], EA); \
        tcg_temp_free(EA);                              \
    }

#define GEN_STFS(name, stop, op, type)      \
    GEN_STF(name, stop, op | 0x20, type);   \
    GEN_STUF(name, stop, op | 0x21, type);  \
    GEN_STUXF(name, stop, op | 0x01, type); \
    GEN_STXF(name, stop, 0x17, op | 0x00, type)

static inline void gen_qemu_st32fs(DisasContext *s, TCGv_i64 arg1, TCGv arg2)
{
    TCGv_i32 t0 = tcg_temp_new_i32();
    TCGv t1 = tcg_temp_local_new();
    gen_helper_float64_to_float32(t0, arg1);
    tcg_gen_extu_i32_tl(t1, t0);
    tcg_temp_free_i32(t0);
    gen_qemu_st32(s, t1, arg2);
    tcg_temp_free(t1);
}

/* stfd stfdu stfdux stfdx */
GEN_STFS(stfd, st64, 0x16, PPC_FLOAT);
/* stfs stfsu stfsux stfsx */
GEN_STFS(stfs, st32fs, 0x14, PPC_FLOAT);

/* Optional: */
static inline void gen_qemu_st32fiw(DisasContext *s, TCGv_i64 arg1, TCGv arg2)
{
    TCGv t0 = tcg_temp_local_new();
    tcg_gen_trunc_i64_tl(t0, arg1);
    gen_qemu_st32(s, t0, arg2);
    tcg_temp_free(t0);
}
/* stfiwx */
GEN_STXF(stfiw, st32fiw, 0x17, 0x1E, PPC_FLOAT_STFIWX);

#if defined(TARGET_PPC64)
static inline void gen_update_cfar(DisasContext *s, target_ulong nip)
{
    if(s->has_cfar) {
        tcg_gen_movi_tl(cpu_cfar, nip);
    }
}
#endif

/***                                Branch                                 ***/
static inline void gen_goto_tb(DisasContext *s, int n, target_ulong dest)
{
    TranslationBlock *tb;
    tb = s->base.tb;
#if defined(TARGET_PPC64)
    if(!s->sf_mode) {
        dest = (uint32_t)dest;
    }
#endif
    if((tb->pc & TARGET_PAGE_MASK) == (dest & TARGET_PAGE_MASK)) {
        tcg_gen_goto_tb(n);
        tcg_gen_movi_tl(cpu_nip, dest & ~3);
        gen_exit_tb(tb, n);
    } else {
        tcg_gen_movi_tl(cpu_nip, dest & ~3);
        gen_exit_tb_no_chaining(tb);
    }
}

static inline void gen_setlr(DisasContext *s, target_ulong nip)
{
#if defined(TARGET_PPC64)
    if(s->sf_mode == 0) {
        tcg_gen_movi_tl(cpu_lr, (uint32_t)nip);
    } else
#endif
        tcg_gen_movi_tl(cpu_lr, nip);
}

/* b ba bl bla */
static void gen_b(DisasContext *s)
{
    target_ulong li, target;

    s->exception = POWERPC_EXCP_BRANCH;
    /* sign extend LI */
#if defined(TARGET_PPC64)
    if(s->sf_mode) {
        li = ((int64_t)LI(s->opcode) << 38) >> 38;
    } else
#endif
        li = ((int32_t)LI(s->opcode) << 6) >> 6;
    if(likely(AA(s->opcode) == 0)) {
        target = s->base.pc + li - 4;
    } else {
        target = li;
    }
    if(LK(s->opcode)) {
        gen_setlr(s, s->base.pc);
    }
    gen_goto_tb(s, 0, target);
}

#define BCOND_IM  0
#define BCOND_LR  1
#define BCOND_CTR 2

static inline void gen_bcond(DisasContext *s, int type)
{
    uint32_t bo = BO(s->opcode);
    int l1;
    TCGv target;

    s->exception = POWERPC_EXCP_BRANCH;
    if(type == BCOND_LR || type == BCOND_CTR) {
        target = tcg_temp_local_new();
        if(type == BCOND_CTR) {
            tcg_gen_mov_tl(target, cpu_ctr);
        } else {
            tcg_gen_mov_tl(target, cpu_lr);
        }
    } else {
        TCGV_UNUSED(target);
    }
    if(LK(s->opcode)) {
        gen_setlr(s, s->base.pc);
    }
    l1 = gen_new_label();
    if((bo & 0x4) == 0) {
        /* Decrement and test CTR */
        TCGv temp = tcg_temp_local_new();
        if(unlikely(type == BCOND_CTR)) {
            gen_inval_exception(s, POWERPC_EXCP_INVAL_INVAL);
            return;
        }
        tcg_gen_subi_tl(cpu_ctr, cpu_ctr, 1);
#if defined(TARGET_PPC64)
        if(!s->sf_mode) {
            tcg_gen_ext32u_tl(temp, cpu_ctr);
        } else
#endif
            tcg_gen_mov_tl(temp, cpu_ctr);
        if(bo & 0x2) {
            tcg_gen_brcondi_tl(TCG_COND_NE, temp, 0, l1);
        } else {
            tcg_gen_brcondi_tl(TCG_COND_EQ, temp, 0, l1);
        }
        tcg_temp_free(temp);
    }
    if((bo & 0x10) == 0) {
        /* Test CR */
        uint32_t bi = BI(s->opcode);
        uint32_t mask = 1 << (3 - (bi & 0x03));
        TCGv_i32 temp = tcg_temp_new_i32();

        if(bo & 0x8) {
            tcg_gen_andi_i32(temp, cpu_crf[bi >> 2], mask);
            tcg_gen_brcondi_i32(TCG_COND_EQ, temp, 0, l1);
        } else {
            tcg_gen_andi_i32(temp, cpu_crf[bi >> 2], mask);
            tcg_gen_brcondi_i32(TCG_COND_NE, temp, 0, l1);
        }
        tcg_temp_free_i32(temp);
    }
    if(type == BCOND_IM) {
        target_ulong li = (target_long)((int16_t)(BD(s->opcode)));
        if(likely(AA(s->opcode) == 0)) {
            gen_goto_tb(s, 0, s->base.pc + li - 4);
        } else {
            gen_goto_tb(s, 0, li);
        }
        gen_set_label(l1);
        gen_goto_tb(s, 1, s->base.pc);
    } else {
#if defined(TARGET_PPC64)
        if(!(s->sf_mode)) {
            tcg_gen_andi_tl(cpu_nip, target, (uint32_t)~3);
        } else
#endif
            tcg_gen_andi_tl(cpu_nip, target, ~3);
        gen_exit_tb_no_chaining(s->base.tb);
        gen_set_label(l1);
#if defined(TARGET_PPC64)
        if(!(s->sf_mode)) {
            tcg_gen_movi_tl(cpu_nip, (uint32_t)s->base.pc);
        } else
#endif
            tcg_gen_movi_tl(cpu_nip, s->base.pc);
        gen_exit_tb_no_chaining(s->base.tb);
    }
}

static void gen_bc(DisasContext *s)
{
    gen_bcond(s, BCOND_IM);
}

static void gen_bcctr(DisasContext *s)
{
    gen_bcond(s, BCOND_CTR);
}

static void gen_bclr(DisasContext *s)
{
    gen_bcond(s, BCOND_LR);
}

/***                      Condition register logical                       ***/
#define GEN_CRLOGIC(name, tcg_op, opc)                                 \
    static void glue(gen_, name)(DisasContext * s)                     \
    {                                                                  \
        uint8_t bitmask;                                               \
        int sh;                                                        \
        TCGv_i32 t0, t1;                                               \
        sh = (crbD(s->opcode) & 0x03) - (crbA(s->opcode) & 0x03);      \
        t0 = tcg_temp_new_i32();                                       \
        if(sh > 0)                                                     \
            tcg_gen_shri_i32(t0, cpu_crf[crbA(s->opcode) >> 2], sh);   \
        else if(sh < 0)                                                \
            tcg_gen_shli_i32(t0, cpu_crf[crbA(s->opcode) >> 2], -sh);  \
        else                                                           \
            tcg_gen_mov_i32(t0, cpu_crf[crbA(s->opcode) >> 2]);        \
        t1 = tcg_temp_new_i32();                                       \
        sh = (crbD(s->opcode) & 0x03) - (crbB(s->opcode) & 0x03);      \
        if(sh > 0)                                                     \
            tcg_gen_shri_i32(t1, cpu_crf[crbB(s->opcode) >> 2], sh);   \
        else if(sh < 0)                                                \
            tcg_gen_shli_i32(t1, cpu_crf[crbB(s->opcode) >> 2], -sh);  \
        else                                                           \
            tcg_gen_mov_i32(t1, cpu_crf[crbB(s->opcode) >> 2]);        \
        tcg_op(t0, t0, t1);                                            \
        bitmask = 1 << (3 - (crbD(s->opcode) & 0x03));                 \
        tcg_gen_andi_i32(t0, t0, bitmask);                             \
        tcg_gen_andi_i32(t1, cpu_crf[crbD(s->opcode) >> 2], ~bitmask); \
        tcg_gen_or_i32(cpu_crf[crbD(s->opcode) >> 2], t0, t1);         \
        tcg_temp_free_i32(t0);                                         \
        tcg_temp_free_i32(t1);                                         \
    }

/* crand */
GEN_CRLOGIC(crand, tcg_gen_and_i32, 0x08);
/* crandc */
GEN_CRLOGIC(crandc, tcg_gen_andc_i32, 0x04);
/* creqv */
GEN_CRLOGIC(creqv, tcg_gen_eqv_i32, 0x09);
/* crnand */
GEN_CRLOGIC(crnand, tcg_gen_nand_i32, 0x07);
/* crnor */
GEN_CRLOGIC(crnor, tcg_gen_nor_i32, 0x01);
/* cror */
GEN_CRLOGIC(cror, tcg_gen_or_i32, 0x0E);
/* crorc */
GEN_CRLOGIC(crorc, tcg_gen_orc_i32, 0x0D);
/* crxor */
GEN_CRLOGIC(crxor, tcg_gen_xor_i32, 0x06);

/* mcrf */
static void gen_mcrf(DisasContext *s)
{
    tcg_gen_mov_i32(cpu_crf[crfD(s->opcode)], cpu_crf[crfS(s->opcode)]);
}

/***                           System linkage                              ***/

/* rfi (mem_idx only) */
static void gen_rfi(DisasContext *s)
{
    /* Restore CPU state */
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    gen_helper_rfi();
    gen_sync_exception(s);
}

#if defined(TARGET_PPC64)
static void gen_rfid(DisasContext *s)
{
    /* Restore CPU state */
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    gen_update_cfar(s, s->base.pc);
    gen_helper_rfid();
    gen_sync_exception(s);
}
static void gen_hrfid(DisasContext *s)
{
    /* Restore CPU state */
    if(unlikely(s->base.mem_idx <= 1)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    gen_helper_hrfid();
    gen_sync_exception(s);
}
#endif

/* sc */
#define POWERPC_SYSCALL POWERPC_EXCP_SYSCALL
static void gen_sc(DisasContext *s)
{
    uint32_t lev;

    lev = (s->opcode >> 5) & 0x7F;
    gen_exception_err(s, POWERPC_SYSCALL, lev);
}

/***                                Trap                                   ***/

/* tw */
static void gen_tw(DisasContext *s)
{
    TCGv_i32 t0 = tcg_const_i32(TO(s->opcode));
    /* Update the nip since this might generate a trap exception */
    gen_update_nip(s, s->base.pc);
    gen_helper_tw(cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)], t0);
    tcg_temp_free_i32(t0);
}

/* twi */
static void gen_twi(DisasContext *s)
{
    TCGv t0 = tcg_const_tl(SIMM(s->opcode));
    TCGv_i32 t1 = tcg_const_i32(TO(s->opcode));
    /* Update the nip since this might generate a trap exception */
    gen_update_nip(s, s->base.pc);
    gen_helper_tw(cpu_gpr[rA(s->opcode)], t0, t1);
    tcg_temp_free(t0);
    tcg_temp_free_i32(t1);
}

#if defined(TARGET_PPC64)
/* td */
static void gen_td(DisasContext *s)
{
    TCGv_i32 t0 = tcg_const_i32(TO(s->opcode));
    /* Update the nip since this might generate a trap exception */
    gen_update_nip(s, s->base.pc);
    gen_helper_td(cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)], t0);
    tcg_temp_free_i32(t0);
}
/* tdi */
static void gen_tdi(DisasContext *s)
{
    TCGv t0 = tcg_const_tl(SIMM(s->opcode));
    TCGv_i32 t1 = tcg_const_i32(TO(s->opcode));
    /* Update the nip since this might generate a trap exception */
    gen_update_nip(s, s->base.pc);
    gen_helper_td(cpu_gpr[rA(s->opcode)], t0, t1);
    tcg_temp_free(t0);
    tcg_temp_free_i32(t1);
}
#endif

/***                          Processor control                            ***/

/* mcrxr */
static void gen_mcrxr(DisasContext *s)
{
    tcg_gen_trunc_tl_i32(cpu_crf[crfD(s->opcode)], cpu_xer);
    tcg_gen_shri_i32(cpu_crf[crfD(s->opcode)], cpu_crf[crfD(s->opcode)], XER_CA);
    tcg_gen_andi_tl(cpu_xer, cpu_xer, ~(1 << XER_SO | 1 << XER_OV | 1 << XER_CA));
}

/* mfcr mfocrf */
static void gen_mfcr(DisasContext *s)
{
    uint32_t crm, crn;

    if(likely(s->opcode & 0x00100000)) {
        crm = CRM(s->opcode);
        if(likely(crm && ((crm & (crm - 1)) == 0))) {
            crn = ctz32(crm);
            tcg_gen_extu_i32_tl(cpu_gpr[rD(s->opcode)], cpu_crf[7 - crn]);
            tcg_gen_shli_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rD(s->opcode)], crn * 4);
        }
    } else {
        TCGv_i32 t0 = tcg_temp_new_i32();
        tcg_gen_mov_i32(t0, cpu_crf[0]);
        tcg_gen_shli_i32(t0, t0, 4);
        tcg_gen_or_i32(t0, t0, cpu_crf[1]);
        tcg_gen_shli_i32(t0, t0, 4);
        tcg_gen_or_i32(t0, t0, cpu_crf[2]);
        tcg_gen_shli_i32(t0, t0, 4);
        tcg_gen_or_i32(t0, t0, cpu_crf[3]);
        tcg_gen_shli_i32(t0, t0, 4);
        tcg_gen_or_i32(t0, t0, cpu_crf[4]);
        tcg_gen_shli_i32(t0, t0, 4);
        tcg_gen_or_i32(t0, t0, cpu_crf[5]);
        tcg_gen_shli_i32(t0, t0, 4);
        tcg_gen_or_i32(t0, t0, cpu_crf[6]);
        tcg_gen_shli_i32(t0, t0, 4);
        tcg_gen_or_i32(t0, t0, cpu_crf[7]);
        tcg_gen_extu_i32_tl(cpu_gpr[rD(s->opcode)], t0);
        tcg_temp_free_i32(t0);
    }
}

/* mfmsr */
static void gen_mfmsr(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_REG);
        return;
    }
    tcg_gen_mov_tl(cpu_gpr[rD(s->opcode)], cpu_msr);
}

static void spr_noaccess(void *opaque, int gprn, int sprn) { }
#define SPR_NOACCESS (&spr_noaccess)

/* mfspr */
static inline void gen_op_mfspr(DisasContext *s)
{
    void (*read_cb)(void *opaque, int gprn, int sprn);
    uint32_t sprn = SPR(s->opcode);

    if(s->base.mem_idx == 2) {
        read_cb = s->spr_cb[sprn].hea_read;
    } else if(s->base.mem_idx) {
        read_cb = s->spr_cb[sprn].oea_read;
    } else {
        read_cb = s->spr_cb[sprn].uea_read;
    }
    if(likely(read_cb != NULL)) {
        if(likely(read_cb != SPR_NOACCESS)) {
            (*read_cb)(s, rD(s->opcode), sprn);
        } else {
            /* Privilege exception */
            /* This is a hack to avoid warnings when running Linux:
             * this OS breaks the PowerPC virtualisation model,
             * allowing userland application to read the PVR
             */
            gen_inval_exception(s, POWERPC_EXCP_PRIV_REG);
        }
    } else {
        /* Not defined */
        gen_inval_exception(s, POWERPC_EXCP_INVAL_SPR);
    }
}

static void gen_mfspr(DisasContext *s)
{
    gen_op_mfspr(s);
}

/* mftb */
static void gen_mftb(DisasContext *s)
{
    gen_op_mfspr(s);
}

/* mtcrf mtocrf*/
static void gen_mtcrf(DisasContext *s)
{
    uint32_t crm, crn;

    crm = CRM(s->opcode);
    if(likely((s->opcode & 0x00100000))) {
        if(crm && ((crm & (crm - 1)) == 0)) {
            TCGv_i32 temp = tcg_temp_new_i32();
            crn = ctz32(crm);
            tcg_gen_trunc_tl_i32(temp, cpu_gpr[rS(s->opcode)]);
            tcg_gen_shri_i32(temp, temp, crn * 4);
            tcg_gen_andi_i32(cpu_crf[7 - crn], temp, 0xf);
            tcg_temp_free_i32(temp);
        }
    } else {
        TCGv_i32 temp = tcg_temp_new_i32();
        tcg_gen_trunc_tl_i32(temp, cpu_gpr[rS(s->opcode)]);
        for(crn = 0; crn < 8; crn++) {
            if(crm & (1 << crn)) {
                tcg_gen_shri_i32(cpu_crf[7 - crn], temp, crn * 4);
                tcg_gen_andi_i32(cpu_crf[7 - crn], cpu_crf[7 - crn], 0xf);
            }
        }
        tcg_temp_free_i32(temp);
    }
}

/* mtmsr */
#if defined(TARGET_PPC64)
static void gen_mtmsrd(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_REG);
        return;
    }
    if(s->opcode & 0x00010000) {
        /* Special form that does not need any synchronisation */
        TCGv t0 = tcg_temp_new();
        tcg_gen_andi_tl(t0, cpu_gpr[rS(s->opcode)], (1 << MSR_RI) | (1 << MSR_EE));
        tcg_gen_andi_tl(cpu_msr, cpu_msr, ~((1 << MSR_RI) | (1 << MSR_EE)));
        tcg_gen_or_tl(cpu_msr, cpu_msr, t0);
        tcg_temp_free(t0);
    } else {
        /* XXX: we need to update nip before the store
         *      if we enter power saving mode, we will exit the loop
         *      directly from ppc_store_msr
         */
        gen_update_nip(s, s->base.pc);
        gen_helper_store_msr(cpu_gpr[rS(s->opcode)]);
        /* Must stop the translation as machine state (may have) changed */
        /* Note that mtmsr is not always defined as context-synchronizing */
        gen_stop_exception(s);
    }
}
#endif

static void gen_mtmsr(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_REG);
        return;
    }
    if(s->opcode & 0x00010000) {
        /* Special form that does not need any synchronisation */
        TCGv t0 = tcg_temp_new();
        tcg_gen_andi_tl(t0, cpu_gpr[rS(s->opcode)], (1 << MSR_RI) | (1 << MSR_EE));
        tcg_gen_andi_tl(cpu_msr, cpu_msr, ~((1 << MSR_RI) | (1 << MSR_EE)));
        tcg_gen_or_tl(cpu_msr, cpu_msr, t0);
        tcg_temp_free(t0);
    } else {
        TCGv msr = tcg_temp_new();

        /* XXX: we need to update nip before the store
         *      if we enter power saving mode, we will exit the loop
         *      directly from ppc_store_msr
         */
        gen_update_nip(s, s->base.pc);
#if defined(TARGET_PPC64)
        tcg_gen_deposit_tl(msr, cpu_msr, cpu_gpr[rS(s->opcode)], 0, 32);
#else
        tcg_gen_mov_tl(msr, cpu_gpr[rS(s->opcode)]);
#endif
        gen_helper_store_msr(msr);
        /* Must stop the translation as machine state (may have) changed */
        /* Note that mtmsr is not always defined as context-synchronizing */
        gen_stop_exception(s);
    }
}

/* mtspr */
static void gen_mtspr(DisasContext *s)
{
    void (*write_cb)(void *opaque, int sprn, int gprn);
    uint32_t sprn = SPR(s->opcode);

    if(s->base.mem_idx == 2) {
        write_cb = s->spr_cb[sprn].hea_write;
    } else if(s->base.mem_idx) {
        write_cb = s->spr_cb[sprn].oea_write;
    } else {
        write_cb = s->spr_cb[sprn].uea_write;
    }
    if(likely(write_cb != NULL)) {
        if(likely(write_cb != SPR_NOACCESS)) {
            (*write_cb)(s, sprn, rS(s->opcode));
        } else {
            /* Privilege exception */
            gen_inval_exception(s, POWERPC_EXCP_PRIV_REG);
        }
    } else {
        /* Not defined */
        gen_inval_exception(s, POWERPC_EXCP_INVAL_SPR);
    }
}

/***                         Cache management                              ***/

/* dcbf */
static void gen_dcbf(DisasContext *s)
{
    /* XXX: specification says this is treated as a load by the MMU */
    TCGv t0;
    gen_set_access_type(s, ACCESS_CACHE);
    t0 = tcg_temp_new();
    gen_addr_reg_index(s, t0);
    gen_qemu_ld8u(s, t0, t0);
    tcg_temp_free(t0);
}

/* dcbi (Supervisor only) */
static void gen_dcbi(DisasContext *s)
{
    TCGv EA, val;
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    EA = tcg_temp_local_new();
    gen_set_access_type(s, ACCESS_CACHE);
    gen_addr_reg_index(s, EA);
    val = tcg_temp_local_new();
    /* XXX: specification says this should be treated as a store by the MMU */
    gen_qemu_ld8u(s, val, EA);
    gen_qemu_st8(s, val, EA);
    tcg_temp_free(val);
    tcg_temp_free(EA);
}

/* dcdst */
static void gen_dcbst(DisasContext *s)
{
    /* XXX: specification say this is treated as a load by the MMU */
    TCGv t0;
    gen_set_access_type(s, ACCESS_CACHE);
    t0 = tcg_temp_new();
    gen_addr_reg_index(s, t0);
    gen_qemu_ld8u(s, t0, t0);
    tcg_temp_free(t0);
}

/* dcbt */
static void gen_dcbt(DisasContext *s)
{
    /* interpreted as no-op */
    /* XXX: specification say this is treated as a load by the MMU
     *      but does not generate any exception
     */
}

/* dcbtst */
static void gen_dcbtst(DisasContext *s)
{
    /* interpreted as no-op */
    /* XXX: specification say this is treated as a load by the MMU
     *      but does not generate any exception
     */
}

/* dcbz */
static void gen_dcbz(DisasContext *s)
{
    TCGv t0;
    gen_set_access_type(s, ACCESS_CACHE);
    /* NIP cannot be restored if the memory exception comes from an helper */
    gen_update_nip(s, s->base.pc - 4);
    t0 = tcg_temp_new();
    gen_addr_reg_index(s, t0);
    gen_helper_dcbz(t0);
    tcg_temp_free(t0);
}

static void gen_dcbz_970(DisasContext *s)
{
    TCGv t0;
    gen_set_access_type(s, ACCESS_CACHE);
    /* NIP cannot be restored if the memory exception comes from an helper */
    gen_update_nip(s, s->base.pc - 4);
    t0 = tcg_temp_new();
    gen_addr_reg_index(s, t0);
    if(s->opcode & 0x00200000) {
        gen_helper_dcbz(t0);
    } else {
        gen_helper_dcbz_970(t0);
    }
    tcg_temp_free(t0);
}

/* dst / dstt */
static void gen_dst(DisasContext *s)
{
    if(rA(s->opcode) == 0) {
        gen_inval_exception(s, POWERPC_EXCP_INVAL_LSWX);
    } else {
        /* interpreted as no-op */
    }
}

/* dstst /dststt */
static void gen_dstst(DisasContext *s)
{
    if(rA(s->opcode) == 0) {
        gen_inval_exception(s, POWERPC_EXCP_INVAL_LSWX);
    } else {
        /* interpreted as no-op */
    }
}

/* dss / dssall */
static void gen_dss(DisasContext *s)
{
    /* interpreted as no-op */
}

/* icbi */
static void gen_icbi(DisasContext *s)
{
    TCGv t0;
    gen_set_access_type(s, ACCESS_CACHE);
    /* NIP cannot be restored if the memory exception comes from an helper */
    gen_update_nip(s, s->base.pc - 4);
    t0 = tcg_temp_new();
    gen_addr_reg_index(s, t0);
    gen_helper_icbi(t0);
    tcg_temp_free(t0);
}

/* Optional: */
/* dcba */
static void gen_dcba(DisasContext *s)
{
    /* interpreted as no-op */
    /* XXX: specification say this is treated as a store by the MMU
     *      but does not generate any exception
     */
}

/***                    Segment register manipulation                      ***/
/* Supervisor only: */

/* mfsr */
static void gen_mfsr(DisasContext *s)
{
    TCGv t0;
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_REG);
        return;
    }
    t0 = tcg_const_tl(SR(s->opcode));
    gen_helper_load_sr(cpu_gpr[rD(s->opcode)], t0);
    tcg_temp_free(t0);
}

/* mfsrin */
static void gen_mfsrin(DisasContext *s)
{
    TCGv t0;
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_REG);
        return;
    }
    t0 = tcg_temp_new();
    tcg_gen_shri_tl(t0, cpu_gpr[rB(s->opcode)], 28);
    tcg_gen_andi_tl(t0, t0, 0xF);
    gen_helper_load_sr(cpu_gpr[rD(s->opcode)], t0);
    tcg_temp_free(t0);
}

/* mtsr */
static void gen_mtsr(DisasContext *s)
{
    TCGv t0;
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_REG);
        return;
    }
    t0 = tcg_const_tl(SR(s->opcode));
    gen_helper_store_sr(t0, cpu_gpr[rS(s->opcode)]);
    tcg_temp_free(t0);
}

/* mtsrin */
static void gen_mtsrin(DisasContext *s)
{
    TCGv t0;
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_REG);
        return;
    }
    t0 = tcg_temp_new();
    tcg_gen_shri_tl(t0, cpu_gpr[rB(s->opcode)], 28);
    tcg_gen_andi_tl(t0, t0, 0xF);
    gen_helper_store_sr(t0, cpu_gpr[rD(s->opcode)]);
    tcg_temp_free(t0);
}

#if defined(TARGET_PPC64)
/* Specific implementation for PowerPC 64 "bridge" emulation using SLB */

/* mfsr */
static void gen_mfsr_64b(DisasContext *s)
{
    TCGv t0;
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_REG);
        return;
    }
    t0 = tcg_const_tl(SR(s->opcode));
    gen_helper_load_sr(cpu_gpr[rD(s->opcode)], t0);
    tcg_temp_free(t0);
}

/* mfsrin */
static void gen_mfsrin_64b(DisasContext *s)
{
    TCGv t0;
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_REG);
        return;
    }
    t0 = tcg_temp_new();
    tcg_gen_shri_tl(t0, cpu_gpr[rB(s->opcode)], 28);
    tcg_gen_andi_tl(t0, t0, 0xF);
    gen_helper_load_sr(cpu_gpr[rD(s->opcode)], t0);
    tcg_temp_free(t0);
}

/* mtsr */
static void gen_mtsr_64b(DisasContext *s)
{
    TCGv t0;
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_REG);
        return;
    }
    t0 = tcg_const_tl(SR(s->opcode));
    gen_helper_store_sr(t0, cpu_gpr[rS(s->opcode)]);
    tcg_temp_free(t0);
}

/* mtsrin */
static void gen_mtsrin_64b(DisasContext *s)
{
    TCGv t0;
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_REG);
        return;
    }
    t0 = tcg_temp_new();
    tcg_gen_shri_tl(t0, cpu_gpr[rB(s->opcode)], 28);
    tcg_gen_andi_tl(t0, t0, 0xF);
    gen_helper_store_sr(t0, cpu_gpr[rS(s->opcode)]);
    tcg_temp_free(t0);
}

/* slbmte */
static void gen_slbmte(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_REG);
        return;
    }
    gen_helper_store_slb(cpu_gpr[rB(s->opcode)], cpu_gpr[rS(s->opcode)]);
}

static void gen_slbmfee(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_REG);
        return;
    }
    gen_helper_load_slb_esid(cpu_gpr[rS(s->opcode)], cpu_gpr[rB(s->opcode)]);
}

static void gen_slbmfev(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_REG);
        return;
    }
    gen_helper_load_slb_vsid(cpu_gpr[rS(s->opcode)], cpu_gpr[rB(s->opcode)]);
}
#endif /* defined(TARGET_PPC64) */

/***                      Lookaside buffer management                      ***/
/* Optional & mem_idx only: */

/* tlbia */
static void gen_tlbia(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    gen_helper_tlbia();
}

/* tlbiel */
static void gen_tlbiel(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    gen_helper_tlbie(cpu_gpr[rB(s->opcode)]);
}

/* tlbie */
static void gen_tlbie(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
#if defined(TARGET_PPC64)
    if(!s->sf_mode) {
        TCGv t0 = tcg_temp_new();
        tcg_gen_ext32u_tl(t0, cpu_gpr[rB(s->opcode)]);
        gen_helper_tlbie(t0);
        tcg_temp_free(t0);
    } else
#endif
        gen_helper_tlbie(cpu_gpr[rB(s->opcode)]);
}

/* tlbsync */
static void gen_tlbsync(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    /* This has no effect: it should ensure that all previous
     * tlbie have completed
     */
    gen_stop_exception(s);
}

#if defined(TARGET_PPC64)
/* slbia */
static void gen_slbia(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    gen_helper_slbia();
}

/* slbie */
static void gen_slbie(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    gen_helper_slbie(cpu_gpr[rB(s->opcode)]);
}
#endif

/***                              External control                         ***/
/* Optional: */

/* eciwx */
static void gen_eciwx(DisasContext *s)
{
    TCGv t0;
    /* Should check EAR[E] ! */
    gen_set_access_type(s, ACCESS_EXT);
    t0 = tcg_temp_new();
    gen_addr_reg_index(s, t0);
    gen_check_align(s, t0, 0x03);
    gen_qemu_ld32u(s, cpu_gpr[rD(s->opcode)], t0);
    tcg_temp_free(t0);
}

/* ecowx */
static void gen_ecowx(DisasContext *s)
{
    TCGv t0;
    /* Should check EAR[E] ! */
    gen_set_access_type(s, ACCESS_EXT);
    t0 = tcg_temp_local_new();
    gen_addr_reg_index(s, t0);
    gen_check_align(s, t0, 0x03);
    gen_qemu_st32(s, cpu_gpr[rD(s->opcode)], t0);
    tcg_temp_free(t0);
}

/* PowerPC 601 specific instructions */

/* abs - abs. */
static void gen_abs(DisasContext *s)
{
    int l1 = gen_new_label();
    int l2 = gen_new_label();
    tcg_gen_brcondi_tl(TCG_COND_GE, cpu_gpr[rA(s->opcode)], 0, l1);
    tcg_gen_neg_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)]);
    tcg_gen_br(l2);
    gen_set_label(l1);
    tcg_gen_mov_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)]);
    gen_set_label(l2);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rD(s->opcode)]);
    }
}

/* abso - abso. */
static void gen_abso(DisasContext *s)
{
    int l1 = gen_new_label();
    int l2 = gen_new_label();
    int l3 = gen_new_label();
    /* Start with XER OV disabled, the most likely case */
    tcg_gen_andi_tl(cpu_xer, cpu_xer, ~(1 << XER_OV));
    tcg_gen_brcondi_tl(TCG_COND_GE, cpu_gpr[rA(s->opcode)], 0, l2);
    tcg_gen_brcondi_tl(TCG_COND_NE, cpu_gpr[rA(s->opcode)], 0x80000000, l1);
    tcg_gen_ori_tl(cpu_xer, cpu_xer, (1 << XER_OV) | (1 << XER_SO));
    tcg_gen_br(l2);
    gen_set_label(l1);
    tcg_gen_neg_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)]);
    tcg_gen_br(l3);
    gen_set_label(l2);
    tcg_gen_mov_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)]);
    gen_set_label(l3);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rD(s->opcode)]);
    }
}

/* clcs */
static void gen_clcs(DisasContext *s)
{
    TCGv_i32 t0 = tcg_const_i32(rA(s->opcode));
    gen_helper_clcs(cpu_gpr[rD(s->opcode)], t0);
    tcg_temp_free_i32(t0);
    /* Rc=1 sets CR0 to an undefined state */
}

/* div - div. */
static void gen_div(DisasContext *s)
{
    gen_helper_div(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)]);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rD(s->opcode)]);
    }
}

/* divo - divo. */
static void gen_divo(DisasContext *s)
{
    gen_helper_divo(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)]);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rD(s->opcode)]);
    }
}

/* divs - divs. */
static void gen_divs(DisasContext *s)
{
    gen_helper_divs(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)]);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rD(s->opcode)]);
    }
}

/* divso - divso. */
static void gen_divso(DisasContext *s)
{
    gen_helper_divso(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)]);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rD(s->opcode)]);
    }
}

/* doz - doz. */
static void gen_doz(DisasContext *s)
{
    int l1 = gen_new_label();
    int l2 = gen_new_label();
    tcg_gen_brcond_tl(TCG_COND_GE, cpu_gpr[rB(s->opcode)], cpu_gpr[rA(s->opcode)], l1);
    tcg_gen_sub_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rB(s->opcode)], cpu_gpr[rA(s->opcode)]);
    tcg_gen_br(l2);
    gen_set_label(l1);
    tcg_gen_movi_tl(cpu_gpr[rD(s->opcode)], 0);
    gen_set_label(l2);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rD(s->opcode)]);
    }
}

/* dozo - dozo. */
static void gen_dozo(DisasContext *s)
{
    int l1 = gen_new_label();
    int l2 = gen_new_label();
    TCGv t0 = tcg_temp_new();
    TCGv t1 = tcg_temp_new();
    TCGv t2 = tcg_temp_new();
    /* Start with XER OV disabled, the most likely case */
    tcg_gen_andi_tl(cpu_xer, cpu_xer, ~(1 << XER_OV));
    tcg_gen_brcond_tl(TCG_COND_GE, cpu_gpr[rB(s->opcode)], cpu_gpr[rA(s->opcode)], l1);
    tcg_gen_sub_tl(t0, cpu_gpr[rB(s->opcode)], cpu_gpr[rA(s->opcode)]);
    tcg_gen_xor_tl(t1, cpu_gpr[rB(s->opcode)], cpu_gpr[rA(s->opcode)]);
    tcg_gen_xor_tl(t2, cpu_gpr[rA(s->opcode)], t0);
    tcg_gen_andc_tl(t1, t1, t2);
    tcg_gen_mov_tl(cpu_gpr[rD(s->opcode)], t0);
    tcg_gen_brcondi_tl(TCG_COND_GE, t1, 0, l2);
    tcg_gen_ori_tl(cpu_xer, cpu_xer, (1 << XER_OV) | (1 << XER_SO));
    tcg_gen_br(l2);
    gen_set_label(l1);
    tcg_gen_movi_tl(cpu_gpr[rD(s->opcode)], 0);
    gen_set_label(l2);
    tcg_temp_free(t0);
    tcg_temp_free(t1);
    tcg_temp_free(t2);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rD(s->opcode)]);
    }
}

/* dozi */
static void gen_dozi(DisasContext *s)
{
    target_long simm = SIMM(s->opcode);
    int l1 = gen_new_label();
    int l2 = gen_new_label();
    tcg_gen_brcondi_tl(TCG_COND_LT, cpu_gpr[rA(s->opcode)], simm, l1);
    tcg_gen_subfi_tl(cpu_gpr[rD(s->opcode)], simm, cpu_gpr[rA(s->opcode)]);
    tcg_gen_br(l2);
    gen_set_label(l1);
    tcg_gen_movi_tl(cpu_gpr[rD(s->opcode)], 0);
    gen_set_label(l2);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rD(s->opcode)]);
    }
}

/* lscbx - lscbx. */
static void gen_lscbx(DisasContext *s)
{
    TCGv t0 = tcg_temp_new();
    TCGv_i32 t1 = tcg_const_i32(rD(s->opcode));
    TCGv_i32 t2 = tcg_const_i32(rA(s->opcode));
    TCGv_i32 t3 = tcg_const_i32(rB(s->opcode));

    gen_addr_reg_index(s, t0);
    /* NIP cannot be restored if the memory exception comes from an helper */
    gen_update_nip(s, s->base.pc - 4);
    gen_helper_lscbx(t0, t0, t1, t2, t3);
    tcg_temp_free_i32(t1);
    tcg_temp_free_i32(t2);
    tcg_temp_free_i32(t3);
    tcg_gen_andi_tl(cpu_xer, cpu_xer, ~0x7F);
    tcg_gen_or_tl(cpu_xer, cpu_xer, t0);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, t0);
    }
    tcg_temp_free(t0);
}

/* maskg - maskg. */
static void gen_maskg(DisasContext *s)
{
    int l1 = gen_new_label();
    TCGv t0 = tcg_temp_new();
    TCGv t1 = tcg_temp_new();
    TCGv t2 = tcg_temp_new();
    TCGv t3 = tcg_temp_new();
    tcg_gen_movi_tl(t3, 0xFFFFFFFF);
    tcg_gen_andi_tl(t0, cpu_gpr[rB(s->opcode)], 0x1F);
    tcg_gen_andi_tl(t1, cpu_gpr[rS(s->opcode)], 0x1F);
    tcg_gen_addi_tl(t2, t0, 1);
    tcg_gen_shr_tl(t2, t3, t2);
    tcg_gen_shr_tl(t3, t3, t1);
    tcg_gen_xor_tl(cpu_gpr[rA(s->opcode)], t2, t3);
    tcg_gen_brcond_tl(TCG_COND_GE, t0, t1, l1);
    tcg_gen_neg_tl(cpu_gpr[rA(s->opcode)], cpu_gpr[rA(s->opcode)]);
    gen_set_label(l1);
    tcg_temp_free(t0);
    tcg_temp_free(t1);
    tcg_temp_free(t2);
    tcg_temp_free(t3);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* maskir - maskir. */
static void gen_maskir(DisasContext *s)
{
    TCGv t0 = tcg_temp_new();
    TCGv t1 = tcg_temp_new();
    tcg_gen_and_tl(t0, cpu_gpr[rS(s->opcode)], cpu_gpr[rB(s->opcode)]);
    tcg_gen_andc_tl(t1, cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)]);
    tcg_gen_or_tl(cpu_gpr[rA(s->opcode)], t0, t1);
    tcg_temp_free(t0);
    tcg_temp_free(t1);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* mul - mul. */
static void gen_mul(DisasContext *s)
{
    TCGv_i64 t0 = tcg_temp_new_i64();
    TCGv_i64 t1 = tcg_temp_new_i64();
    TCGv t2 = tcg_temp_new();
    tcg_gen_extu_tl_i64(t0, cpu_gpr[rA(s->opcode)]);
    tcg_gen_extu_tl_i64(t1, cpu_gpr[rB(s->opcode)]);
    tcg_gen_mul_i64(t0, t0, t1);
    tcg_gen_trunc_i64_tl(t2, t0);
    gen_store_spr(SPR_MQ, t2);
    tcg_gen_shri_i64(t1, t0, 32);
    tcg_gen_trunc_i64_tl(cpu_gpr[rD(s->opcode)], t1);
    tcg_temp_free_i64(t0);
    tcg_temp_free_i64(t1);
    tcg_temp_free(t2);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rD(s->opcode)]);
    }
}

/* mulo - mulo. */
static void gen_mulo(DisasContext *s)
{
    int l1 = gen_new_label();
    TCGv_i64 t0 = tcg_temp_new_i64();
    TCGv_i64 t1 = tcg_temp_new_i64();
    TCGv t2 = tcg_temp_new();
    /* Start with XER OV disabled, the most likely case */
    tcg_gen_andi_tl(cpu_xer, cpu_xer, ~(1 << XER_OV));
    tcg_gen_extu_tl_i64(t0, cpu_gpr[rA(s->opcode)]);
    tcg_gen_extu_tl_i64(t1, cpu_gpr[rB(s->opcode)]);
    tcg_gen_mul_i64(t0, t0, t1);
    tcg_gen_trunc_i64_tl(t2, t0);
    gen_store_spr(SPR_MQ, t2);
    tcg_gen_shri_i64(t1, t0, 32);
    tcg_gen_trunc_i64_tl(cpu_gpr[rD(s->opcode)], t1);
    tcg_gen_ext32s_i64(t1, t0);
    tcg_gen_brcond_i64(TCG_COND_EQ, t0, t1, l1);
    tcg_gen_ori_tl(cpu_xer, cpu_xer, (1 << XER_OV) | (1 << XER_SO));
    gen_set_label(l1);
    tcg_temp_free_i64(t0);
    tcg_temp_free_i64(t1);
    tcg_temp_free(t2);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rD(s->opcode)]);
    }
}

/* nabs - nabs. */
static void gen_nabs(DisasContext *s)
{
    int l1 = gen_new_label();
    int l2 = gen_new_label();
    tcg_gen_brcondi_tl(TCG_COND_GT, cpu_gpr[rA(s->opcode)], 0, l1);
    tcg_gen_mov_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)]);
    tcg_gen_br(l2);
    gen_set_label(l1);
    tcg_gen_neg_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)]);
    gen_set_label(l2);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rD(s->opcode)]);
    }
}

/* nabso - nabso. */
static void gen_nabso(DisasContext *s)
{
    int l1 = gen_new_label();
    int l2 = gen_new_label();
    tcg_gen_brcondi_tl(TCG_COND_GT, cpu_gpr[rA(s->opcode)], 0, l1);
    tcg_gen_mov_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)]);
    tcg_gen_br(l2);
    gen_set_label(l1);
    tcg_gen_neg_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)]);
    gen_set_label(l2);
    /* nabs never overflows */
    tcg_gen_andi_tl(cpu_xer, cpu_xer, ~(1 << XER_OV));
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rD(s->opcode)]);
    }
}

/* rlmi - rlmi. */
static void gen_rlmi(DisasContext *s)
{
    uint32_t mb = MB(s->opcode);
    uint32_t me = ME(s->opcode);
    TCGv t0 = tcg_temp_new();
    tcg_gen_andi_tl(t0, cpu_gpr[rB(s->opcode)], 0x1F);
    tcg_gen_rotl_tl(t0, cpu_gpr[rS(s->opcode)], t0);
    tcg_gen_andi_tl(t0, t0, MASK(mb, me));
    tcg_gen_andi_tl(cpu_gpr[rA(s->opcode)], cpu_gpr[rA(s->opcode)], ~MASK(mb, me));
    tcg_gen_or_tl(cpu_gpr[rA(s->opcode)], cpu_gpr[rA(s->opcode)], t0);
    tcg_temp_free(t0);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* rrib - rrib. */
static void gen_rrib(DisasContext *s)
{
    TCGv t0 = tcg_temp_new();
    TCGv t1 = tcg_temp_new();
    tcg_gen_andi_tl(t0, cpu_gpr[rB(s->opcode)], 0x1F);
    tcg_gen_movi_tl(t1, 0x80000000);
    tcg_gen_shr_tl(t1, t1, t0);
    tcg_gen_shr_tl(t0, cpu_gpr[rS(s->opcode)], t0);
    tcg_gen_and_tl(t0, t0, t1);
    tcg_gen_andc_tl(t1, cpu_gpr[rA(s->opcode)], t1);
    tcg_gen_or_tl(cpu_gpr[rA(s->opcode)], t0, t1);
    tcg_temp_free(t0);
    tcg_temp_free(t1);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* sle - sle. */
static void gen_sle(DisasContext *s)
{
    TCGv t0 = tcg_temp_new();
    TCGv t1 = tcg_temp_new();
    tcg_gen_andi_tl(t1, cpu_gpr[rB(s->opcode)], 0x1F);
    tcg_gen_shl_tl(t0, cpu_gpr[rS(s->opcode)], t1);
    tcg_gen_subfi_tl(t1, 32, t1);
    tcg_gen_shr_tl(t1, cpu_gpr[rS(s->opcode)], t1);
    tcg_gen_or_tl(t1, t0, t1);
    tcg_gen_mov_tl(cpu_gpr[rA(s->opcode)], t0);
    gen_store_spr(SPR_MQ, t1);
    tcg_temp_free(t0);
    tcg_temp_free(t1);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* sleq - sleq. */
static void gen_sleq(DisasContext *s)
{
    TCGv t0 = tcg_temp_new();
    TCGv t1 = tcg_temp_new();
    TCGv t2 = tcg_temp_new();
    tcg_gen_andi_tl(t0, cpu_gpr[rB(s->opcode)], 0x1F);
    tcg_gen_movi_tl(t2, 0xFFFFFFFF);
    tcg_gen_shl_tl(t2, t2, t0);
    tcg_gen_rotl_tl(t0, cpu_gpr[rS(s->opcode)], t0);
    gen_load_spr(t1, SPR_MQ);
    gen_store_spr(SPR_MQ, t0);
    tcg_gen_and_tl(t0, t0, t2);
    tcg_gen_andc_tl(t1, t1, t2);
    tcg_gen_or_tl(cpu_gpr[rA(s->opcode)], t0, t1);
    tcg_temp_free(t0);
    tcg_temp_free(t1);
    tcg_temp_free(t2);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* sliq - sliq. */
static void gen_sliq(DisasContext *s)
{
    int sh = SH(s->opcode);
    TCGv t0 = tcg_temp_new();
    TCGv t1 = tcg_temp_new();
    tcg_gen_shli_tl(t0, cpu_gpr[rS(s->opcode)], sh);
    tcg_gen_shri_tl(t1, cpu_gpr[rS(s->opcode)], 32 - sh);
    tcg_gen_or_tl(t1, t0, t1);
    tcg_gen_mov_tl(cpu_gpr[rA(s->opcode)], t0);
    gen_store_spr(SPR_MQ, t1);
    tcg_temp_free(t0);
    tcg_temp_free(t1);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* slliq - slliq. */
static void gen_slliq(DisasContext *s)
{
    int sh = SH(s->opcode);
    TCGv t0 = tcg_temp_new();
    TCGv t1 = tcg_temp_new();
    tcg_gen_rotli_tl(t0, cpu_gpr[rS(s->opcode)], sh);
    gen_load_spr(t1, SPR_MQ);
    gen_store_spr(SPR_MQ, t0);
    tcg_gen_andi_tl(t0, t0, (0xFFFFFFFFU << sh));
    tcg_gen_andi_tl(t1, t1, ~(0xFFFFFFFFU << sh));
    tcg_gen_or_tl(cpu_gpr[rA(s->opcode)], t0, t1);
    tcg_temp_free(t0);
    tcg_temp_free(t1);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* sllq - sllq. */
static void gen_sllq(DisasContext *s)
{
    int l1 = gen_new_label();
    int l2 = gen_new_label();
    TCGv t0 = tcg_temp_local_new();
    TCGv t1 = tcg_temp_local_new();
    TCGv t2 = tcg_temp_local_new();
    tcg_gen_andi_tl(t2, cpu_gpr[rB(s->opcode)], 0x1F);
    tcg_gen_movi_tl(t1, 0xFFFFFFFF);
    tcg_gen_shl_tl(t1, t1, t2);
    tcg_gen_andi_tl(t0, cpu_gpr[rB(s->opcode)], 0x20);
    tcg_gen_brcondi_tl(TCG_COND_EQ, t0, 0, l1);
    gen_load_spr(t0, SPR_MQ);
    tcg_gen_and_tl(cpu_gpr[rA(s->opcode)], t0, t1);
    tcg_gen_br(l2);
    gen_set_label(l1);
    tcg_gen_shl_tl(t0, cpu_gpr[rS(s->opcode)], t2);
    gen_load_spr(t2, SPR_MQ);
    tcg_gen_andc_tl(t1, t2, t1);
    tcg_gen_or_tl(cpu_gpr[rA(s->opcode)], t0, t1);
    gen_set_label(l2);
    tcg_temp_free(t0);
    tcg_temp_free(t1);
    tcg_temp_free(t2);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* slq - slq. */
static void gen_slq(DisasContext *s)
{
    int l1 = gen_new_label();
    TCGv t0 = tcg_temp_new();
    TCGv t1 = tcg_temp_new();
    tcg_gen_andi_tl(t1, cpu_gpr[rB(s->opcode)], 0x1F);
    tcg_gen_shl_tl(t0, cpu_gpr[rS(s->opcode)], t1);
    tcg_gen_subfi_tl(t1, 32, t1);
    tcg_gen_shr_tl(t1, cpu_gpr[rS(s->opcode)], t1);
    tcg_gen_or_tl(t1, t0, t1);
    gen_store_spr(SPR_MQ, t1);
    tcg_gen_andi_tl(t1, cpu_gpr[rB(s->opcode)], 0x20);
    tcg_gen_mov_tl(cpu_gpr[rA(s->opcode)], t0);
    tcg_gen_brcondi_tl(TCG_COND_EQ, t1, 0, l1);
    tcg_gen_movi_tl(cpu_gpr[rA(s->opcode)], 0);
    gen_set_label(l1);
    tcg_temp_free(t0);
    tcg_temp_free(t1);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* sraiq - sraiq. */
static void gen_sraiq(DisasContext *s)
{
    int sh = SH(s->opcode);
    int l1 = gen_new_label();
    TCGv t0 = tcg_temp_new();
    TCGv t1 = tcg_temp_new();
    tcg_gen_shri_tl(t0, cpu_gpr[rS(s->opcode)], sh);
    tcg_gen_shli_tl(t1, cpu_gpr[rS(s->opcode)], 32 - sh);
    tcg_gen_or_tl(t0, t0, t1);
    gen_store_spr(SPR_MQ, t0);
    tcg_gen_andi_tl(cpu_xer, cpu_xer, ~(1 << XER_CA));
    tcg_gen_brcondi_tl(TCG_COND_EQ, t1, 0, l1);
    tcg_gen_brcondi_tl(TCG_COND_GE, cpu_gpr[rS(s->opcode)], 0, l1);
    tcg_gen_ori_tl(cpu_xer, cpu_xer, (1 << XER_CA));
    gen_set_label(l1);
    tcg_gen_sari_tl(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)], sh);
    tcg_temp_free(t0);
    tcg_temp_free(t1);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* sraq - sraq. */
static void gen_sraq(DisasContext *s)
{
    int l1 = gen_new_label();
    int l2 = gen_new_label();
    TCGv t0 = tcg_temp_new();
    TCGv t1 = tcg_temp_local_new();
    TCGv t2 = tcg_temp_local_new();
    tcg_gen_andi_tl(t2, cpu_gpr[rB(s->opcode)], 0x1F);
    tcg_gen_shr_tl(t0, cpu_gpr[rS(s->opcode)], t2);
    tcg_gen_sar_tl(t1, cpu_gpr[rS(s->opcode)], t2);
    tcg_gen_subfi_tl(t2, 32, t2);
    tcg_gen_shl_tl(t2, cpu_gpr[rS(s->opcode)], t2);
    tcg_gen_or_tl(t0, t0, t2);
    gen_store_spr(SPR_MQ, t0);
    tcg_gen_andi_tl(t0, cpu_gpr[rB(s->opcode)], 0x20);
    tcg_gen_brcondi_tl(TCG_COND_EQ, t2, 0, l1);
    tcg_gen_mov_tl(t2, cpu_gpr[rS(s->opcode)]);
    tcg_gen_sari_tl(t1, cpu_gpr[rS(s->opcode)], 31);
    gen_set_label(l1);
    tcg_temp_free(t0);
    tcg_gen_mov_tl(cpu_gpr[rA(s->opcode)], t1);
    tcg_gen_andi_tl(cpu_xer, cpu_xer, ~(1 << XER_CA));
    tcg_gen_brcondi_tl(TCG_COND_GE, t1, 0, l2);
    tcg_gen_brcondi_tl(TCG_COND_EQ, t2, 0, l2);
    tcg_gen_ori_tl(cpu_xer, cpu_xer, (1 << XER_CA));
    gen_set_label(l2);
    tcg_temp_free(t1);
    tcg_temp_free(t2);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* sre - sre. */
static void gen_sre(DisasContext *s)
{
    TCGv t0 = tcg_temp_new();
    TCGv t1 = tcg_temp_new();
    tcg_gen_andi_tl(t1, cpu_gpr[rB(s->opcode)], 0x1F);
    tcg_gen_shr_tl(t0, cpu_gpr[rS(s->opcode)], t1);
    tcg_gen_subfi_tl(t1, 32, t1);
    tcg_gen_shl_tl(t1, cpu_gpr[rS(s->opcode)], t1);
    tcg_gen_or_tl(t1, t0, t1);
    tcg_gen_mov_tl(cpu_gpr[rA(s->opcode)], t0);
    gen_store_spr(SPR_MQ, t1);
    tcg_temp_free(t0);
    tcg_temp_free(t1);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* srea - srea. */
static void gen_srea(DisasContext *s)
{
    TCGv t0 = tcg_temp_new();
    TCGv t1 = tcg_temp_new();
    tcg_gen_andi_tl(t1, cpu_gpr[rB(s->opcode)], 0x1F);
    tcg_gen_rotr_tl(t0, cpu_gpr[rS(s->opcode)], t1);
    gen_store_spr(SPR_MQ, t0);
    tcg_gen_sar_tl(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)], t1);
    tcg_temp_free(t0);
    tcg_temp_free(t1);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* sreq */
static void gen_sreq(DisasContext *s)
{
    TCGv t0 = tcg_temp_new();
    TCGv t1 = tcg_temp_new();
    TCGv t2 = tcg_temp_new();
    tcg_gen_andi_tl(t0, cpu_gpr[rB(s->opcode)], 0x1F);
    tcg_gen_movi_tl(t1, 0xFFFFFFFF);
    tcg_gen_shr_tl(t1, t1, t0);
    tcg_gen_rotr_tl(t0, cpu_gpr[rS(s->opcode)], t0);
    gen_load_spr(t2, SPR_MQ);
    gen_store_spr(SPR_MQ, t0);
    tcg_gen_and_tl(t0, t0, t1);
    tcg_gen_andc_tl(t2, t2, t1);
    tcg_gen_or_tl(cpu_gpr[rA(s->opcode)], t0, t2);
    tcg_temp_free(t0);
    tcg_temp_free(t1);
    tcg_temp_free(t2);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* sriq */
static void gen_sriq(DisasContext *s)
{
    int sh = SH(s->opcode);
    TCGv t0 = tcg_temp_new();
    TCGv t1 = tcg_temp_new();
    tcg_gen_shri_tl(t0, cpu_gpr[rS(s->opcode)], sh);
    tcg_gen_shli_tl(t1, cpu_gpr[rS(s->opcode)], 32 - sh);
    tcg_gen_or_tl(t1, t0, t1);
    tcg_gen_mov_tl(cpu_gpr[rA(s->opcode)], t0);
    gen_store_spr(SPR_MQ, t1);
    tcg_temp_free(t0);
    tcg_temp_free(t1);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* srliq */
static void gen_srliq(DisasContext *s)
{
    int sh = SH(s->opcode);
    TCGv t0 = tcg_temp_new();
    TCGv t1 = tcg_temp_new();
    tcg_gen_rotri_tl(t0, cpu_gpr[rS(s->opcode)], sh);
    gen_load_spr(t1, SPR_MQ);
    gen_store_spr(SPR_MQ, t0);
    tcg_gen_andi_tl(t0, t0, (0xFFFFFFFFU >> sh));
    tcg_gen_andi_tl(t1, t1, ~(0xFFFFFFFFU >> sh));
    tcg_gen_or_tl(cpu_gpr[rA(s->opcode)], t0, t1);
    tcg_temp_free(t0);
    tcg_temp_free(t1);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* srlq */
static void gen_srlq(DisasContext *s)
{
    int l1 = gen_new_label();
    int l2 = gen_new_label();
    TCGv t0 = tcg_temp_local_new();
    TCGv t1 = tcg_temp_local_new();
    TCGv t2 = tcg_temp_local_new();
    tcg_gen_andi_tl(t2, cpu_gpr[rB(s->opcode)], 0x1F);
    tcg_gen_movi_tl(t1, 0xFFFFFFFF);
    tcg_gen_shr_tl(t2, t1, t2);
    tcg_gen_andi_tl(t0, cpu_gpr[rB(s->opcode)], 0x20);
    tcg_gen_brcondi_tl(TCG_COND_EQ, t0, 0, l1);
    gen_load_spr(t0, SPR_MQ);
    tcg_gen_and_tl(cpu_gpr[rA(s->opcode)], t0, t2);
    tcg_gen_br(l2);
    gen_set_label(l1);
    tcg_gen_shr_tl(t0, cpu_gpr[rS(s->opcode)], t2);
    tcg_gen_and_tl(t0, t0, t2);
    gen_load_spr(t1, SPR_MQ);
    tcg_gen_andc_tl(t1, t1, t2);
    tcg_gen_or_tl(cpu_gpr[rA(s->opcode)], t0, t1);
    gen_set_label(l2);
    tcg_temp_free(t0);
    tcg_temp_free(t1);
    tcg_temp_free(t2);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* srq */
static void gen_srq(DisasContext *s)
{
    int l1 = gen_new_label();
    TCGv t0 = tcg_temp_new();
    TCGv t1 = tcg_temp_new();
    tcg_gen_andi_tl(t1, cpu_gpr[rB(s->opcode)], 0x1F);
    tcg_gen_shr_tl(t0, cpu_gpr[rS(s->opcode)], t1);
    tcg_gen_subfi_tl(t1, 32, t1);
    tcg_gen_shl_tl(t1, cpu_gpr[rS(s->opcode)], t1);
    tcg_gen_or_tl(t1, t0, t1);
    gen_store_spr(SPR_MQ, t1);
    tcg_gen_andi_tl(t1, cpu_gpr[rB(s->opcode)], 0x20);
    tcg_gen_mov_tl(cpu_gpr[rA(s->opcode)], t0);
    tcg_gen_brcondi_tl(TCG_COND_EQ, t0, 0, l1);
    tcg_gen_movi_tl(cpu_gpr[rA(s->opcode)], 0);
    gen_set_label(l1);
    tcg_temp_free(t0);
    tcg_temp_free(t1);
    if(unlikely(Rc(s->opcode) != 0)) {
        gen_set_Rc0(s, cpu_gpr[rA(s->opcode)]);
    }
}

/* PowerPC 602 specific instructions */

/* dsa  */
static void gen_dsa(DisasContext *s)
{
    /* XXX: TODO */
    gen_inval_exception(s, POWERPC_EXCP_INVAL_INVAL);
}

/* esa */
static void gen_esa(DisasContext *s)
{
    /* XXX: TODO */
    gen_inval_exception(s, POWERPC_EXCP_INVAL_INVAL);
}

/* mfrom */
static void gen_mfrom(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    gen_helper_602_mfrom(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)]);
}

/* 602 - 603 - G2 TLB management */

/* tlbld */
static void gen_tlbld_6xx(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    gen_helper_6xx_tlbd(cpu_gpr[rB(s->opcode)]);
}

/* tlbli */
static void gen_tlbli_6xx(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    gen_helper_6xx_tlbi(cpu_gpr[rB(s->opcode)]);
}

/* 74xx TLB management */

/* tlbld */
static void gen_tlbld_74xx(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    gen_helper_74xx_tlbd(cpu_gpr[rB(s->opcode)]);
}

/* tlbli */
static void gen_tlbli_74xx(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    gen_helper_74xx_tlbi(cpu_gpr[rB(s->opcode)]);
}

/* POWER instructions not in PowerPC 601 */

/* clf */
static void gen_clf(DisasContext *s)
{
    /* Cache line flush: implemented as no-op */
}

/* cli */
static void gen_cli(DisasContext *s)
{
    /* Cache line invalidate: privileged and treated as no-op */
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
}

/* dclst */
static void gen_dclst(DisasContext *s)
{
    /* Data cache line store: treated as no-op */
}

static void gen_mfsri(DisasContext *s)
{
    int ra = rA(s->opcode);
    int rd = rD(s->opcode);
    TCGv t0;
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    t0 = tcg_temp_new();
    gen_addr_reg_index(s, t0);
    tcg_gen_shri_tl(t0, t0, 28);
    tcg_gen_andi_tl(t0, t0, 0xF);
    gen_helper_load_sr(cpu_gpr[rd], t0);
    tcg_temp_free(t0);
    if(ra != 0 && ra != rd) {
        tcg_gen_mov_tl(cpu_gpr[ra], cpu_gpr[rd]);
    }
}

static void gen_rac(DisasContext *s)
{
    TCGv t0;
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    t0 = tcg_temp_new();
    gen_addr_reg_index(s, t0);
    gen_helper_rac(cpu_gpr[rD(s->opcode)], t0);
    tcg_temp_free(t0);
}

static void gen_rfsvc(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    gen_helper_rfsvc();
    gen_sync_exception(s);
}

/* svc is not implemented for now */

/* POWER2 specific instructions */
/* Quad manipulation (load/store two floats at a time) */

/* lfq */
static void gen_lfq(DisasContext *s)
{
    int rd = rD(s->opcode);
    TCGv t0;
    gen_set_access_type(s, ACCESS_FLOAT);
    t0 = tcg_temp_new();
    gen_addr_imm_index(s, t0, 0);
    gen_qemu_ld64(s, cpu_fpr[rd], t0);
    gen_addr_add(s, t0, t0, 8);
    gen_qemu_ld64(s, cpu_fpr[(rd + 1) % 32], t0);
    tcg_temp_free(t0);
}

/* lfqu */
static void gen_lfqu(DisasContext *s)
{
    int ra = rA(s->opcode);
    int rd = rD(s->opcode);
    TCGv t0, t1;
    gen_set_access_type(s, ACCESS_FLOAT);
    t0 = tcg_temp_new();
    t1 = tcg_temp_new();
    gen_addr_imm_index(s, t0, 0);
    gen_qemu_ld64(s, cpu_fpr[rd], t0);
    gen_addr_add(s, t1, t0, 8);
    gen_qemu_ld64(s, cpu_fpr[(rd + 1) % 32], t1);
    if(ra != 0) {
        tcg_gen_mov_tl(cpu_gpr[ra], t0);
    }
    tcg_temp_free(t0);
    tcg_temp_free(t1);
}

/* lfqux */
static void gen_lfqux(DisasContext *s)
{
    int ra = rA(s->opcode);
    int rd = rD(s->opcode);
    gen_set_access_type(s, ACCESS_FLOAT);
    TCGv t0, t1;
    t0 = tcg_temp_new();
    gen_addr_reg_index(s, t0);
    gen_qemu_ld64(s, cpu_fpr[rd], t0);
    t1 = tcg_temp_new();
    gen_addr_add(s, t1, t0, 8);
    gen_qemu_ld64(s, cpu_fpr[(rd + 1) % 32], t1);
    tcg_temp_free(t1);
    if(ra != 0) {
        tcg_gen_mov_tl(cpu_gpr[ra], t0);
    }
    tcg_temp_free(t0);
}

/* lfqx */
static void gen_lfqx(DisasContext *s)
{
    int rd = rD(s->opcode);
    TCGv t0;
    gen_set_access_type(s, ACCESS_FLOAT);
    t0 = tcg_temp_new();
    gen_addr_reg_index(s, t0);
    gen_qemu_ld64(s, cpu_fpr[rd], t0);
    gen_addr_add(s, t0, t0, 8);
    gen_qemu_ld64(s, cpu_fpr[(rd + 1) % 32], t0);
    tcg_temp_free(t0);
}

/* stfq */
static void gen_stfq(DisasContext *s)
{
    int rd = rD(s->opcode);
    TCGv t0;
    gen_set_access_type(s, ACCESS_FLOAT);
    t0 = tcg_temp_local_new();
    gen_addr_imm_index(s, t0, 0);
    gen_qemu_st64(s, cpu_fpr[rd], t0);
    gen_addr_add(s, t0, t0, 8);
    gen_qemu_st64(s, cpu_fpr[(rd + 1) % 32], t0);
    tcg_temp_free(t0);
}

/* stfqu */
static void gen_stfqu(DisasContext *s)
{
    int ra = rA(s->opcode);
    int rd = rD(s->opcode);
    TCGv t0, t1;
    gen_set_access_type(s, ACCESS_FLOAT);
    t0 = tcg_temp_local_new();
    gen_addr_imm_index(s, t0, 0);
    gen_qemu_st64(s, cpu_fpr[rd], t0);
    t1 = tcg_temp_local_new();
    gen_addr_add(s, t1, t0, 8);
    gen_qemu_st64(s, cpu_fpr[(rd + 1) % 32], t1);
    tcg_temp_free(t1);
    if(ra != 0) {
        tcg_gen_mov_tl(cpu_gpr[ra], t0);
    }
    tcg_temp_free(t0);
}

/* stfqux */
static void gen_stfqux(DisasContext *s)
{
    int ra = rA(s->opcode);
    int rd = rD(s->opcode);
    TCGv t0, t1;
    gen_set_access_type(s, ACCESS_FLOAT);
    t0 = tcg_temp_local_new();
    gen_addr_reg_index(s, t0);
    gen_qemu_st64(s, cpu_fpr[rd], t0);
    t1 = tcg_temp_local_new();
    gen_addr_add(s, t1, t0, 8);
    gen_qemu_st64(s, cpu_fpr[(rd + 1) % 32], t1);
    tcg_temp_free(t1);
    if(ra != 0) {
        tcg_gen_mov_tl(cpu_gpr[ra], t0);
    }
    tcg_temp_free(t0);
}

/* stfqx */
static void gen_stfqx(DisasContext *s)
{
    int rd = rD(s->opcode);
    TCGv t0;
    gen_set_access_type(s, ACCESS_FLOAT);
    t0 = tcg_temp_local_new();
    gen_addr_reg_index(s, t0);
    gen_qemu_st64(s, cpu_fpr[rd], t0);
    gen_addr_add(s, t0, t0, 8);
    gen_qemu_st64(s, cpu_fpr[(rd + 1) % 32], t0);
    tcg_temp_free(t0);
}

/* BookE specific instructions */

/* XXX: not implemented on 440 ? */
static void gen_mfapidi(DisasContext *s)
{
    /* XXX: TODO */
    gen_inval_exception(s, POWERPC_EXCP_INVAL_INVAL);
}

/* XXX: not implemented on 440 ? */
static void gen_tlbiva(DisasContext *s)
{
    TCGv t0;
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    t0 = tcg_temp_new();
    gen_addr_reg_index(s, t0);
    gen_helper_tlbie(cpu_gpr[rB(s->opcode)]);
    tcg_temp_free(t0);
}

/* All 405 MAC instructions are translated here */
static inline void gen_405_mulladd_insn(DisasContext *s, int opc2, int opc3, int ra, int rb, int rt, int Rc)
{
    TCGv t0, t1;

    t0 = tcg_temp_local_new();
    t1 = tcg_temp_local_new();

    switch(opc3 & 0x0D) {
        case 0x05:
            /* macchw    - macchw.    - macchwo   - macchwo.   */
            /* macchws   - macchws.   - macchwso  - macchwso.  */
            /* nmacchw   - nmacchw.   - nmacchwo  - nmacchwo.  */
            /* nmacchws  - nmacchws.  - nmacchwso - nmacchwso. */
            /* mulchw - mulchw. */
            tcg_gen_ext16s_tl(t0, cpu_gpr[ra]);
            tcg_gen_sari_tl(t1, cpu_gpr[rb], 16);
            tcg_gen_ext16s_tl(t1, t1);
            break;
        case 0x04:
            /* macchwu   - macchwu.   - macchwuo  - macchwuo.  */
            /* macchwsu  - macchwsu.  - macchwsuo - macchwsuo. */
            /* mulchwu - mulchwu. */
            tcg_gen_ext16u_tl(t0, cpu_gpr[ra]);
            tcg_gen_shri_tl(t1, cpu_gpr[rb], 16);
            tcg_gen_ext16u_tl(t1, t1);
            break;
        case 0x01:
            /* machhw    - machhw.    - machhwo   - machhwo.   */
            /* machhws   - machhws.   - machhwso  - machhwso.  */
            /* nmachhw   - nmachhw.   - nmachhwo  - nmachhwo.  */
            /* nmachhws  - nmachhws.  - nmachhwso - nmachhwso. */
            /* mulhhw - mulhhw. */
            tcg_gen_sari_tl(t0, cpu_gpr[ra], 16);
            tcg_gen_ext16s_tl(t0, t0);
            tcg_gen_sari_tl(t1, cpu_gpr[rb], 16);
            tcg_gen_ext16s_tl(t1, t1);
            break;
        case 0x00:
            /* machhwu   - machhwu.   - machhwuo  - machhwuo.  */
            /* machhwsu  - machhwsu.  - machhwsuo - machhwsuo. */
            /* mulhhwu - mulhhwu. */
            tcg_gen_shri_tl(t0, cpu_gpr[ra], 16);
            tcg_gen_ext16u_tl(t0, t0);
            tcg_gen_shri_tl(t1, cpu_gpr[rb], 16);
            tcg_gen_ext16u_tl(t1, t1);
            break;
        case 0x0D:
            /* maclhw    - maclhw.    - maclhwo   - maclhwo.   */
            /* maclhws   - maclhws.   - maclhwso  - maclhwso.  */
            /* nmaclhw   - nmaclhw.   - nmaclhwo  - nmaclhwo.  */
            /* nmaclhws  - nmaclhws.  - nmaclhwso - nmaclhwso. */
            /* mullhw - mullhw. */
            tcg_gen_ext16s_tl(t0, cpu_gpr[ra]);
            tcg_gen_ext16s_tl(t1, cpu_gpr[rb]);
            break;
        case 0x0C:
            /* maclhwu   - maclhwu.   - maclhwuo  - maclhwuo.  */
            /* maclhwsu  - maclhwsu.  - maclhwsuo - maclhwsuo. */
            /* mullhwu - mullhwu. */
            tcg_gen_ext16u_tl(t0, cpu_gpr[ra]);
            tcg_gen_ext16u_tl(t1, cpu_gpr[rb]);
            break;
    }
    if(opc2 & 0x04) {
        /* (n)multiply-and-accumulate (0x0C / 0x0E) */
        tcg_gen_mul_tl(t1, t0, t1);
        if(opc2 & 0x02) {
            /* nmultiply-and-accumulate (0x0E) */
            tcg_gen_sub_tl(t0, cpu_gpr[rt], t1);
        } else {
            /* multiply-and-accumulate (0x0C) */
            tcg_gen_add_tl(t0, cpu_gpr[rt], t1);
        }

        if(opc3 & 0x12) {
            /* Check overflow and/or saturate */
            int l1 = gen_new_label();

            if(opc3 & 0x10) {
                /* Start with XER OV disabled, the most likely case */
                tcg_gen_andi_tl(cpu_xer, cpu_xer, ~(1 << XER_OV));
            }
            if(opc3 & 0x01) {
                /* Signed */
                tcg_gen_xor_tl(t1, cpu_gpr[rt], t1);
                tcg_gen_brcondi_tl(TCG_COND_GE, t1, 0, l1);
                tcg_gen_xor_tl(t1, cpu_gpr[rt], t0);
                tcg_gen_brcondi_tl(TCG_COND_LT, t1, 0, l1);
                if(opc3 & 0x02) {
                    /* Saturate */
                    tcg_gen_sari_tl(t0, cpu_gpr[rt], 31);
                    tcg_gen_xori_tl(t0, t0, 0x7fffffff);
                }
            } else {
                /* Unsigned */
                tcg_gen_brcond_tl(TCG_COND_GEU, t0, t1, l1);
                if(opc3 & 0x02) {
                    /* Saturate */
                    tcg_gen_movi_tl(t0, UINT32_MAX);
                }
            }
            if(opc3 & 0x10) {
                /* Check overflow */
                tcg_gen_ori_tl(cpu_xer, cpu_xer, (1 << XER_OV) | (1 << XER_SO));
            }
            gen_set_label(l1);
            tcg_gen_mov_tl(cpu_gpr[rt], t0);
        }
    } else {
        tcg_gen_mul_tl(cpu_gpr[rt], t0, t1);
    }
    tcg_temp_free(t0);
    tcg_temp_free(t1);
    if(unlikely(Rc) != 0) {
        /* Update Rc0 */
        gen_set_Rc0(s, cpu_gpr[rt]);
    }
}

#define GEN_MAC_HANDLER(name, opc2, opc3)                                                                \
    static void glue(gen_, name)(DisasContext * s)                                                       \
    {                                                                                                    \
        gen_405_mulladd_insn(s, opc2, opc3, rA(s->opcode), rB(s->opcode), rD(s->opcode), Rc(s->opcode)); \
    }

/* macchw    - macchw.    */
GEN_MAC_HANDLER(macchw, 0x0C, 0x05);
/* macchwo   - macchwo.   */
GEN_MAC_HANDLER(macchwo, 0x0C, 0x15);
/* macchws   - macchws.   */
GEN_MAC_HANDLER(macchws, 0x0C, 0x07);
/* macchwso  - macchwso.  */
GEN_MAC_HANDLER(macchwso, 0x0C, 0x17);
/* macchwsu  - macchwsu.  */
GEN_MAC_HANDLER(macchwsu, 0x0C, 0x06);
/* macchwsuo - macchwsuo. */
GEN_MAC_HANDLER(macchwsuo, 0x0C, 0x16);
/* macchwu   - macchwu.   */
GEN_MAC_HANDLER(macchwu, 0x0C, 0x04);
/* macchwuo  - macchwuo.  */
GEN_MAC_HANDLER(macchwuo, 0x0C, 0x14);
/* machhw    - machhw.    */
GEN_MAC_HANDLER(machhw, 0x0C, 0x01);
/* machhwo   - machhwo.   */
GEN_MAC_HANDLER(machhwo, 0x0C, 0x11);
/* machhws   - machhws.   */
GEN_MAC_HANDLER(machhws, 0x0C, 0x03);
/* machhwso  - machhwso.  */
GEN_MAC_HANDLER(machhwso, 0x0C, 0x13);
/* machhwsu  - machhwsu.  */
GEN_MAC_HANDLER(machhwsu, 0x0C, 0x02);
/* machhwsuo - machhwsuo. */
GEN_MAC_HANDLER(machhwsuo, 0x0C, 0x12);
/* machhwu   - machhwu.   */
GEN_MAC_HANDLER(machhwu, 0x0C, 0x00);
/* machhwuo  - machhwuo.  */
GEN_MAC_HANDLER(machhwuo, 0x0C, 0x10);
/* maclhw    - maclhw.    */
GEN_MAC_HANDLER(maclhw, 0x0C, 0x0D);
/* maclhwo   - maclhwo.   */
GEN_MAC_HANDLER(maclhwo, 0x0C, 0x1D);
/* maclhws   - maclhws.   */
GEN_MAC_HANDLER(maclhws, 0x0C, 0x0F);
/* maclhwso  - maclhwso.  */
GEN_MAC_HANDLER(maclhwso, 0x0C, 0x1F);
/* maclhwu   - maclhwu.   */
GEN_MAC_HANDLER(maclhwu, 0x0C, 0x0C);
/* maclhwuo  - maclhwuo.  */
GEN_MAC_HANDLER(maclhwuo, 0x0C, 0x1C);
/* maclhwsu  - maclhwsu.  */
GEN_MAC_HANDLER(maclhwsu, 0x0C, 0x0E);
/* maclhwsuo - maclhwsuo. */
GEN_MAC_HANDLER(maclhwsuo, 0x0C, 0x1E);
/* nmacchw   - nmacchw.   */
GEN_MAC_HANDLER(nmacchw, 0x0E, 0x05);
/* nmacchwo  - nmacchwo.  */
GEN_MAC_HANDLER(nmacchwo, 0x0E, 0x15);
/* nmacchws  - nmacchws.  */
GEN_MAC_HANDLER(nmacchws, 0x0E, 0x07);
/* nmacchwso - nmacchwso. */
GEN_MAC_HANDLER(nmacchwso, 0x0E, 0x17);
/* nmachhw   - nmachhw.   */
GEN_MAC_HANDLER(nmachhw, 0x0E, 0x01);
/* nmachhwo  - nmachhwo.  */
GEN_MAC_HANDLER(nmachhwo, 0x0E, 0x11);
/* nmachhws  - nmachhws.  */
GEN_MAC_HANDLER(nmachhws, 0x0E, 0x03);
/* nmachhwso - nmachhwso. */
GEN_MAC_HANDLER(nmachhwso, 0x0E, 0x13);
/* nmaclhw   - nmaclhw.   */
GEN_MAC_HANDLER(nmaclhw, 0x0E, 0x0D);
/* nmaclhwo  - nmaclhwo.  */
GEN_MAC_HANDLER(nmaclhwo, 0x0E, 0x1D);
/* nmaclhws  - nmaclhws.  */
GEN_MAC_HANDLER(nmaclhws, 0x0E, 0x0F);
/* nmaclhwso - nmaclhwso. */
GEN_MAC_HANDLER(nmaclhwso, 0x0E, 0x1F);

/* mulchw  - mulchw.  */
GEN_MAC_HANDLER(mulchw, 0x08, 0x05);
/* mulchwu - mulchwu. */
GEN_MAC_HANDLER(mulchwu, 0x08, 0x04);
/* mulhhw  - mulhhw.  */
GEN_MAC_HANDLER(mulhhw, 0x08, 0x01);
/* mulhhwu - mulhhwu. */
GEN_MAC_HANDLER(mulhhwu, 0x08, 0x00);
/* mullhw  - mullhw.  */
GEN_MAC_HANDLER(mullhw, 0x08, 0x0D);
/* mullhwu - mullhwu. */
GEN_MAC_HANDLER(mullhwu, 0x08, 0x0C);

/* mfdcr */
static void gen_mfdcr(DisasContext *s)
{
    TCGv dcrn;
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_REG);
        return;
    }
    /* NIP cannot be restored if the memory exception comes from an helper */
    gen_update_nip(s, s->base.pc - 4);
    dcrn = tcg_const_tl(SPR(s->opcode));
    gen_helper_load_dcr(cpu_gpr[rD(s->opcode)], dcrn);
    tcg_temp_free(dcrn);
}

/* mtdcr */
static void gen_mtdcr(DisasContext *s)
{
    TCGv dcrn;
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_REG);
        return;
    }
    /* NIP cannot be restored if the memory exception comes from an helper */
    gen_update_nip(s, s->base.pc - 4);
    dcrn = tcg_const_tl(SPR(s->opcode));
    gen_helper_store_dcr(dcrn, cpu_gpr[rS(s->opcode)]);
    tcg_temp_free(dcrn);
}

/* mfdcrx */
/* XXX: not implemented on 440 ? */
static void gen_mfdcrx(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_REG);
        return;
    }
    /* NIP cannot be restored if the memory exception comes from an helper */
    gen_update_nip(s, s->base.pc - 4);
    gen_helper_load_dcr(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)]);
    /* Note: Rc update flag set leads to undefined state of Rc0 */
}

/* mtdcrx */
/* XXX: not implemented on 440 ? */
static void gen_mtdcrx(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_REG);
        return;
    }
    /* NIP cannot be restored if the memory exception comes from an helper */
    gen_update_nip(s, s->base.pc - 4);
    gen_helper_store_dcr(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)]);
    /* Note: Rc update flag set leads to undefined state of Rc0 */
}

/* mfdcrux (PPC 460) : user-mode access to DCR */
static void gen_mfdcrux(DisasContext *s)
{
    /* NIP cannot be restored if the memory exception comes from an helper */
    gen_update_nip(s, s->base.pc - 4);
    gen_helper_load_dcr(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)]);
    /* Note: Rc update flag set leads to undefined state of Rc0 */
}

/* mtdcrux (PPC 460) : user-mode access to DCR */
static void gen_mtdcrux(DisasContext *s)
{
    /* NIP cannot be restored if the memory exception comes from an helper */
    gen_update_nip(s, s->base.pc - 4);
    gen_helper_store_dcr(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)]);
    /* Note: Rc update flag set leads to undefined state of Rc0 */
}

/* dccci */
static void gen_dccci(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    /* interpreted as no-op */
}

/* dcread */
static void gen_dcread(DisasContext *s)
{
    TCGv EA, val;
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    gen_set_access_type(s, ACCESS_CACHE);
    EA = tcg_temp_new();
    gen_addr_reg_index(s, EA);
    val = tcg_temp_new();
    gen_qemu_ld32u(s, val, EA);
    tcg_temp_free(val);
    tcg_gen_mov_tl(cpu_gpr[rD(s->opcode)], EA);
    tcg_temp_free(EA);
}

/* icbt */
static void gen_icbt_40x(DisasContext *s)
{
    /* interpreted as no-op */
    /* XXX: specification say this is treated as a load by the MMU
     *      but does not generate any exception
     */
}

/* iccci */
static void gen_iccci(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    /* interpreted as no-op */
}

/* icread */
static void gen_icread(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    /* interpreted as no-op */
}

/* rfci (mem_idx only) */
static void gen_rfci_40x(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    /* Restore CPU state */
    gen_helper_40x_rfci();
    gen_sync_exception(s);
}

static void gen_rfci(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    /* Restore CPU state */
    gen_helper_rfci();
    gen_sync_exception(s);
}

/* BookE specific */

/* XXX: not implemented on 440 ? */
static void gen_rfdi(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    /* Restore CPU state */
    gen_helper_rfdi();
    gen_sync_exception(s);
}

/* XXX: not implemented on 440 ? */
static void gen_rfmci(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    /* Restore CPU state */
    gen_helper_rfmci();
    gen_sync_exception(s);
}

/* TLB management - PowerPC 405 implementation */

/* tlbre */
static void gen_tlbre_40x(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    switch(rB(s->opcode)) {
        case 0:
            gen_helper_4xx_tlbre_hi(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)]);
            break;
        case 1:
            gen_helper_4xx_tlbre_lo(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)]);
            break;
        default:
            gen_inval_exception(s, POWERPC_EXCP_INVAL_INVAL);
            break;
    }
}

/* tlbsx - tlbsx. */
static void gen_tlbsx_40x(DisasContext *s)
{
    TCGv t0;
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    t0 = tcg_temp_new();
    gen_addr_reg_index(s, t0);
    gen_helper_4xx_tlbsx(cpu_gpr[rD(s->opcode)], t0);
    tcg_temp_free(t0);
    if(Rc(s->opcode)) {
        int l1 = gen_new_label();
        tcg_gen_trunc_tl_i32(cpu_crf[0], cpu_xer);
        tcg_gen_shri_i32(cpu_crf[0], cpu_crf[0], XER_SO);
        tcg_gen_andi_i32(cpu_crf[0], cpu_crf[0], 1);
        tcg_gen_brcondi_tl(TCG_COND_EQ, cpu_gpr[rD(s->opcode)], -1, l1);
        tcg_gen_ori_i32(cpu_crf[0], cpu_crf[0], 0x02);
        gen_set_label(l1);
    }
}

/* tlbwe */
static void gen_tlbwe_40x(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    switch(rB(s->opcode)) {
        case 0:
            gen_helper_4xx_tlbwe_hi(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)]);
            break;
        case 1:
            gen_helper_4xx_tlbwe_lo(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)]);
            break;
        default:
            gen_inval_exception(s, POWERPC_EXCP_INVAL_INVAL);
            break;
    }
}

/* TLB management - PowerPC 440 implementation */

/* tlbre */
static void gen_tlbre_440(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    switch(rB(s->opcode)) {
        case 0:
        case 1:
        case 2: {
            TCGv_i32 t0 = tcg_const_i32(rB(s->opcode));
            gen_helper_440_tlbre(cpu_gpr[rD(s->opcode)], t0, cpu_gpr[rA(s->opcode)]);
            tcg_temp_free_i32(t0);
        } break;
        default:
            gen_inval_exception(s, POWERPC_EXCP_INVAL_INVAL);
            break;
    }
}

/* tlbsx - tlbsx. */
static void gen_tlbsx_440(DisasContext *s)
{
    TCGv t0;
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    t0 = tcg_temp_new();
    gen_addr_reg_index(s, t0);
    gen_helper_440_tlbsx(cpu_gpr[rD(s->opcode)], t0);
    tcg_temp_free(t0);
    if(Rc(s->opcode)) {
        int l1 = gen_new_label();
        tcg_gen_trunc_tl_i32(cpu_crf[0], cpu_xer);
        tcg_gen_shri_i32(cpu_crf[0], cpu_crf[0], XER_SO);
        tcg_gen_andi_i32(cpu_crf[0], cpu_crf[0], 1);
        tcg_gen_brcondi_tl(TCG_COND_EQ, cpu_gpr[rD(s->opcode)], -1, l1);
        tcg_gen_ori_i32(cpu_crf[0], cpu_crf[0], 0x02);
        gen_set_label(l1);
    }
}

/* tlbwe */
static void gen_tlbwe_440(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    switch(rB(s->opcode)) {
        case 0:
        case 1:
        case 2: {
            TCGv_i32 t0 = tcg_const_i32(rB(s->opcode));
            gen_helper_440_tlbwe(t0, cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)]);
            tcg_temp_free_i32(t0);
        } break;
        default:
            gen_inval_exception(s, POWERPC_EXCP_INVAL_INVAL);
            break;
    }
}

/* TLB management - PowerPC BookE 2.06 implementation */

/* tlbre */
static void gen_tlbre_booke206(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }

    gen_helper_booke206_tlbre();
}

/* tlbsx - tlbsx. */
static void gen_tlbsx_booke206(DisasContext *s)
{
    TCGv t0;
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }

    if(rA(s->opcode)) {
        t0 = tcg_temp_new();
        tcg_gen_mov_tl(t0, cpu_gpr[rD(s->opcode)]);
    } else {
        t0 = tcg_const_tl(0);
    }

    tcg_gen_add_tl(t0, t0, cpu_gpr[rB(s->opcode)]);
    gen_helper_booke206_tlbsx(t0);
}

/* tlbwe */
static void gen_tlbwe_booke206(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    gen_helper_booke206_tlbwe();
}

static void gen_tlbivax_booke206(DisasContext *s)
{
    TCGv t0;
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }

    t0 = tcg_temp_new();
    gen_addr_reg_index(s, t0);

    gen_helper_booke206_tlbivax(t0);
}

/* wrtee */
static void gen_wrtee(DisasContext *s)
{
    TCGv t0;
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    t0 = tcg_temp_new();
    tcg_gen_andi_tl(t0, cpu_gpr[rD(s->opcode)], (1 << MSR_EE));
    tcg_gen_andi_tl(cpu_msr, cpu_msr, ~(1 << MSR_EE));
    tcg_gen_or_tl(cpu_msr, cpu_msr, t0);
    tcg_temp_free(t0);
    /* Stop translation to have a chance to raise an exception
     * if we just set msr_ee to 1
     */
    gen_stop_exception(s);
}

/* wrteei */
static void gen_wrteei(DisasContext *s)
{
    if(unlikely(!s->base.mem_idx)) {
        gen_inval_exception(s, POWERPC_EXCP_PRIV_OPC);
        return;
    }
    if(s->opcode & 0x00008000) {
        tcg_gen_ori_tl(cpu_msr, cpu_msr, (1 << MSR_EE));
        /* Stop translation to have a chance to raise an exception */
        gen_stop_exception(s);
    } else {
        tcg_gen_andi_tl(cpu_msr, cpu_msr, ~(1 << MSR_EE));
    }
}

/* PowerPC 440 specific instructions */

/* dlmzb */
static void gen_dlmzb(DisasContext *s)
{
    TCGv_i32 t0 = tcg_const_i32(Rc(s->opcode));
    gen_helper_dlmzb(cpu_gpr[rA(s->opcode)], cpu_gpr[rS(s->opcode)], cpu_gpr[rB(s->opcode)], t0);
    tcg_temp_free_i32(t0);
}

/* mbar replaces eieio on 440 */
static void gen_mbar(DisasContext *s)
{
    /* interpreted as no-op */
}

/* msync replaces sync on 440 */
static void gen_msync(DisasContext *s)
{
    /* interpreted as no-op */
}

/* icbt */
static void gen_icbt_440(DisasContext *s)
{
    /* interpreted as no-op */
    /* XXX: specification say this is treated as a load by the MMU
     *      but does not generate any exception
     */
}

/***                      Altivec vector extension                         ***/
/* Altivec registers moves */

static inline TCGv_ptr gen_avr_ptr(int reg)
{
    TCGv_ptr r = tcg_temp_new_ptr();
    tcg_gen_addi_ptr(r, cpu_env, offsetof(CPUState, avr[reg]));
    return r;
}

#define GEN_VR_LDX(name, opc2, opc3)                       \
    static void glue(gen_, name)(DisasContext * s)         \
    {                                                      \
        TCGv EA;                                           \
        if(unlikely(!s->altivec_enabled)) {                \
            gen_exception(s, POWERPC_EXCP_VPU);            \
            return;                                        \
        }                                                  \
        gen_set_access_type(s, ACCESS_INT);                \
        EA = tcg_temp_new();                               \
        gen_addr_reg_index(s, EA);                         \
        tcg_gen_andi_tl(EA, EA, ~0xf);                     \
        if(s->le_mode) {                                   \
            gen_qemu_ld64(s, cpu_avrl[rD(s->opcode)], EA); \
            tcg_gen_addi_tl(EA, EA, 8);                    \
            gen_qemu_ld64(s, cpu_avrh[rD(s->opcode)], EA); \
        } else {                                           \
            gen_qemu_ld64(s, cpu_avrh[rD(s->opcode)], EA); \
            tcg_gen_addi_tl(EA, EA, 8);                    \
            gen_qemu_ld64(s, cpu_avrl[rD(s->opcode)], EA); \
        }                                                  \
        tcg_temp_free(EA);                                 \
    }

#define GEN_VR_STX(name, opc2, opc3)                       \
    static void gen_st##name(DisasContext *s)              \
    {                                                      \
        TCGv EA;                                           \
        if(unlikely(!s->altivec_enabled)) {                \
            gen_exception(s, POWERPC_EXCP_VPU);            \
            return;                                        \
        }                                                  \
        gen_set_access_type(s, ACCESS_INT);                \
        EA = tcg_temp_local_new();                         \
        gen_addr_reg_index(s, EA);                         \
        tcg_gen_andi_tl(EA, EA, ~0xf);                     \
        if(s->le_mode) {                                   \
            gen_qemu_st64(s, cpu_avrl[rD(s->opcode)], EA); \
            tcg_gen_addi_tl(EA, EA, 8);                    \
            gen_qemu_st64(s, cpu_avrh[rD(s->opcode)], EA); \
        } else {                                           \
            gen_qemu_st64(s, cpu_avrh[rD(s->opcode)], EA); \
            tcg_gen_addi_tl(EA, EA, 8);                    \
            gen_qemu_st64(s, cpu_avrl[rD(s->opcode)], EA); \
        }                                                  \
        tcg_temp_free(EA);                                 \
    }

#define GEN_VR_LVE(name, opc2, opc3)            \
    static void gen_lve##name(DisasContext *s)  \
    {                                           \
        TCGv EA;                                \
        TCGv_ptr rs;                            \
        if(unlikely(!s->altivec_enabled)) {     \
            gen_exception(s, POWERPC_EXCP_VPU); \
            return;                             \
        }                                       \
        gen_set_access_type(s, ACCESS_INT);     \
        EA = tcg_temp_new();                    \
        gen_addr_reg_index(s, EA);              \
        rs = gen_avr_ptr(rS(s->opcode));        \
        gen_helper_lve##name(rs, EA);           \
        tcg_temp_free(EA);                      \
        tcg_temp_free_ptr(rs);                  \
    }

#define GEN_VR_STVE(name, opc2, opc3)           \
    static void gen_stve##name(DisasContext *s) \
    {                                           \
        TCGv EA;                                \
        TCGv_ptr rs;                            \
        if(unlikely(!s->altivec_enabled)) {     \
            gen_exception(s, POWERPC_EXCP_VPU); \
            return;                             \
        }                                       \
        gen_set_access_type(s, ACCESS_INT);     \
        EA = tcg_temp_local_new();              \
        gen_addr_reg_index(s, EA);              \
        rs = gen_avr_ptr(rS(s->opcode));        \
        gen_helper_stve##name(rs, EA);          \
        tcg_temp_free(EA);                      \
        tcg_temp_free_ptr(rs);                  \
    }

GEN_VR_LDX(lvx, 0x07, 0x03);
/* As we don't emulate the cache, lvxl is stricly equivalent to lvx */
GEN_VR_LDX(lvxl, 0x07, 0x0B);

GEN_VR_LVE(bx, 0x07, 0x00);
GEN_VR_LVE(hx, 0x07, 0x01);
GEN_VR_LVE(wx, 0x07, 0x02);

GEN_VR_STX(svx, 0x07, 0x07);
/* As we don't emulate the cache, stvxl is stricly equivalent to stvx */
GEN_VR_STX(svxl, 0x07, 0x0F);

GEN_VR_STVE(bx, 0x07, 0x04);
GEN_VR_STVE(hx, 0x07, 0x05);
GEN_VR_STVE(wx, 0x07, 0x06);

static void gen_lvsl(DisasContext *s)
{
    TCGv_ptr rd;
    TCGv EA;
    if(unlikely(!s->altivec_enabled)) {
        gen_exception(s, POWERPC_EXCP_VPU);
        return;
    }
    EA = tcg_temp_new();
    gen_addr_reg_index(s, EA);
    rd = gen_avr_ptr(rD(s->opcode));
    gen_helper_lvsl(rd, EA);
    tcg_temp_free(EA);
    tcg_temp_free_ptr(rd);
}

static void gen_lvsr(DisasContext *s)
{
    TCGv_ptr rd;
    TCGv EA;
    if(unlikely(!s->altivec_enabled)) {
        gen_exception(s, POWERPC_EXCP_VPU);
        return;
    }
    EA = tcg_temp_new();
    gen_addr_reg_index(s, EA);
    rd = gen_avr_ptr(rD(s->opcode));
    gen_helper_lvsr(rd, EA);
    tcg_temp_free(EA);
    tcg_temp_free_ptr(rd);
}

static void gen_mfvscr(DisasContext *s)
{
    TCGv_i32 t;
    if(unlikely(!s->altivec_enabled)) {
        gen_exception(s, POWERPC_EXCP_VPU);
        return;
    }
    tcg_gen_movi_i64(cpu_avrh[rD(s->opcode)], 0);
    t = tcg_temp_new_i32();
    tcg_gen_ld_i32(t, cpu_env, offsetof(CPUState, vscr));
    tcg_gen_extu_i32_i64(cpu_avrl[rD(s->opcode)], t);
    tcg_temp_free_i32(t);
}

static void gen_mtvscr(DisasContext *s)
{
    TCGv_ptr p;
    if(unlikely(!s->altivec_enabled)) {
        gen_exception(s, POWERPC_EXCP_VPU);
        return;
    }
    p = gen_avr_ptr(rD(s->opcode));
    gen_helper_mtvscr(p);
    tcg_temp_free_ptr(p);
}

/* Logical operations */
#define GEN_VX_LOGICAL(name, tcg_op, opc2, opc3)                                           \
    static void glue(gen_, name)(DisasContext * s)                                         \
    {                                                                                      \
        if(unlikely(!s->altivec_enabled)) {                                                \
            gen_exception(s, POWERPC_EXCP_VPU);                                            \
            return;                                                                        \
        }                                                                                  \
        tcg_op(cpu_avrh[rD(s->opcode)], cpu_avrh[rA(s->opcode)], cpu_avrh[rB(s->opcode)]); \
        tcg_op(cpu_avrl[rD(s->opcode)], cpu_avrl[rA(s->opcode)], cpu_avrl[rB(s->opcode)]); \
    }

GEN_VX_LOGICAL(vand, tcg_gen_and_i64, 2, 16);
GEN_VX_LOGICAL(vandc, tcg_gen_andc_i64, 2, 17);
GEN_VX_LOGICAL(vor, tcg_gen_or_i64, 2, 18);
GEN_VX_LOGICAL(vxor, tcg_gen_xor_i64, 2, 19);
GEN_VX_LOGICAL(vnor, tcg_gen_nor_i64, 2, 20);

#define GEN_VXFORM(name, opc2, opc3)               \
    static void glue(gen_, name)(DisasContext * s) \
    {                                              \
        TCGv_ptr ra, rb, rd;                       \
        if(unlikely(!s->altivec_enabled)) {        \
            gen_exception(s, POWERPC_EXCP_VPU);    \
            return;                                \
        }                                          \
        ra = gen_avr_ptr(rA(s->opcode));           \
        rb = gen_avr_ptr(rB(s->opcode));           \
        rd = gen_avr_ptr(rD(s->opcode));           \
        gen_helper_##name(rd, ra, rb);             \
        tcg_temp_free_ptr(ra);                     \
        tcg_temp_free_ptr(rb);                     \
        tcg_temp_free_ptr(rd);                     \
    }

GEN_VXFORM(vaddubm, 0, 0);
GEN_VXFORM(vadduhm, 0, 1);
GEN_VXFORM(vadduwm, 0, 2);
GEN_VXFORM(vsububm, 0, 16);
GEN_VXFORM(vsubuhm, 0, 17);
GEN_VXFORM(vsubuwm, 0, 18);
GEN_VXFORM(vmaxub, 1, 0);
GEN_VXFORM(vmaxuh, 1, 1);
GEN_VXFORM(vmaxuw, 1, 2);
GEN_VXFORM(vmaxsb, 1, 4);
GEN_VXFORM(vmaxsh, 1, 5);
GEN_VXFORM(vmaxsw, 1, 6);
GEN_VXFORM(vminub, 1, 8);
GEN_VXFORM(vminuh, 1, 9);
GEN_VXFORM(vminuw, 1, 10);
GEN_VXFORM(vminsb, 1, 12);
GEN_VXFORM(vminsh, 1, 13);
GEN_VXFORM(vminsw, 1, 14);
GEN_VXFORM(vavgub, 1, 16);
GEN_VXFORM(vavguh, 1, 17);
GEN_VXFORM(vavguw, 1, 18);
GEN_VXFORM(vavgsb, 1, 20);
GEN_VXFORM(vavgsh, 1, 21);
GEN_VXFORM(vavgsw, 1, 22);
GEN_VXFORM(vmrghb, 6, 0);
GEN_VXFORM(vmrghh, 6, 1);
GEN_VXFORM(vmrghw, 6, 2);
GEN_VXFORM(vmrglb, 6, 4);
GEN_VXFORM(vmrglh, 6, 5);
GEN_VXFORM(vmrglw, 6, 6);
GEN_VXFORM(vmuloub, 4, 0);
GEN_VXFORM(vmulouh, 4, 1);
GEN_VXFORM(vmulosb, 4, 4);
GEN_VXFORM(vmulosh, 4, 5);
GEN_VXFORM(vmuleub, 4, 8);
GEN_VXFORM(vmuleuh, 4, 9);
GEN_VXFORM(vmulesb, 4, 12);
GEN_VXFORM(vmulesh, 4, 13);
GEN_VXFORM(vslb, 2, 4);
GEN_VXFORM(vslh, 2, 5);
GEN_VXFORM(vslw, 2, 6);
GEN_VXFORM(vsrb, 2, 8);
GEN_VXFORM(vsrh, 2, 9);
GEN_VXFORM(vsrw, 2, 10);
GEN_VXFORM(vsrab, 2, 12);
GEN_VXFORM(vsrah, 2, 13);
GEN_VXFORM(vsraw, 2, 14);
GEN_VXFORM(vslo, 6, 16);
GEN_VXFORM(vsro, 6, 17);
GEN_VXFORM(vaddcuw, 0, 6);
GEN_VXFORM(vsubcuw, 0, 22);
GEN_VXFORM(vaddubs, 0, 8);
GEN_VXFORM(vadduhs, 0, 9);
GEN_VXFORM(vadduws, 0, 10);
GEN_VXFORM(vaddsbs, 0, 12);
GEN_VXFORM(vaddshs, 0, 13);
GEN_VXFORM(vaddsws, 0, 14);
GEN_VXFORM(vsububs, 0, 24);
GEN_VXFORM(vsubuhs, 0, 25);
GEN_VXFORM(vsubuws, 0, 26);
GEN_VXFORM(vsubsbs, 0, 28);
GEN_VXFORM(vsubshs, 0, 29);
GEN_VXFORM(vsubsws, 0, 30);
GEN_VXFORM(vrlb, 2, 0);
GEN_VXFORM(vrlh, 2, 1);
GEN_VXFORM(vrlw, 2, 2);
GEN_VXFORM(vsl, 2, 7);
GEN_VXFORM(vsr, 2, 11);
GEN_VXFORM(vpkuhum, 7, 0);
GEN_VXFORM(vpkuwum, 7, 1);
GEN_VXFORM(vpkuhus, 7, 2);
GEN_VXFORM(vpkuwus, 7, 3);
GEN_VXFORM(vpkshus, 7, 4);
GEN_VXFORM(vpkswus, 7, 5);
GEN_VXFORM(vpkshss, 7, 6);
GEN_VXFORM(vpkswss, 7, 7);
GEN_VXFORM(vpkpx, 7, 12);
GEN_VXFORM(vsum4ubs, 4, 24);
GEN_VXFORM(vsum4sbs, 4, 28);
GEN_VXFORM(vsum4shs, 4, 25);
GEN_VXFORM(vsum2sws, 4, 26);
GEN_VXFORM(vsumsws, 4, 30);
GEN_VXFORM(vaddfp, 5, 0);
GEN_VXFORM(vsubfp, 5, 1);
GEN_VXFORM(vmaxfp, 5, 16);
GEN_VXFORM(vminfp, 5, 17);

#define GEN_VXRFORM1(opname, name, str, opc2, opc3) \
    static void glue(gen_, name)(DisasContext * s)  \
    {                                               \
        TCGv_ptr ra, rb, rd;                        \
        if(unlikely(!s->altivec_enabled)) {         \
            gen_exception(s, POWERPC_EXCP_VPU);     \
            return;                                 \
        }                                           \
        ra = gen_avr_ptr(rA(s->opcode));            \
        rb = gen_avr_ptr(rB(s->opcode));            \
        rd = gen_avr_ptr(rD(s->opcode));            \
        gen_helper_##opname(rd, ra, rb);            \
        tcg_temp_free_ptr(ra);                      \
        tcg_temp_free_ptr(rb);                      \
        tcg_temp_free_ptr(rd);                      \
    }

#define GEN_VXRFORM(name, opc2, opc3)           \
    GEN_VXRFORM1(name, name, #name, opc2, opc3) \
    GEN_VXRFORM1(name##_dot, name##_, #name ".", opc2, (opc3 | (0x1 << 4)))

GEN_VXRFORM(vcmpequb, 3, 0)
GEN_VXRFORM(vcmpequh, 3, 1)
GEN_VXRFORM(vcmpequw, 3, 2)
GEN_VXRFORM(vcmpgtsb, 3, 12)
GEN_VXRFORM(vcmpgtsh, 3, 13)
GEN_VXRFORM(vcmpgtsw, 3, 14)
GEN_VXRFORM(vcmpgtub, 3, 8)
GEN_VXRFORM(vcmpgtuh, 3, 9)
GEN_VXRFORM(vcmpgtuw, 3, 10)
GEN_VXRFORM(vcmpeqfp, 3, 3)
GEN_VXRFORM(vcmpgefp, 3, 7)
GEN_VXRFORM(vcmpgtfp, 3, 11)
GEN_VXRFORM(vcmpbfp, 3, 15)

#define GEN_VXFORM_SIMM(name, opc2, opc3)          \
    static void glue(gen_, name)(DisasContext * s) \
    {                                              \
        TCGv_ptr rd;                               \
        TCGv_i32 simm;                             \
        if(unlikely(!s->altivec_enabled)) {        \
            gen_exception(s, POWERPC_EXCP_VPU);    \
            return;                                \
        }                                          \
        simm = tcg_const_i32(SIMM5(s->opcode));    \
        rd = gen_avr_ptr(rD(s->opcode));           \
        gen_helper_##name(rd, simm);               \
        tcg_temp_free_i32(simm);                   \
        tcg_temp_free_ptr(rd);                     \
    }

GEN_VXFORM_SIMM(vspltisb, 6, 12);
GEN_VXFORM_SIMM(vspltish, 6, 13);
GEN_VXFORM_SIMM(vspltisw, 6, 14);

#define GEN_VXFORM_NOA(name, opc2, opc3)           \
    static void glue(gen_, name)(DisasContext * s) \
    {                                              \
        TCGv_ptr rb, rd;                           \
        if(unlikely(!s->altivec_enabled)) {        \
            gen_exception(s, POWERPC_EXCP_VPU);    \
            return;                                \
        }                                          \
        rb = gen_avr_ptr(rB(s->opcode));           \
        rd = gen_avr_ptr(rD(s->opcode));           \
        gen_helper_##name(rd, rb);                 \
        tcg_temp_free_ptr(rb);                     \
        tcg_temp_free_ptr(rd);                     \
    }

GEN_VXFORM_NOA(vupkhsb, 7, 8);
GEN_VXFORM_NOA(vupkhsh, 7, 9);
GEN_VXFORM_NOA(vupklsb, 7, 10);
GEN_VXFORM_NOA(vupklsh, 7, 11);
GEN_VXFORM_NOA(vupkhpx, 7, 13);
GEN_VXFORM_NOA(vupklpx, 7, 15);
GEN_VXFORM_NOA(vrefp, 5, 4);
GEN_VXFORM_NOA(vrsqrtefp, 5, 5);
GEN_VXFORM_NOA(vexptefp, 5, 6);
GEN_VXFORM_NOA(vlogefp, 5, 7);
GEN_VXFORM_NOA(vrfim, 5, 8);
GEN_VXFORM_NOA(vrfin, 5, 9);
GEN_VXFORM_NOA(vrfip, 5, 10);
GEN_VXFORM_NOA(vrfiz, 5, 11);

#define GEN_VXFORM_SIMM(name, opc2, opc3)          \
    static void glue(gen_, name)(DisasContext * s) \
    {                                              \
        TCGv_ptr rd;                               \
        TCGv_i32 simm;                             \
        if(unlikely(!s->altivec_enabled)) {        \
            gen_exception(s, POWERPC_EXCP_VPU);    \
            return;                                \
        }                                          \
        simm = tcg_const_i32(SIMM5(s->opcode));    \
        rd = gen_avr_ptr(rD(s->opcode));           \
        gen_helper_##name(rd, simm);               \
        tcg_temp_free_i32(simm);                   \
        tcg_temp_free_ptr(rd);                     \
    }

#define GEN_VXFORM_UIMM(name, opc2, opc3)          \
    static void glue(gen_, name)(DisasContext * s) \
    {                                              \
        TCGv_ptr rb, rd;                           \
        TCGv_i32 uimm;                             \
        if(unlikely(!s->altivec_enabled)) {        \
            gen_exception(s, POWERPC_EXCP_VPU);    \
            return;                                \
        }                                          \
        uimm = tcg_const_i32(UIMM5(s->opcode));    \
        rb = gen_avr_ptr(rB(s->opcode));           \
        rd = gen_avr_ptr(rD(s->opcode));           \
        gen_helper_##name(rd, rb, uimm);           \
        tcg_temp_free_i32(uimm);                   \
        tcg_temp_free_ptr(rb);                     \
        tcg_temp_free_ptr(rd);                     \
    }

GEN_VXFORM_UIMM(vspltb, 6, 8);
GEN_VXFORM_UIMM(vsplth, 6, 9);
GEN_VXFORM_UIMM(vspltw, 6, 10);
GEN_VXFORM_UIMM(vcfux, 5, 12);
GEN_VXFORM_UIMM(vcfsx, 5, 13);
GEN_VXFORM_UIMM(vctuxs, 5, 14);
GEN_VXFORM_UIMM(vctsxs, 5, 15);

static void gen_vsldoi(DisasContext *s)
{
    TCGv_ptr ra, rb, rd;
    TCGv_i32 sh;
    if(unlikely(!s->altivec_enabled)) {
        gen_exception(s, POWERPC_EXCP_VPU);
        return;
    }
    ra = gen_avr_ptr(rA(s->opcode));
    rb = gen_avr_ptr(rB(s->opcode));
    rd = gen_avr_ptr(rD(s->opcode));
    sh = tcg_const_i32(VSH(s->opcode));
    gen_helper_vsldoi(rd, ra, rb, sh);
    tcg_temp_free_ptr(ra);
    tcg_temp_free_ptr(rb);
    tcg_temp_free_ptr(rd);
    tcg_temp_free_i32(sh);
}

#define GEN_VAFORM_PAIRED(name0, name1, opc2)                 \
    static void glue(gen_, name0##_##name1)(DisasContext * s) \
    {                                                         \
        TCGv_ptr ra, rb, rc, rd;                              \
        if(unlikely(!s->altivec_enabled)) {                   \
            gen_exception(s, POWERPC_EXCP_VPU);               \
            return;                                           \
        }                                                     \
        ra = gen_avr_ptr(rA(s->opcode));                      \
        rb = gen_avr_ptr(rB(s->opcode));                      \
        rc = gen_avr_ptr(rC(s->opcode));                      \
        rd = gen_avr_ptr(rD(s->opcode));                      \
        if(Rc(s->opcode)) {                                   \
            gen_helper_##name1(rd, ra, rb, rc);               \
        } else {                                              \
            gen_helper_##name0(rd, ra, rb, rc);               \
        }                                                     \
        tcg_temp_free_ptr(ra);                                \
        tcg_temp_free_ptr(rb);                                \
        tcg_temp_free_ptr(rc);                                \
        tcg_temp_free_ptr(rd);                                \
    }

GEN_VAFORM_PAIRED(vmhaddshs, vmhraddshs, 16)

static void gen_vmladduhm(DisasContext *s)
{
    TCGv_ptr ra, rb, rc, rd;
    if(unlikely(!s->altivec_enabled)) {
        gen_exception(s, POWERPC_EXCP_VPU);
        return;
    }
    ra = gen_avr_ptr(rA(s->opcode));
    rb = gen_avr_ptr(rB(s->opcode));
    rc = gen_avr_ptr(rC(s->opcode));
    rd = gen_avr_ptr(rD(s->opcode));
    gen_helper_vmladduhm(rd, ra, rb, rc);
    tcg_temp_free_ptr(ra);
    tcg_temp_free_ptr(rb);
    tcg_temp_free_ptr(rc);
    tcg_temp_free_ptr(rd);
}

GEN_VAFORM_PAIRED(vmsumubm, vmsummbm, 18)
GEN_VAFORM_PAIRED(vmsumuhm, vmsumuhs, 19)
GEN_VAFORM_PAIRED(vmsumshm, vmsumshs, 20)
GEN_VAFORM_PAIRED(vsel, vperm, 21)
GEN_VAFORM_PAIRED(vmaddfp, vnmsubfp, 23)

/***                           SPE extension                               ***/
/* Register moves */

static inline void gen_evmra(DisasContext *s)
{

    if(unlikely(!s->spe_enabled)) {
        gen_exception(s, POWERPC_EXCP_SPEU);
        return;
    }

#if defined(TARGET_PPC64)
    /* rD := rA */
    tcg_gen_mov_i64(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)]);

    /* spe_acc := rA */
    tcg_gen_st_i64(cpu_gpr[rA(s->opcode)], cpu_env, offsetof(CPUState, spe_acc));
#else

    TCGv_i64 tmp = tcg_temp_local_new_i64();

    /* tmp := rA_lo + rA_hi << 32 */
    tcg_gen_concat_i32_i64(tmp, cpu_gpr[rA(s->opcode)], cpu_gprh[rA(s->opcode)]);

    /* spe_acc := tmp */
    tcg_gen_st_i64(tmp, cpu_env, offsetof(CPUState, spe_acc));
    tcg_temp_free_i64(tmp);

    /* rD := rA */
    tcg_gen_mov_i32(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)]);
    tcg_gen_mov_i32(cpu_gprh[rD(s->opcode)], cpu_gprh[rA(s->opcode)]);
#endif
}

static inline void gen_load_gpr64(TCGv_i64 t, int reg)
{
#if defined(TARGET_PPC64)
    tcg_gen_mov_i64(t, cpu_gpr[reg]);
#else
    tcg_gen_concat_i32_i64(t, cpu_gpr[reg], cpu_gprh[reg]);
#endif
}

static inline void gen_store_gpr64(int reg, TCGv_i64 t)
{
#if defined(TARGET_PPC64)
    tcg_gen_mov_i64(cpu_gpr[reg], t);
#else
    TCGv_i64 tmp = tcg_temp_new_i64();
    tcg_gen_trunc_i64_i32(cpu_gpr[reg], t);
    tcg_gen_shri_i64(tmp, t, 32);
    tcg_gen_trunc_i64_i32(cpu_gprh[reg], tmp);
    tcg_temp_free_i64(tmp);
#endif
}

#define GEN_SPE(name0, name1, opc2, opc3, inval0, inval1, type) \
    static void glue(gen_, name0##_##name1)(DisasContext * s)   \
    {                                                           \
        if(Rc(s->opcode))                                       \
            gen_##name1(s);                                     \
        else                                                    \
            gen_##name0(s);                                     \
    }

/* Handler for undefined SPE opcodes */
static inline void gen_speundef(DisasContext *s)
{
    gen_inval_exception(s, POWERPC_EXCP_INVAL_INVAL);
}

/* SPE logic */
#if defined(TARGET_PPC64)
#define GEN_SPEOP_LOGIC2(name, tcg_op)                                                  \
    static inline void gen_##name(DisasContext *s)                                      \
    {                                                                                   \
        if(unlikely(!s->spe_enabled)) {                                                 \
            gen_exception(s, POWERPC_EXCP_SPEU);                                        \
            return;                                                                     \
        }                                                                               \
        tcg_op(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)]); \
    }
#else
#define GEN_SPEOP_LOGIC2(name, tcg_op)                                                     \
    static inline void gen_##name(DisasContext *s)                                         \
    {                                                                                      \
        if(unlikely(!s->spe_enabled)) {                                                    \
            gen_exception(s, POWERPC_EXCP_SPEU);                                           \
            return;                                                                        \
        }                                                                                  \
        tcg_op(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)]);    \
        tcg_op(cpu_gprh[rD(s->opcode)], cpu_gprh[rA(s->opcode)], cpu_gprh[rB(s->opcode)]); \
    }
#endif

GEN_SPEOP_LOGIC2(evand, tcg_gen_and_tl);
GEN_SPEOP_LOGIC2(evandc, tcg_gen_andc_tl);
GEN_SPEOP_LOGIC2(evxor, tcg_gen_xor_tl);
GEN_SPEOP_LOGIC2(evor, tcg_gen_or_tl);
GEN_SPEOP_LOGIC2(evnor, tcg_gen_nor_tl);
GEN_SPEOP_LOGIC2(eveqv, tcg_gen_eqv_tl);
GEN_SPEOP_LOGIC2(evorc, tcg_gen_orc_tl);
GEN_SPEOP_LOGIC2(evnand, tcg_gen_nand_tl);

/* SPE logic immediate */
#if defined(TARGET_PPC64)
#define GEN_SPEOP_TCG_LOGIC_IMM2(name, tcg_opi)                 \
    static inline void gen_##name(DisasContext *s)              \
    {                                                           \
        if(unlikely(!s->spe_enabled)) {                         \
            gen_exception(s, POWERPC_EXCP_SPEU);                \
            return;                                             \
        }                                                       \
        TCGv_i32 t0 = tcg_temp_local_new_i32();                 \
        TCGv_i32 t1 = tcg_temp_local_new_i32();                 \
        TCGv_i64 t2 = tcg_temp_local_new_i64();                 \
        tcg_gen_trunc_i64_i32(t0, cpu_gpr[rA(s->opcode)]);      \
        tcg_opi(t0, t0, rB(s->opcode));                         \
        tcg_gen_shri_i64(t2, cpu_gpr[rA(s->opcode)], 32);       \
        tcg_gen_trunc_i64_i32(t1, t2);                          \
        tcg_temp_free_i64(t2);                                  \
        tcg_opi(t1, t1, rB(s->opcode));                         \
        tcg_gen_concat_i32_i64(cpu_gpr[rD(s->opcode)], t0, t1); \
        tcg_temp_free_i32(t0);                                  \
        tcg_temp_free_i32(t1);                                  \
    }
#else
#define GEN_SPEOP_TCG_LOGIC_IMM2(name, tcg_opi)                                   \
    static inline void gen_##name(DisasContext *s)                                \
    {                                                                             \
        if(unlikely(!s->spe_enabled)) {                                           \
            gen_exception(s, POWERPC_EXCP_SPEU);                                  \
            return;                                                               \
        }                                                                         \
        tcg_opi(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], rB(s->opcode));   \
        tcg_opi(cpu_gprh[rD(s->opcode)], cpu_gprh[rA(s->opcode)], rB(s->opcode)); \
    }
#endif
GEN_SPEOP_TCG_LOGIC_IMM2(evslwi, tcg_gen_shli_i32);
GEN_SPEOP_TCG_LOGIC_IMM2(evsrwiu, tcg_gen_shri_i32);
GEN_SPEOP_TCG_LOGIC_IMM2(evsrwis, tcg_gen_sari_i32);
GEN_SPEOP_TCG_LOGIC_IMM2(evrlwi, tcg_gen_rotli_i32);

/* SPE arithmetic */
#if defined(TARGET_PPC64)
#define GEN_SPEOP_ARITH1(name, tcg_op)                          \
    static inline void gen_##name(DisasContext *s)              \
    {                                                           \
        if(unlikely(!s->spe_enabled)) {                         \
            gen_exception(s, POWERPC_EXCP_SPEU);                \
            return;                                             \
        }                                                       \
        TCGv_i32 t0 = tcg_temp_local_new_i32();                 \
        TCGv_i32 t1 = tcg_temp_local_new_i32();                 \
        TCGv_i64 t2 = tcg_temp_local_new_i64();                 \
        tcg_gen_trunc_i64_i32(t0, cpu_gpr[rA(s->opcode)]);      \
        tcg_op(t0, t0);                                         \
        tcg_gen_shri_i64(t2, cpu_gpr[rA(s->opcode)], 32);       \
        tcg_gen_trunc_i64_i32(t1, t2);                          \
        tcg_temp_free_i64(t2);                                  \
        tcg_op(t1, t1);                                         \
        tcg_gen_concat_i32_i64(cpu_gpr[rD(s->opcode)], t0, t1); \
        tcg_temp_free_i32(t0);                                  \
        tcg_temp_free_i32(t1);                                  \
    }
#else
#define GEN_SPEOP_ARITH1(name, tcg_op)                            \
    static inline void gen_##name(DisasContext *s)                \
    {                                                             \
        if(unlikely(!s->spe_enabled)) {                           \
            gen_exception(s, POWERPC_EXCP_SPEU);                  \
            return;                                               \
        }                                                         \
        tcg_op(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)]);   \
        tcg_op(cpu_gprh[rD(s->opcode)], cpu_gprh[rA(s->opcode)]); \
    }
#endif

static inline void gen_op_evabs(TCGv_i32 ret, TCGv_i32 arg1)
{
    int l1 = gen_new_label();
    int l2 = gen_new_label();

    tcg_gen_brcondi_i32(TCG_COND_GE, arg1, 0, l1);
    tcg_gen_neg_i32(ret, arg1);
    tcg_gen_br(l2);
    gen_set_label(l1);
    tcg_gen_mov_i32(ret, arg1);
    gen_set_label(l2);
}
GEN_SPEOP_ARITH1(evabs, gen_op_evabs);
GEN_SPEOP_ARITH1(evneg, tcg_gen_neg_i32);
GEN_SPEOP_ARITH1(evextsb, tcg_gen_ext8s_i32);
GEN_SPEOP_ARITH1(evextsh, tcg_gen_ext16s_i32);
static inline void gen_op_evrndw(TCGv_i32 ret, TCGv_i32 arg1)
{
    tcg_gen_addi_i32(ret, arg1, 0x8000);
    tcg_gen_ext16u_i32(ret, ret);
}
GEN_SPEOP_ARITH1(evrndw, gen_op_evrndw);
GEN_SPEOP_ARITH1(evcntlsw, gen_helper_cntlsw32);
GEN_SPEOP_ARITH1(evcntlzw, gen_helper_cntlzw32);

#if defined(TARGET_PPC64)
#define GEN_SPEOP_ARITH2(name, tcg_op)                          \
    static inline void gen_##name(DisasContext *s)              \
    {                                                           \
        if(unlikely(!s->spe_enabled)) {                         \
            gen_exception(s, POWERPC_EXCP_SPEU);                \
            return;                                             \
        }                                                       \
        TCGv_i32 t0 = tcg_temp_local_new_i32();                 \
        TCGv_i32 t1 = tcg_temp_local_new_i32();                 \
        TCGv_i32 t2 = tcg_temp_local_new_i32();                 \
        TCGv_i64 t3 = tcg_temp_local_new_i64();                 \
        tcg_gen_trunc_i64_i32(t0, cpu_gpr[rA(s->opcode)]);      \
        tcg_gen_trunc_i64_i32(t2, cpu_gpr[rB(s->opcode)]);      \
        tcg_op(t0, t0, t2);                                     \
        tcg_gen_shri_i64(t3, cpu_gpr[rA(s->opcode)], 32);       \
        tcg_gen_trunc_i64_i32(t1, t3);                          \
        tcg_gen_shri_i64(t3, cpu_gpr[rB(s->opcode)], 32);       \
        tcg_gen_trunc_i64_i32(t2, t3);                          \
        tcg_temp_free_i64(t3);                                  \
        tcg_op(t1, t1, t2);                                     \
        tcg_temp_free_i32(t2);                                  \
        tcg_gen_concat_i32_i64(cpu_gpr[rD(s->opcode)], t0, t1); \
        tcg_temp_free_i32(t0);                                  \
        tcg_temp_free_i32(t1);                                  \
    }
#else
#define GEN_SPEOP_ARITH2(name, tcg_op)                                                     \
    static inline void gen_##name(DisasContext *s)                                         \
    {                                                                                      \
        if(unlikely(!s->spe_enabled)) {                                                    \
            gen_exception(s, POWERPC_EXCP_SPEU);                                           \
            return;                                                                        \
        }                                                                                  \
        tcg_op(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)]);    \
        tcg_op(cpu_gprh[rD(s->opcode)], cpu_gprh[rA(s->opcode)], cpu_gprh[rB(s->opcode)]); \
    }
#endif

static inline void gen_op_evsrwu(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    TCGv_i32 t0;
    int l1, l2;

    l1 = gen_new_label();
    l2 = gen_new_label();
    t0 = tcg_temp_local_new_i32();
    /* No error here: 6 bits are used */
    tcg_gen_andi_i32(t0, arg2, 0x3F);
    tcg_gen_brcondi_i32(TCG_COND_GE, t0, 32, l1);
    tcg_gen_shr_i32(ret, arg1, t0);
    tcg_gen_br(l2);
    gen_set_label(l1);
    tcg_gen_movi_i32(ret, 0);
    gen_set_label(l2);
    tcg_temp_free_i32(t0);
}
GEN_SPEOP_ARITH2(evsrwu, gen_op_evsrwu);
static inline void gen_op_evsrws(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    TCGv_i32 t0;
    int l1, l2;

    l1 = gen_new_label();
    l2 = gen_new_label();
    t0 = tcg_temp_local_new_i32();
    /* No error here: 6 bits are used */
    tcg_gen_andi_i32(t0, arg2, 0x3F);
    tcg_gen_brcondi_i32(TCG_COND_GE, t0, 32, l1);
    tcg_gen_sar_i32(ret, arg1, t0);
    tcg_gen_br(l2);
    gen_set_label(l1);
    tcg_gen_movi_i32(ret, 0);
    gen_set_label(l2);
    tcg_temp_free_i32(t0);
}
GEN_SPEOP_ARITH2(evsrws, gen_op_evsrws);
static inline void gen_op_evslw(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    TCGv_i32 t0;
    int l1, l2;

    l1 = gen_new_label();
    l2 = gen_new_label();
    t0 = tcg_temp_local_new_i32();
    /* No error here: 6 bits are used */
    tcg_gen_andi_i32(t0, arg2, 0x3F);
    tcg_gen_brcondi_i32(TCG_COND_GE, t0, 32, l1);
    tcg_gen_shl_i32(ret, arg1, t0);
    tcg_gen_br(l2);
    gen_set_label(l1);
    tcg_gen_movi_i32(ret, 0);
    gen_set_label(l2);
    tcg_temp_free_i32(t0);
}
GEN_SPEOP_ARITH2(evslw, gen_op_evslw);
static inline void gen_op_evrlw(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    TCGv_i32 t0 = tcg_temp_new_i32();
    tcg_gen_andi_i32(t0, arg2, 0x1F);
    tcg_gen_rotl_i32(ret, arg1, t0);
    tcg_temp_free_i32(t0);
}
GEN_SPEOP_ARITH2(evrlw, gen_op_evrlw);
static inline void gen_evmergehi(DisasContext *s)
{
    if(unlikely(!s->spe_enabled)) {
        gen_exception(s, POWERPC_EXCP_SPEU);
        return;
    }
#if defined(TARGET_PPC64)
    TCGv t0 = tcg_temp_new();
    TCGv t1 = tcg_temp_new();
    tcg_gen_shri_tl(t0, cpu_gpr[rB(s->opcode)], 32);
    tcg_gen_andi_tl(t1, cpu_gpr[rA(s->opcode)], 0xFFFFFFFF0000000ULL);
    tcg_gen_or_tl(cpu_gpr[rD(s->opcode)], t0, t1);
    tcg_temp_free(t0);
    tcg_temp_free(t1);
#else
    tcg_gen_mov_i32(cpu_gpr[rD(s->opcode)], cpu_gprh[rB(s->opcode)]);
    tcg_gen_mov_i32(cpu_gprh[rD(s->opcode)], cpu_gprh[rA(s->opcode)]);
#endif
}
GEN_SPEOP_ARITH2(evaddw, tcg_gen_add_i32);
static inline void gen_op_evsubf(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    tcg_gen_sub_i32(ret, arg2, arg1);
}
GEN_SPEOP_ARITH2(evsubfw, gen_op_evsubf);

/* SPE arithmetic immediate */
#if defined(TARGET_PPC64)
#define GEN_SPEOP_ARITH_IMM2(name, tcg_op)                      \
    static inline void gen_##name(DisasContext *s)              \
    {                                                           \
        if(unlikely(!s->spe_enabled)) {                         \
            gen_exception(s, POWERPC_EXCP_SPEU);                \
            return;                                             \
        }                                                       \
        TCGv_i32 t0 = tcg_temp_local_new_i32();                 \
        TCGv_i32 t1 = tcg_temp_local_new_i32();                 \
        TCGv_i64 t2 = tcg_temp_local_new_i64();                 \
        tcg_gen_trunc_i64_i32(t0, cpu_gpr[rB(s->opcode)]);      \
        tcg_op(t0, t0, rA(s->opcode));                          \
        tcg_gen_shri_i64(t2, cpu_gpr[rB(s->opcode)], 32);       \
        tcg_gen_trunc_i64_i32(t1, t2);                          \
        tcg_temp_free_i64(t2);                                  \
        tcg_op(t1, t1, rA(s->opcode));                          \
        tcg_gen_concat_i32_i64(cpu_gpr[rD(s->opcode)], t0, t1); \
        tcg_temp_free_i32(t0);                                  \
        tcg_temp_free_i32(t1);                                  \
    }
#else
#define GEN_SPEOP_ARITH_IMM2(name, tcg_op)                                       \
    static inline void gen_##name(DisasContext *s)                               \
    {                                                                            \
        if(unlikely(!s->spe_enabled)) {                                          \
            gen_exception(s, POWERPC_EXCP_SPEU);                                 \
            return;                                                              \
        }                                                                        \
        tcg_op(cpu_gpr[rD(s->opcode)], cpu_gpr[rB(s->opcode)], rA(s->opcode));   \
        tcg_op(cpu_gprh[rD(s->opcode)], cpu_gprh[rB(s->opcode)], rA(s->opcode)); \
    }
#endif
GEN_SPEOP_ARITH_IMM2(evaddiw, tcg_gen_addi_i32);
GEN_SPEOP_ARITH_IMM2(evsubifw, tcg_gen_subi_i32);

/* SPE comparison */
#if defined(TARGET_PPC64)
#define GEN_SPEOP_COMP(name, tcg_cond)                                                                   \
    static inline void gen_##name(DisasContext *s)                                                       \
    {                                                                                                    \
        if(unlikely(!s->spe_enabled)) {                                                                  \
            gen_exception(s, POWERPC_EXCP_SPEU);                                                         \
            return;                                                                                      \
        }                                                                                                \
        int l1 = gen_new_label();                                                                        \
        int l2 = gen_new_label();                                                                        \
        int l3 = gen_new_label();                                                                        \
        int l4 = gen_new_label();                                                                        \
        TCGv_i32 t0 = tcg_temp_local_new_i32();                                                          \
        TCGv_i32 t1 = tcg_temp_local_new_i32();                                                          \
        TCGv_i64 t2 = tcg_temp_local_new_i64();                                                          \
        tcg_gen_trunc_i64_i32(t0, cpu_gpr[rA(s->opcode)]);                                               \
        tcg_gen_trunc_i64_i32(t1, cpu_gpr[rB(s->opcode)]);                                               \
        tcg_gen_brcond_i32(tcg_cond, t0, t1, l1);                                                        \
        tcg_gen_movi_i32(cpu_crf[crfD(s->opcode)], 0);                                                   \
        tcg_gen_br(l2);                                                                                  \
        gen_set_label(l1);                                                                               \
        tcg_gen_movi_i32(cpu_crf[crfD(s->opcode)], CRF_CL | CRF_CH_OR_CL | CRF_CH_AND_CL);               \
        gen_set_label(l2);                                                                               \
        tcg_gen_shri_i64(t2, cpu_gpr[rA(s->opcode)], 32);                                                \
        tcg_gen_trunc_i64_i32(t0, t2);                                                                   \
        tcg_gen_shri_i64(t2, cpu_gpr[rB(s->opcode)], 32);                                                \
        tcg_gen_trunc_i64_i32(t1, t2);                                                                   \
        tcg_temp_free_i64(t2);                                                                           \
        tcg_gen_brcond_i32(tcg_cond, t0, t1, l3);                                                        \
        tcg_gen_andi_i32(cpu_crf[crfD(s->opcode)], cpu_crf[crfD(s->opcode)], ~(CRF_CH | CRF_CH_AND_CL)); \
        tcg_gen_br(l4);                                                                                  \
        gen_set_label(l3);                                                                               \
        tcg_gen_ori_i32(cpu_crf[crfD(s->opcode)], cpu_crf[crfD(s->opcode)], CRF_CH | CRF_CH_OR_CL);      \
        gen_set_label(l4);                                                                               \
        tcg_temp_free_i32(t0);                                                                           \
        tcg_temp_free_i32(t1);                                                                           \
    }
#else
#define GEN_SPEOP_COMP(name, tcg_cond)                                                                   \
    static inline void gen_##name(DisasContext *s)                                                       \
    {                                                                                                    \
        if(unlikely(!s->spe_enabled)) {                                                                  \
            gen_exception(s, POWERPC_EXCP_SPEU);                                                         \
            return;                                                                                      \
        }                                                                                                \
        int l1 = gen_new_label();                                                                        \
        int l2 = gen_new_label();                                                                        \
        int l3 = gen_new_label();                                                                        \
        int l4 = gen_new_label();                                                                        \
                                                                                                         \
        tcg_gen_brcond_i32(tcg_cond, cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)], l1);                \
        tcg_gen_movi_tl(cpu_crf[crfD(s->opcode)], 0);                                                    \
        tcg_gen_br(l2);                                                                                  \
        gen_set_label(l1);                                                                               \
        tcg_gen_movi_i32(cpu_crf[crfD(s->opcode)], CRF_CL | CRF_CH_OR_CL | CRF_CH_AND_CL);               \
        gen_set_label(l2);                                                                               \
        tcg_gen_brcond_i32(tcg_cond, cpu_gprh[rA(s->opcode)], cpu_gprh[rB(s->opcode)], l3);              \
        tcg_gen_andi_i32(cpu_crf[crfD(s->opcode)], cpu_crf[crfD(s->opcode)], ~(CRF_CH | CRF_CH_AND_CL)); \
        tcg_gen_br(l4);                                                                                  \
        gen_set_label(l3);                                                                               \
        tcg_gen_ori_i32(cpu_crf[crfD(s->opcode)], cpu_crf[crfD(s->opcode)], CRF_CH | CRF_CH_OR_CL);      \
        gen_set_label(l4);                                                                               \
    }
#endif
GEN_SPEOP_COMP(evcmpgtu, TCG_COND_GTU);
GEN_SPEOP_COMP(evcmpgts, TCG_COND_GT);
GEN_SPEOP_COMP(evcmpltu, TCG_COND_LTU);
GEN_SPEOP_COMP(evcmplts, TCG_COND_LT);
GEN_SPEOP_COMP(evcmpeq, TCG_COND_EQ);

/* SPE misc */
static inline void gen_brinc(DisasContext *s)
{
    /* Note: brinc is usable even if SPE is disabled */
    gen_helper_brinc(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)]);
}
static inline void gen_evmergelo(DisasContext *s)
{
    if(unlikely(!s->spe_enabled)) {
        gen_exception(s, POWERPC_EXCP_SPEU);
        return;
    }
#if defined(TARGET_PPC64)
    TCGv t0 = tcg_temp_new();
    TCGv t1 = tcg_temp_new();
    tcg_gen_ext32u_tl(t0, cpu_gpr[rB(s->opcode)]);
    tcg_gen_shli_tl(t1, cpu_gpr[rA(s->opcode)], 32);
    tcg_gen_or_tl(cpu_gpr[rD(s->opcode)], t0, t1);
    tcg_temp_free(t0);
    tcg_temp_free(t1);
#else
    tcg_gen_mov_i32(cpu_gprh[rD(s->opcode)], cpu_gpr[rA(s->opcode)]);
    tcg_gen_mov_i32(cpu_gpr[rD(s->opcode)], cpu_gpr[rB(s->opcode)]);
#endif
}
static inline void gen_evmergehilo(DisasContext *s)
{
    if(unlikely(!s->spe_enabled)) {
        gen_exception(s, POWERPC_EXCP_SPEU);
        return;
    }
#if defined(TARGET_PPC64)
    TCGv t0 = tcg_temp_new();
    TCGv t1 = tcg_temp_new();
    tcg_gen_ext32u_tl(t0, cpu_gpr[rB(s->opcode)]);
    tcg_gen_andi_tl(t1, cpu_gpr[rA(s->opcode)], 0xFFFFFFFF0000000ULL);
    tcg_gen_or_tl(cpu_gpr[rD(s->opcode)], t0, t1);
    tcg_temp_free(t0);
    tcg_temp_free(t1);
#else
    tcg_gen_mov_i32(cpu_gpr[rD(s->opcode)], cpu_gpr[rB(s->opcode)]);
    tcg_gen_mov_i32(cpu_gprh[rD(s->opcode)], cpu_gprh[rA(s->opcode)]);
#endif
}
static inline void gen_evmergelohi(DisasContext *s)
{
    if(unlikely(!s->spe_enabled)) {
        gen_exception(s, POWERPC_EXCP_SPEU);
        return;
    }
#if defined(TARGET_PPC64)
    TCGv t0 = tcg_temp_new();
    TCGv t1 = tcg_temp_new();
    tcg_gen_shri_tl(t0, cpu_gpr[rB(s->opcode)], 32);
    tcg_gen_shli_tl(t1, cpu_gpr[rA(s->opcode)], 32);
    tcg_gen_or_tl(cpu_gpr[rD(s->opcode)], t0, t1);
    tcg_temp_free(t0);
    tcg_temp_free(t1);
#else
    if(rD(s->opcode) == rA(s->opcode)) {
        TCGv_i32 tmp = tcg_temp_new_i32();
        tcg_gen_mov_i32(tmp, cpu_gpr[rA(s->opcode)]);
        tcg_gen_mov_i32(cpu_gpr[rD(s->opcode)], cpu_gprh[rB(s->opcode)]);
        tcg_gen_mov_i32(cpu_gprh[rD(s->opcode)], tmp);
        tcg_temp_free_i32(tmp);
    } else {
        tcg_gen_mov_i32(cpu_gpr[rD(s->opcode)], cpu_gprh[rB(s->opcode)]);
        tcg_gen_mov_i32(cpu_gprh[rD(s->opcode)], cpu_gpr[rA(s->opcode)]);
    }
#endif
}
static inline void gen_evsplati(DisasContext *s)
{
    uint64_t imm = ((int32_t)(rA(s->opcode) << 27)) >> 27;

#if defined(TARGET_PPC64)
    tcg_gen_movi_tl(cpu_gpr[rD(s->opcode)], (imm << 32) | imm);
#else
    tcg_gen_movi_i32(cpu_gpr[rD(s->opcode)], imm);
    tcg_gen_movi_i32(cpu_gprh[rD(s->opcode)], imm);
#endif
}
static inline void gen_evsplatfi(DisasContext *s)
{
    uint64_t imm = rA(s->opcode) << 27;

#if defined(TARGET_PPC64)
    tcg_gen_movi_tl(cpu_gpr[rD(s->opcode)], (imm << 32) | imm);
#else
    tcg_gen_movi_i32(cpu_gpr[rD(s->opcode)], imm);
    tcg_gen_movi_i32(cpu_gprh[rD(s->opcode)], imm);
#endif
}

static inline void gen_evsel(DisasContext *s)
{
    int l1 = gen_new_label();
    int l2 = gen_new_label();
    int l3 = gen_new_label();
    int l4 = gen_new_label();
    TCGv_i32 t0 = tcg_temp_local_new_i32();
#if defined(TARGET_PPC64)
    TCGv t1 = tcg_temp_local_new();
    TCGv t2 = tcg_temp_local_new();
#endif
    tcg_gen_andi_i32(t0, cpu_crf[s->opcode & 0x07], 1 << 3);
    tcg_gen_brcondi_i32(TCG_COND_EQ, t0, 0, l1);
#if defined(TARGET_PPC64)
    tcg_gen_andi_tl(t1, cpu_gpr[rA(s->opcode)], 0xFFFFFFFF00000000ULL);
#else
    tcg_gen_mov_tl(cpu_gprh[rD(s->opcode)], cpu_gprh[rA(s->opcode)]);
#endif
    tcg_gen_br(l2);
    gen_set_label(l1);
#if defined(TARGET_PPC64)
    tcg_gen_andi_tl(t1, cpu_gpr[rB(s->opcode)], 0xFFFFFFFF00000000ULL);
#else
    tcg_gen_mov_tl(cpu_gprh[rD(s->opcode)], cpu_gprh[rB(s->opcode)]);
#endif
    gen_set_label(l2);
    tcg_gen_andi_i32(t0, cpu_crf[s->opcode & 0x07], 1 << 2);
    tcg_gen_brcondi_i32(TCG_COND_EQ, t0, 0, l3);
#if defined(TARGET_PPC64)
    tcg_gen_ext32u_tl(t2, cpu_gpr[rA(s->opcode)]);
#else
    tcg_gen_mov_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)]);
#endif
    tcg_gen_br(l4);
    gen_set_label(l3);
#if defined(TARGET_PPC64)
    tcg_gen_ext32u_tl(t2, cpu_gpr[rB(s->opcode)]);
#else
    tcg_gen_mov_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rB(s->opcode)]);
#endif
    gen_set_label(l4);
    tcg_temp_free_i32(t0);
#if defined(TARGET_PPC64)
    tcg_gen_or_tl(cpu_gpr[rD(s->opcode)], t1, t2);
    tcg_temp_free(t1);
    tcg_temp_free(t2);
#endif
}

static void gen_evsel0(DisasContext *s)
{
    gen_evsel(s);
}

static void gen_evsel1(DisasContext *s)
{
    gen_evsel(s);
}

static void gen_evsel2(DisasContext *s)
{
    gen_evsel(s);
}

static void gen_evsel3(DisasContext *s)
{
    gen_evsel(s);
}

/* Multiply */

static inline void gen_evmwumi(DisasContext *s)
{
    TCGv_i64 t0, t1;

    if(unlikely(!s->spe_enabled)) {
        gen_exception(s, POWERPC_EXCP_SPEU);
        return;
    }

    t0 = tcg_temp_new_i64();
    t1 = tcg_temp_new_i64();

    /* t0 := rA; t1 := rB */
#if defined(TARGET_PPC64)
    tcg_gen_ext32u_tl(t0, cpu_gpr[rA(s->opcode)]);
    tcg_gen_ext32u_tl(t1, cpu_gpr[rB(s->opcode)]);
#else
    tcg_gen_extu_tl_i64(t0, cpu_gpr[rA(s->opcode)]);
    tcg_gen_extu_tl_i64(t1, cpu_gpr[rB(s->opcode)]);
#endif

    tcg_gen_mul_i64(t0, t0, t1); /* t0 := rA * rB */

    gen_store_gpr64(rD(s->opcode), t0); /* rD := t0 */

    tcg_temp_free_i64(t0);
    tcg_temp_free_i64(t1);
}

static inline void gen_evmwumia(DisasContext *s)
{
    TCGv_i64 tmp;

    if(unlikely(!s->spe_enabled)) {
        gen_exception(s, POWERPC_EXCP_SPEU);
        return;
    }

    gen_evmwumi(s); /* rD := rA * rB */

    tmp = tcg_temp_local_new_i64();

    /* acc := rD */
    gen_load_gpr64(tmp, rD(s->opcode));
    tcg_gen_st_i64(tmp, cpu_env, offsetof(CPUState, spe_acc));
    tcg_temp_free_i64(tmp);
}

static inline void gen_evmwumiaa(DisasContext *s)
{
    TCGv_i64 acc;
    TCGv_i64 tmp;

    if(unlikely(!s->spe_enabled)) {
        gen_exception(s, POWERPC_EXCP_SPEU);
        return;
    }

    gen_evmwumi(s); /* rD := rA * rB */

    acc = tcg_temp_new_i64();
    tmp = tcg_temp_new_i64();

    /* tmp := rD */
    gen_load_gpr64(tmp, rD(s->opcode));

    /* Load acc */
    tcg_gen_ld_i64(acc, cpu_env, offsetof(CPUState, spe_acc));

    /* acc := tmp + acc */
    tcg_gen_add_i64(acc, acc, tmp);

    /* Store acc */
    tcg_gen_st_i64(acc, cpu_env, offsetof(CPUState, spe_acc));

    /* rD := acc */
    gen_store_gpr64(rD(s->opcode), acc);

    tcg_temp_free_i64(acc);
    tcg_temp_free_i64(tmp);
}

static inline void gen_evmwsmi(DisasContext *s)
{
    TCGv_i64 t0, t1;

    if(unlikely(!s->spe_enabled)) {
        gen_exception(s, POWERPC_EXCP_SPEU);
        return;
    }

    t0 = tcg_temp_new_i64();
    t1 = tcg_temp_new_i64();

    /* t0 := rA; t1 := rB */
#if defined(TARGET_PPC64)
    tcg_gen_ext32s_tl(t0, cpu_gpr[rA(s->opcode)]);
    tcg_gen_ext32s_tl(t1, cpu_gpr[rB(s->opcode)]);
#else
    tcg_gen_ext_tl_i64(t0, cpu_gpr[rA(s->opcode)]);
    tcg_gen_ext_tl_i64(t1, cpu_gpr[rB(s->opcode)]);
#endif

    tcg_gen_mul_i64(t0, t0, t1); /* t0 := rA * rB */

    gen_store_gpr64(rD(s->opcode), t0); /* rD := t0 */

    tcg_temp_free_i64(t0);
    tcg_temp_free_i64(t1);
}

static inline void gen_evmwsmia(DisasContext *s)
{
    TCGv_i64 tmp;

    gen_evmwsmi(s); /* rD := rA * rB */

    tmp = tcg_temp_new_i64();

    /* acc := rD */
    gen_load_gpr64(tmp, rD(s->opcode));
    tcg_gen_st_i64(tmp, cpu_env, offsetof(CPUState, spe_acc));

    tcg_temp_free_i64(tmp);
}

static inline void gen_evmwsmiaa(DisasContext *s)
{
    TCGv_i64 acc = tcg_temp_new_i64();
    TCGv_i64 tmp = tcg_temp_new_i64();

    gen_evmwsmi(s); /* rD := rA * rB */

    acc = tcg_temp_new_i64();
    tmp = tcg_temp_new_i64();

    /* tmp := rD */
    gen_load_gpr64(tmp, rD(s->opcode));

    /* Load acc */
    tcg_gen_ld_i64(acc, cpu_env, offsetof(CPUState, spe_acc));

    /* acc := tmp + acc */
    tcg_gen_add_i64(acc, acc, tmp);

    /* Store acc */
    tcg_gen_st_i64(acc, cpu_env, offsetof(CPUState, spe_acc));

    /* rD := acc */
    gen_store_gpr64(rD(s->opcode), acc);

    tcg_temp_free_i64(acc);
    tcg_temp_free_i64(tmp);
}

GEN_SPE(evaddw, speundef, 0x00, 0x08, 0x00000000, 0xFFFFFFFF, PPC_SPE);
GEN_SPE(evaddiw, speundef, 0x01, 0x08, 0x00000000, 0xFFFFFFFF, PPC_SPE);
GEN_SPE(evsubfw, speundef, 0x02, 0x08, 0x00000000, 0xFFFFFFFF, PPC_SPE);
GEN_SPE(evsubifw, speundef, 0x03, 0x08, 0x00000000, 0xFFFFFFFF, PPC_SPE);
GEN_SPE(evabs, evneg, 0x04, 0x08, 0x0000F800, 0x0000F800, PPC_SPE);
GEN_SPE(evextsb, evextsh, 0x05, 0x08, 0x0000F800, 0x0000F800, PPC_SPE);
GEN_SPE(evrndw, evcntlzw, 0x06, 0x08, 0x0000F800, 0x0000F800, PPC_SPE);
GEN_SPE(evcntlsw, brinc, 0x07, 0x08, 0x0000F800, 0x00000000, PPC_SPE);
GEN_SPE(evmra, speundef, 0x02, 0x13, 0x0000F800, 0xFFFFFFFF, PPC_SPE);
GEN_SPE(speundef, evand, 0x08, 0x08, 0xFFFFFFFF, 0x00000000, PPC_SPE);
GEN_SPE(evandc, speundef, 0x09, 0x08, 0x00000000, 0xFFFFFFFF, PPC_SPE);
GEN_SPE(evxor, evor, 0x0B, 0x08, 0x00000000, 0x00000000, PPC_SPE);
GEN_SPE(evnor, eveqv, 0x0C, 0x08, 0x00000000, 0x00000000, PPC_SPE);
GEN_SPE(evmwumi, evmwsmi, 0x0C, 0x11, 0x00000000, 0x00000000, PPC_SPE);
GEN_SPE(evmwumia, evmwsmia, 0x1C, 0x11, 0x00000000, 0x00000000, PPC_SPE);
GEN_SPE(evmwumiaa, evmwsmiaa, 0x0C, 0x15, 0x00000000, 0x00000000, PPC_SPE);
GEN_SPE(speundef, evorc, 0x0D, 0x08, 0xFFFFFFFF, 0x00000000, PPC_SPE);
GEN_SPE(evnand, speundef, 0x0F, 0x08, 0x00000000, 0xFFFFFFFF, PPC_SPE);
GEN_SPE(evsrwu, evsrws, 0x10, 0x08, 0x00000000, 0x00000000, PPC_SPE);
GEN_SPE(evsrwiu, evsrwis, 0x11, 0x08, 0x00000000, 0x00000000, PPC_SPE);
GEN_SPE(evslw, speundef, 0x12, 0x08, 0x00000000, 0xFFFFFFFF, PPC_SPE);
GEN_SPE(evslwi, speundef, 0x13, 0x08, 0x00000000, 0xFFFFFFFF, PPC_SPE);
GEN_SPE(evrlw, evsplati, 0x14, 0x08, 0x00000000, 0x0000F800, PPC_SPE);
GEN_SPE(evrlwi, evsplatfi, 0x15, 0x08, 0x00000000, 0x0000F800, PPC_SPE);
GEN_SPE(evmergehi, evmergelo, 0x16, 0x08, 0x00000000, 0x00000000, PPC_SPE);
GEN_SPE(evmergehilo, evmergelohi, 0x17, 0x08, 0x00000000, 0x00000000, PPC_SPE);
GEN_SPE(evcmpgtu, evcmpgts, 0x18, 0x08, 0x00600000, 0x00600000, PPC_SPE);
GEN_SPE(evcmpltu, evcmplts, 0x19, 0x08, 0x00600000, 0x00600000, PPC_SPE);
GEN_SPE(evcmpeq, speundef, 0x1A, 0x08, 0x00600000, 0xFFFFFFFF, PPC_SPE);

/* SPE load and stores */
static inline void gen_addr_spe_imm_index(DisasContext *s, TCGv EA, int sh)
{
    target_ulong uimm = rB(s->opcode);

    if(rA(s->opcode) == 0) {
        tcg_gen_movi_tl(EA, uimm << sh);
    } else {
        tcg_gen_addi_tl(EA, cpu_gpr[rA(s->opcode)], uimm << sh);
#if defined(TARGET_PPC64)
        if(!s->sf_mode) {
            tcg_gen_ext32u_tl(EA, EA);
        }
#endif
    }
}

static inline void gen_op_evldd(DisasContext *s, TCGv addr)
{
#if defined(TARGET_PPC64)
    gen_qemu_ld64(s, cpu_gpr[rD(s->opcode)], addr);
#else
    TCGv_i64 t0 = tcg_temp_new_i64();
    gen_qemu_ld64(s, t0, addr);
    tcg_gen_trunc_i64_i32(cpu_gpr[rD(s->opcode)], t0);
    tcg_gen_shri_i64(t0, t0, 32);
    tcg_gen_trunc_i64_i32(cpu_gprh[rD(s->opcode)], t0);
    tcg_temp_free_i64(t0);
#endif
}

static inline void gen_op_evldw(DisasContext *s, TCGv addr)
{
#if defined(TARGET_PPC64)
    TCGv t0 = tcg_temp_new();
    gen_qemu_ld32u(s, t0, addr);
    tcg_gen_shli_tl(cpu_gpr[rD(s->opcode)], t0, 32);
    gen_addr_add(s, addr, addr, 4);
    gen_qemu_ld32u(s, t0, addr);
    tcg_gen_or_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rD(s->opcode)], t0);
    tcg_temp_free(t0);
#else
    gen_qemu_ld32u(s, cpu_gprh[rD(s->opcode)], addr);
    gen_addr_add(s, addr, addr, 4);
    gen_qemu_ld32u(s, cpu_gpr[rD(s->opcode)], addr);
#endif
}

static inline void gen_op_evldh(DisasContext *s, TCGv addr)
{
    TCGv t0 = tcg_temp_new();
#if defined(TARGET_PPC64)
    gen_qemu_ld16u(s, t0, addr);
    tcg_gen_shli_tl(cpu_gpr[rD(s->opcode)], t0, 48);
    gen_addr_add(s, addr, addr, 2);
    gen_qemu_ld16u(s, t0, addr);
    tcg_gen_shli_tl(t0, t0, 32);
    tcg_gen_or_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rD(s->opcode)], t0);
    gen_addr_add(s, addr, addr, 2);
    gen_qemu_ld16u(s, t0, addr);
    tcg_gen_shli_tl(t0, t0, 16);
    tcg_gen_or_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rD(s->opcode)], t0);
    gen_addr_add(s, addr, addr, 2);
    gen_qemu_ld16u(s, t0, addr);
    tcg_gen_or_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rD(s->opcode)], t0);
#else
    gen_qemu_ld16u(s, t0, addr);
    tcg_gen_shli_tl(cpu_gprh[rD(s->opcode)], t0, 16);
    gen_addr_add(s, addr, addr, 2);
    gen_qemu_ld16u(s, t0, addr);
    tcg_gen_or_tl(cpu_gprh[rD(s->opcode)], cpu_gprh[rD(s->opcode)], t0);
    gen_addr_add(s, addr, addr, 2);
    gen_qemu_ld16u(s, t0, addr);
    tcg_gen_shli_tl(cpu_gprh[rD(s->opcode)], t0, 16);
    gen_addr_add(s, addr, addr, 2);
    gen_qemu_ld16u(s, t0, addr);
    tcg_gen_or_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rD(s->opcode)], t0);
#endif
    tcg_temp_free(t0);
}

static inline void gen_op_evlhhesplat(DisasContext *s, TCGv addr)
{
    TCGv t0 = tcg_temp_new();
    gen_qemu_ld16u(s, t0, addr);
#if defined(TARGET_PPC64)
    tcg_gen_shli_tl(cpu_gpr[rD(s->opcode)], t0, 48);
    tcg_gen_shli_tl(t0, t0, 16);
    tcg_gen_or_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rD(s->opcode)], t0);
#else
    tcg_gen_shli_tl(t0, t0, 16);
    tcg_gen_mov_tl(cpu_gprh[rD(s->opcode)], t0);
    tcg_gen_mov_tl(cpu_gpr[rD(s->opcode)], t0);
#endif
    tcg_temp_free(t0);
}

static inline void gen_op_evlhhousplat(DisasContext *s, TCGv addr)
{
    TCGv t0 = tcg_temp_new();
    gen_qemu_ld16u(s, t0, addr);
#if defined(TARGET_PPC64)
    tcg_gen_shli_tl(cpu_gpr[rD(s->opcode)], t0, 32);
    tcg_gen_or_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rD(s->opcode)], t0);
#else
    tcg_gen_mov_tl(cpu_gprh[rD(s->opcode)], t0);
    tcg_gen_mov_tl(cpu_gpr[rD(s->opcode)], t0);
#endif
    tcg_temp_free(t0);
}

static inline void gen_op_evlhhossplat(DisasContext *s, TCGv addr)
{
    TCGv t0 = tcg_temp_new();
    gen_qemu_ld16s(s, t0, addr);
#if defined(TARGET_PPC64)
    tcg_gen_shli_tl(cpu_gpr[rD(s->opcode)], t0, 32);
    tcg_gen_ext32u_tl(t0, t0);
    tcg_gen_or_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rD(s->opcode)], t0);
#else
    tcg_gen_mov_tl(cpu_gprh[rD(s->opcode)], t0);
    tcg_gen_mov_tl(cpu_gpr[rD(s->opcode)], t0);
#endif
    tcg_temp_free(t0);
}

static inline void gen_op_evlwhe(DisasContext *s, TCGv addr)
{
    TCGv t0 = tcg_temp_new();
#if defined(TARGET_PPC64)
    gen_qemu_ld16u(s, t0, addr);
    tcg_gen_shli_tl(cpu_gpr[rD(s->opcode)], t0, 48);
    gen_addr_add(s, addr, addr, 2);
    gen_qemu_ld16u(s, t0, addr);
    tcg_gen_shli_tl(t0, t0, 16);
    tcg_gen_or_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rD(s->opcode)], t0);
#else
    gen_qemu_ld16u(s, t0, addr);
    tcg_gen_shli_tl(cpu_gprh[rD(s->opcode)], t0, 16);
    gen_addr_add(s, addr, addr, 2);
    gen_qemu_ld16u(s, t0, addr);
    tcg_gen_shli_tl(cpu_gpr[rD(s->opcode)], t0, 16);
#endif
    tcg_temp_free(t0);
}

static inline void gen_op_evlwhou(DisasContext *s, TCGv addr)
{
#if defined(TARGET_PPC64)
    TCGv t0 = tcg_temp_new();
    gen_qemu_ld16u(s, cpu_gpr[rD(s->opcode)], addr);
    gen_addr_add(s, addr, addr, 2);
    gen_qemu_ld16u(s, t0, addr);
    tcg_gen_shli_tl(t0, t0, 32);
    tcg_gen_or_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rD(s->opcode)], t0);
    tcg_temp_free(t0);
#else
    gen_qemu_ld16u(s, cpu_gprh[rD(s->opcode)], addr);
    gen_addr_add(s, addr, addr, 2);
    gen_qemu_ld16u(s, cpu_gpr[rD(s->opcode)], addr);
#endif
}

static inline void gen_op_evlwhos(DisasContext *s, TCGv addr)
{
#if defined(TARGET_PPC64)
    TCGv t0 = tcg_temp_new();
    gen_qemu_ld16s(s, t0, addr);
    tcg_gen_ext32u_tl(cpu_gpr[rD(s->opcode)], t0);
    gen_addr_add(s, addr, addr, 2);
    gen_qemu_ld16s(s, t0, addr);
    tcg_gen_shli_tl(t0, t0, 32);
    tcg_gen_or_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rD(s->opcode)], t0);
    tcg_temp_free(t0);
#else
    gen_qemu_ld16s(s, cpu_gprh[rD(s->opcode)], addr);
    gen_addr_add(s, addr, addr, 2);
    gen_qemu_ld16s(s, cpu_gpr[rD(s->opcode)], addr);
#endif
}

static inline void gen_op_evlwwsplat(DisasContext *s, TCGv addr)
{
    TCGv t0 = tcg_temp_new();
    gen_qemu_ld32u(s, t0, addr);
#if defined(TARGET_PPC64)
    tcg_gen_shli_tl(cpu_gpr[rD(s->opcode)], t0, 32);
    tcg_gen_or_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rD(s->opcode)], t0);
#else
    tcg_gen_mov_tl(cpu_gprh[rD(s->opcode)], t0);
    tcg_gen_mov_tl(cpu_gpr[rD(s->opcode)], t0);
#endif
    tcg_temp_free(t0);
}

static inline void gen_op_evlwhsplat(DisasContext *s, TCGv addr)
{
    TCGv t0 = tcg_temp_new();
#if defined(TARGET_PPC64)
    gen_qemu_ld16u(s, t0, addr);
    tcg_gen_shli_tl(cpu_gpr[rD(s->opcode)], t0, 48);
    tcg_gen_shli_tl(t0, t0, 32);
    tcg_gen_or_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rD(s->opcode)], t0);
    gen_addr_add(s, addr, addr, 2);
    gen_qemu_ld16u(s, t0, addr);
    tcg_gen_or_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rD(s->opcode)], t0);
    tcg_gen_shli_tl(t0, t0, 16);
    tcg_gen_or_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rD(s->opcode)], t0);
#else
    gen_qemu_ld16u(s, t0, addr);
    tcg_gen_shli_tl(cpu_gprh[rD(s->opcode)], t0, 16);
    tcg_gen_or_tl(cpu_gprh[rD(s->opcode)], cpu_gprh[rD(s->opcode)], t0);
    gen_addr_add(s, addr, addr, 2);
    gen_qemu_ld16u(s, t0, addr);
    tcg_gen_shli_tl(cpu_gpr[rD(s->opcode)], t0, 16);
    tcg_gen_or_tl(cpu_gpr[rD(s->opcode)], cpu_gprh[rD(s->opcode)], t0);
#endif
    tcg_temp_free(t0);
}

static inline void gen_op_evstdd(DisasContext *s, TCGv addr)
{
#if defined(TARGET_PPC64)
    gen_qemu_st64(s, cpu_gpr[rS(s->opcode)], addr);
#else
    TCGv_i64 t0 = tcg_temp_local_new_i64();
    tcg_gen_concat_i32_i64(t0, cpu_gpr[rS(s->opcode)], cpu_gprh[rS(s->opcode)]);
    gen_qemu_st64(s, t0, addr);
    tcg_temp_free_i64(t0);
#endif
}

static inline void gen_op_evstdw(DisasContext *s, TCGv addr)
{
#if defined(TARGET_PPC64)
    TCGv t0 = tcg_temp_local_new();
    tcg_gen_shri_tl(t0, cpu_gpr[rS(s->opcode)], 32);
    gen_qemu_st32(s, t0, addr);
    tcg_temp_free(t0);
#else
    gen_qemu_st32(s, cpu_gprh[rS(s->opcode)], addr);
#endif
    gen_addr_add(s, addr, addr, 4);
    gen_qemu_st32(s, cpu_gpr[rS(s->opcode)], addr);
}

static inline void gen_op_evstdh(DisasContext *s, TCGv addr)
{
    TCGv t0 = tcg_temp_local_new();

#if defined(TARGET_PPC64)
    tcg_gen_shri_tl(t0, cpu_gpr[rS(s->opcode)], 48);
#else
    tcg_gen_shri_tl(t0, cpu_gprh[rS(s->opcode)], 16);
#endif
    gen_qemu_st16(s, t0, addr);
    gen_addr_add(s, addr, addr, 2);
#if defined(TARGET_PPC64)
    tcg_gen_shri_tl(t0, cpu_gpr[rS(s->opcode)], 32);
    gen_qemu_st16(s, t0, addr);
#else
    gen_qemu_st16(s, cpu_gprh[rS(s->opcode)], addr);
#endif
    gen_addr_add(s, addr, addr, 2);
    tcg_gen_shri_tl(t0, cpu_gpr[rS(s->opcode)], 16);
    gen_qemu_st16(s, t0, addr);
    tcg_temp_free(t0);
    gen_addr_add(s, addr, addr, 2);
    gen_qemu_st16(s, cpu_gpr[rS(s->opcode)], addr);
}

static inline void gen_op_evstwhe(DisasContext *s, TCGv addr)
{
    TCGv t0 = tcg_temp_local_new();
#if defined(TARGET_PPC64)
    tcg_gen_shri_tl(t0, cpu_gpr[rS(s->opcode)], 48);
#else
    tcg_gen_shri_tl(t0, cpu_gprh[rS(s->opcode)], 16);
#endif
    gen_qemu_st16(s, t0, addr);
    gen_addr_add(s, addr, addr, 2);
    tcg_gen_shri_tl(t0, cpu_gpr[rS(s->opcode)], 16);
    gen_qemu_st16(s, t0, addr);
    tcg_temp_free(t0);
}

static inline void gen_op_evstwho(DisasContext *s, TCGv addr)
{
#if defined(TARGET_PPC64)
    TCGv t0 = tcg_temp_local_new();
    tcg_gen_shri_tl(t0, cpu_gpr[rS(s->opcode)], 32);
    gen_qemu_st16(s, t0, addr);
    tcg_temp_free(t0);
#else
    gen_qemu_st16(s, cpu_gprh[rS(s->opcode)], addr);
#endif
    gen_addr_add(s, addr, addr, 2);
    gen_qemu_st16(s, cpu_gpr[rS(s->opcode)], addr);
}

static inline void gen_op_evstwwe(DisasContext *s, TCGv addr)
{
#if defined(TARGET_PPC64)
    TCGv t0 = tcg_temp_local_new();
    tcg_gen_shri_tl(t0, cpu_gpr[rS(s->opcode)], 32);
    gen_qemu_st32(s, t0, addr);
    tcg_temp_free(t0);
#else
    gen_qemu_st32(s, cpu_gprh[rS(s->opcode)], addr);
#endif
}

static inline void gen_op_evstwwo(DisasContext *s, TCGv addr)
{
    gen_qemu_st32(s, cpu_gpr[rS(s->opcode)], addr);
}

#define GEN_SPEOP_LDST(name, opc2, sh)             \
    static void glue(gen_, name)(DisasContext * s) \
    {                                              \
        TCGv t0;                                   \
        if(unlikely(!s->spe_enabled)) {            \
            gen_exception(s, POWERPC_EXCP_SPEU);   \
            return;                                \
        }                                          \
        gen_set_access_type(s, ACCESS_INT);        \
        t0 = tcg_temp_local_new();                 \
        if(Rc(s->opcode)) {                        \
            gen_addr_spe_imm_index(s, t0, sh);     \
        } else {                                   \
            gen_addr_reg_index(s, t0);             \
        }                                          \
        gen_op_##name(s, t0);                      \
        tcg_temp_free(t0);                         \
    }

GEN_SPEOP_LDST(evldd, 0x00, 3);
GEN_SPEOP_LDST(evldw, 0x01, 3);
GEN_SPEOP_LDST(evldh, 0x02, 3);
GEN_SPEOP_LDST(evlhhesplat, 0x04, 1);
GEN_SPEOP_LDST(evlhhousplat, 0x06, 1);
GEN_SPEOP_LDST(evlhhossplat, 0x07, 1);
GEN_SPEOP_LDST(evlwhe, 0x08, 2);
GEN_SPEOP_LDST(evlwhou, 0x0A, 2);
GEN_SPEOP_LDST(evlwhos, 0x0B, 2);
GEN_SPEOP_LDST(evlwwsplat, 0x0C, 2);
GEN_SPEOP_LDST(evlwhsplat, 0x0E, 2);

GEN_SPEOP_LDST(evstdd, 0x10, 3);
GEN_SPEOP_LDST(evstdw, 0x11, 3);
GEN_SPEOP_LDST(evstdh, 0x12, 3);
GEN_SPEOP_LDST(evstwhe, 0x18, 2);
GEN_SPEOP_LDST(evstwho, 0x1A, 2);
GEN_SPEOP_LDST(evstwwe, 0x1C, 2);
GEN_SPEOP_LDST(evstwwo, 0x1E, 2);

/***                      SPE floating-point extension                      ***/
#if defined(TARGET_PPC64)
#define GEN_SPEFPUOP_CONV_32_32(name)                                                           \
    static inline void gen_##name(DisasContext *s)                                              \
    {                                                                                           \
        TCGv_i32 t0;                                                                            \
        TCGv t1;                                                                                \
        t0 = tcg_temp_new_i32();                                                                \
        tcg_gen_trunc_tl_i32(t0, cpu_gpr[rB(s->opcode)]);                                       \
        gen_helper_##name(t0, t0);                                                              \
        t1 = tcg_temp_new();                                                                    \
        tcg_gen_extu_i32_tl(t1, t0);                                                            \
        tcg_temp_free_i32(t0);                                                                  \
        tcg_gen_andi_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rD(s->opcode)], 0xFFFFFFFF00000000ULL); \
        tcg_gen_or_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rD(s->opcode)], t1);                      \
        tcg_temp_free(t1);                                                                      \
    }
#define GEN_SPEFPUOP_CONV_32_64(name)                                                           \
    static inline void gen_##name(DisasContext *s)                                              \
    {                                                                                           \
        TCGv_i32 t0;                                                                            \
        TCGv t1;                                                                                \
        t0 = tcg_temp_new_i32();                                                                \
        gen_helper_##name(t0, cpu_gpr[rB(s->opcode)]);                                          \
        t1 = tcg_temp_new();                                                                    \
        tcg_gen_extu_i32_tl(t1, t0);                                                            \
        tcg_temp_free_i32(t0);                                                                  \
        tcg_gen_andi_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rD(s->opcode)], 0xFFFFFFFF00000000ULL); \
        tcg_gen_or_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rD(s->opcode)], t1);                      \
        tcg_temp_free(t1);                                                                      \
    }
#define GEN_SPEFPUOP_CONV_64_32(name)                     \
    static inline void gen_##name(DisasContext *s)        \
    {                                                     \
        TCGv_i32 t0 = tcg_temp_new_i32();                 \
        tcg_gen_trunc_tl_i32(t0, cpu_gpr[rB(s->opcode)]); \
        gen_helper_##name(cpu_gpr[rD(s->opcode)], t0);    \
        tcg_temp_free_i32(t0);                            \
    }
#define GEN_SPEFPUOP_CONV_64_64(name)                                      \
    static inline void gen_##name(DisasContext *s)                         \
    {                                                                      \
        gen_helper_##name(cpu_gpr[rD(s->opcode)], cpu_gpr[rB(s->opcode)]); \
    }
#define GEN_SPEFPUOP_ARITH2_32_32(name)                                                         \
    static inline void gen_##name(DisasContext *s)                                              \
    {                                                                                           \
        TCGv_i32 t0, t1;                                                                        \
        TCGv_i64 t2;                                                                            \
        if(unlikely(!s->spe_enabled)) {                                                         \
            gen_exception(s, POWERPC_EXCP_SPEU);                                                \
            return;                                                                             \
        }                                                                                       \
        t0 = tcg_temp_new_i32();                                                                \
        t1 = tcg_temp_new_i32();                                                                \
        tcg_gen_trunc_tl_i32(t0, cpu_gpr[rA(s->opcode)]);                                       \
        tcg_gen_trunc_tl_i32(t1, cpu_gpr[rB(s->opcode)]);                                       \
        gen_helper_##name(t0, t0, t1);                                                          \
        tcg_temp_free_i32(t1);                                                                  \
        t2 = tcg_temp_new();                                                                    \
        tcg_gen_extu_i32_tl(t2, t0);                                                            \
        tcg_temp_free_i32(t0);                                                                  \
        tcg_gen_andi_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rD(s->opcode)], 0xFFFFFFFF00000000ULL); \
        tcg_gen_or_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rD(s->opcode)], t2);                      \
        tcg_temp_free(t2);                                                                      \
    }
#define GEN_SPEFPUOP_ARITH2_64_64(name)                                                            \
    static inline void gen_##name(DisasContext *s)                                                 \
    {                                                                                              \
        if(unlikely(!s->spe_enabled)) {                                                            \
            gen_exception(s, POWERPC_EXCP_SPEU);                                                   \
            return;                                                                                \
        }                                                                                          \
        gen_helper_##name(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)]); \
    }
#define GEN_SPEFPUOP_COMP_32(name)                           \
    static inline void gen_##name(DisasContext *s)           \
    {                                                        \
        TCGv_i32 t0, t1;                                     \
        if(unlikely(!s->spe_enabled)) {                      \
            gen_exception(s, POWERPC_EXCP_SPEU);             \
            return;                                          \
        }                                                    \
        t0 = tcg_temp_new_i32();                             \
        t1 = tcg_temp_new_i32();                             \
        tcg_gen_trunc_tl_i32(t0, cpu_gpr[rA(s->opcode)]);    \
        tcg_gen_trunc_tl_i32(t1, cpu_gpr[rB(s->opcode)]);    \
        gen_helper_##name(cpu_crf[crfD(s->opcode)], t0, t1); \
        tcg_temp_free_i32(t0);                               \
        tcg_temp_free_i32(t1);                               \
    }
#define GEN_SPEFPUOP_COMP_64(name)                                                                   \
    static inline void gen_##name(DisasContext *s)                                                   \
    {                                                                                                \
        if(unlikely(!s->spe_enabled)) {                                                              \
            gen_exception(s, POWERPC_EXCP_SPEU);                                                     \
            return;                                                                                  \
        }                                                                                            \
        gen_helper_##name(cpu_crf[crfD(s->opcode)], cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)]); \
    }
#else
#define GEN_SPEFPUOP_CONV_32_32(name)                                      \
    static inline void gen_##name(DisasContext *s)                         \
    {                                                                      \
        gen_helper_##name(cpu_gpr[rD(s->opcode)], cpu_gpr[rB(s->opcode)]); \
    }
#define GEN_SPEFPUOP_CONV_32_64(name)                  \
    static inline void gen_##name(DisasContext *s)     \
    {                                                  \
        TCGv_i64 t0 = tcg_temp_new_i64();              \
        gen_load_gpr64(t0, rB(s->opcode));             \
        gen_helper_##name(cpu_gpr[rD(s->opcode)], t0); \
        tcg_temp_free_i64(t0);                         \
    }
#define GEN_SPEFPUOP_CONV_64_32(name)                  \
    static inline void gen_##name(DisasContext *s)     \
    {                                                  \
        TCGv_i64 t0 = tcg_temp_new_i64();              \
        gen_helper_##name(t0, cpu_gpr[rB(s->opcode)]); \
        gen_store_gpr64(rD(s->opcode), t0);            \
        tcg_temp_free_i64(t0);                         \
    }
#define GEN_SPEFPUOP_CONV_64_64(name)              \
    static inline void gen_##name(DisasContext *s) \
    {                                              \
        TCGv_i64 t0 = tcg_temp_new_i64();          \
        gen_load_gpr64(t0, rB(s->opcode));         \
        gen_helper_##name(t0, t0);                 \
        gen_store_gpr64(rD(s->opcode), t0);        \
        tcg_temp_free_i64(t0);                     \
    }
#define GEN_SPEFPUOP_ARITH2_32_32(name)                                                            \
    static inline void gen_##name(DisasContext *s)                                                 \
    {                                                                                              \
        if(unlikely(!s->spe_enabled)) {                                                            \
            gen_exception(s, POWERPC_EXCP_SPEU);                                                   \
            return;                                                                                \
        }                                                                                          \
        gen_helper_##name(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)]); \
    }
#define GEN_SPEFPUOP_ARITH2_64_64(name)            \
    static inline void gen_##name(DisasContext *s) \
    {                                              \
        TCGv_i64 t0, t1;                           \
        if(unlikely(!s->spe_enabled)) {            \
            gen_exception(s, POWERPC_EXCP_SPEU);   \
            return;                                \
        }                                          \
        t0 = tcg_temp_new_i64();                   \
        t1 = tcg_temp_new_i64();                   \
        gen_load_gpr64(t0, rA(s->opcode));         \
        gen_load_gpr64(t1, rB(s->opcode));         \
        gen_helper_##name(t0, t0, t1);             \
        gen_store_gpr64(rD(s->opcode), t0);        \
        tcg_temp_free_i64(t0);                     \
        tcg_temp_free_i64(t1);                     \
    }
#define GEN_SPEFPUOP_COMP_32(name)                                                                   \
    static inline void gen_##name(DisasContext *s)                                                   \
    {                                                                                                \
        if(unlikely(!s->spe_enabled)) {                                                              \
            gen_exception(s, POWERPC_EXCP_SPEU);                                                     \
            return;                                                                                  \
        }                                                                                            \
        gen_helper_##name(cpu_crf[crfD(s->opcode)], cpu_gpr[rA(s->opcode)], cpu_gpr[rB(s->opcode)]); \
    }
#define GEN_SPEFPUOP_COMP_64(name)                           \
    static inline void gen_##name(DisasContext *s)           \
    {                                                        \
        TCGv_i64 t0, t1;                                     \
        if(unlikely(!s->spe_enabled)) {                      \
            gen_exception(s, POWERPC_EXCP_SPEU);             \
            return;                                          \
        }                                                    \
        t0 = tcg_temp_new_i64();                             \
        t1 = tcg_temp_new_i64();                             \
        gen_load_gpr64(t0, rA(s->opcode));                   \
        gen_load_gpr64(t1, rB(s->opcode));                   \
        gen_helper_##name(cpu_crf[crfD(s->opcode)], t0, t1); \
        tcg_temp_free_i64(t0);                               \
        tcg_temp_free_i64(t1);                               \
    }
#endif

/* Single precision floating-point vectors operations */
/* Arithmetic */
GEN_SPEFPUOP_ARITH2_64_64(evfsadd);
GEN_SPEFPUOP_ARITH2_64_64(evfssub);
GEN_SPEFPUOP_ARITH2_64_64(evfsmul);
GEN_SPEFPUOP_ARITH2_64_64(evfsdiv);
static inline void gen_evfsabs(DisasContext *s)
{
    if(unlikely(!s->spe_enabled)) {
        gen_exception(s, POWERPC_EXCP_SPEU);
        return;
    }
#if defined(TARGET_PPC64)
    tcg_gen_andi_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], ~0x8000000080000000LL);
#else
    tcg_gen_andi_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], ~0x80000000);
    tcg_gen_andi_tl(cpu_gprh[rD(s->opcode)], cpu_gprh[rA(s->opcode)], ~0x80000000);
#endif
}
static inline void gen_evfsnabs(DisasContext *s)
{
    if(unlikely(!s->spe_enabled)) {
        gen_exception(s, POWERPC_EXCP_SPEU);
        return;
    }
#if defined(TARGET_PPC64)
    tcg_gen_ori_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], 0x8000000080000000LL);
#else
    tcg_gen_ori_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], 0x80000000);
    tcg_gen_ori_tl(cpu_gprh[rD(s->opcode)], cpu_gprh[rA(s->opcode)], 0x80000000);
#endif
}
static inline void gen_evfsneg(DisasContext *s)
{
    if(unlikely(!s->spe_enabled)) {
        gen_exception(s, POWERPC_EXCP_SPEU);
        return;
    }
#if defined(TARGET_PPC64)
    tcg_gen_xori_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], 0x8000000080000000LL);
#else
    tcg_gen_xori_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], 0x80000000);
    tcg_gen_xori_tl(cpu_gprh[rD(s->opcode)], cpu_gprh[rA(s->opcode)], 0x80000000);
#endif
}

/* Conversion */
GEN_SPEFPUOP_CONV_64_64(evfscfui);
GEN_SPEFPUOP_CONV_64_64(evfscfsi);
GEN_SPEFPUOP_CONV_64_64(evfscfuf);
GEN_SPEFPUOP_CONV_64_64(evfscfsf);
GEN_SPEFPUOP_CONV_64_64(evfsctui);
GEN_SPEFPUOP_CONV_64_64(evfsctsi);
GEN_SPEFPUOP_CONV_64_64(evfsctuf);
GEN_SPEFPUOP_CONV_64_64(evfsctsf);
GEN_SPEFPUOP_CONV_64_64(evfsctuiz);
GEN_SPEFPUOP_CONV_64_64(evfsctsiz);

/* Comparison */
GEN_SPEFPUOP_COMP_64(evfscmpgt);
GEN_SPEFPUOP_COMP_64(evfscmplt);
GEN_SPEFPUOP_COMP_64(evfscmpeq);
GEN_SPEFPUOP_COMP_64(evfststgt);
GEN_SPEFPUOP_COMP_64(evfststlt);
GEN_SPEFPUOP_COMP_64(evfststeq);

/* Opcodes definitions */
GEN_SPE(evfsadd, evfssub, 0x00, 0x0A, 0x00000000, 0x00000000, PPC_SPE_SINGLE);
GEN_SPE(evfsabs, evfsnabs, 0x02, 0x0A, 0x0000F800, 0x0000F800, PPC_SPE_SINGLE);
GEN_SPE(evfsneg, speundef, 0x03, 0x0A, 0x0000F800, 0xFFFFFFFF, PPC_SPE_SINGLE);
GEN_SPE(evfsmul, evfsdiv, 0x04, 0x0A, 0x00000000, 0x00000000, PPC_SPE_SINGLE);
GEN_SPE(evfscmpgt, evfscmplt, 0x06, 0x0A, 0x00600000, 0x00600000, PPC_SPE_SINGLE);
GEN_SPE(evfscmpeq, speundef, 0x07, 0x0A, 0x00600000, 0xFFFFFFFF, PPC_SPE_SINGLE);
GEN_SPE(evfscfui, evfscfsi, 0x08, 0x0A, 0x00180000, 0x00180000, PPC_SPE_SINGLE);
GEN_SPE(evfscfuf, evfscfsf, 0x09, 0x0A, 0x00180000, 0x00180000, PPC_SPE_SINGLE);
GEN_SPE(evfsctui, evfsctsi, 0x0A, 0x0A, 0x00180000, 0x00180000, PPC_SPE_SINGLE);
GEN_SPE(evfsctuf, evfsctsf, 0x0B, 0x0A, 0x00180000, 0x00180000, PPC_SPE_SINGLE);
GEN_SPE(evfsctuiz, speundef, 0x0C, 0x0A, 0x00180000, 0xFFFFFFFF, PPC_SPE_SINGLE);
GEN_SPE(evfsctsiz, speundef, 0x0D, 0x0A, 0x00180000, 0xFFFFFFFF, PPC_SPE_SINGLE);
GEN_SPE(evfststgt, evfststlt, 0x0E, 0x0A, 0x00600000, 0x00600000, PPC_SPE_SINGLE);
GEN_SPE(evfststeq, speundef, 0x0F, 0x0A, 0x00600000, 0xFFFFFFFF, PPC_SPE_SINGLE);

/* Single precision floating-point operations */
/* Arithmetic */
GEN_SPEFPUOP_ARITH2_32_32(efsadd);
GEN_SPEFPUOP_ARITH2_32_32(efssub);
GEN_SPEFPUOP_ARITH2_32_32(efsmul);
GEN_SPEFPUOP_ARITH2_32_32(efsdiv);
static inline void gen_efsabs(DisasContext *s)
{
    if(unlikely(!s->spe_enabled)) {
        gen_exception(s, POWERPC_EXCP_SPEU);
        return;
    }
    tcg_gen_andi_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], (target_long)~0x80000000LL);
}
static inline void gen_efsnabs(DisasContext *s)
{
    if(unlikely(!s->spe_enabled)) {
        gen_exception(s, POWERPC_EXCP_SPEU);
        return;
    }
    tcg_gen_ori_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], 0x80000000);
}
static inline void gen_efsneg(DisasContext *s)
{
    if(unlikely(!s->spe_enabled)) {
        gen_exception(s, POWERPC_EXCP_SPEU);
        return;
    }
    tcg_gen_xori_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], 0x80000000);
}

/* Conversion */
GEN_SPEFPUOP_CONV_32_32(efscfui);
GEN_SPEFPUOP_CONV_32_32(efscfsi);
GEN_SPEFPUOP_CONV_32_32(efscfuf);
GEN_SPEFPUOP_CONV_32_32(efscfsf);
GEN_SPEFPUOP_CONV_32_32(efsctui);
GEN_SPEFPUOP_CONV_32_32(efsctsi);
GEN_SPEFPUOP_CONV_32_32(efsctuf);
GEN_SPEFPUOP_CONV_32_32(efsctsf);
GEN_SPEFPUOP_CONV_32_32(efsctuiz);
GEN_SPEFPUOP_CONV_32_32(efsctsiz);
GEN_SPEFPUOP_CONV_32_64(efscfd);

/* Comparison */
GEN_SPEFPUOP_COMP_32(efscmpgt);
GEN_SPEFPUOP_COMP_32(efscmplt);
GEN_SPEFPUOP_COMP_32(efscmpeq);
GEN_SPEFPUOP_COMP_32(efststgt);
GEN_SPEFPUOP_COMP_32(efststlt);
GEN_SPEFPUOP_COMP_32(efststeq);

/* Opcodes definitions */
GEN_SPE(efsadd, efssub, 0x00, 0x0B, 0x00000000, 0x00000000, PPC_SPE_SINGLE);
GEN_SPE(efsabs, efsnabs, 0x02, 0x0B, 0x0000F800, 0x0000F800, PPC_SPE_SINGLE);
GEN_SPE(efsneg, speundef, 0x03, 0x0B, 0x0000F800, 0xFFFFFFFF, PPC_SPE_SINGLE);
GEN_SPE(efsmul, efsdiv, 0x04, 0x0B, 0x00000000, 0x00000000, PPC_SPE_SINGLE);
GEN_SPE(efscmpgt, efscmplt, 0x06, 0x0B, 0x00600000, 0x00600000, PPC_SPE_SINGLE);
GEN_SPE(efscmpeq, efscfd, 0x07, 0x0B, 0x00600000, 0x00180000, PPC_SPE_SINGLE);
GEN_SPE(efscfui, efscfsi, 0x08, 0x0B, 0x00180000, 0x00180000, PPC_SPE_SINGLE);
GEN_SPE(efscfuf, efscfsf, 0x09, 0x0B, 0x00180000, 0x00180000, PPC_SPE_SINGLE);
GEN_SPE(efsctui, efsctsi, 0x0A, 0x0B, 0x00180000, 0x00180000, PPC_SPE_SINGLE);
GEN_SPE(efsctuf, efsctsf, 0x0B, 0x0B, 0x00180000, 0x00180000, PPC_SPE_SINGLE);
GEN_SPE(efsctuiz, speundef, 0x0C, 0x0B, 0x00180000, 0xFFFFFFFF, PPC_SPE_SINGLE);
GEN_SPE(efsctsiz, speundef, 0x0D, 0x0B, 0x00180000, 0xFFFFFFFF, PPC_SPE_SINGLE);
GEN_SPE(efststgt, efststlt, 0x0E, 0x0B, 0x00600000, 0x00600000, PPC_SPE_SINGLE);
GEN_SPE(efststeq, speundef, 0x0F, 0x0B, 0x00600000, 0xFFFFFFFF, PPC_SPE_SINGLE);

/* Double precision floating-point operations */
/* Arithmetic */
GEN_SPEFPUOP_ARITH2_64_64(efdadd);
GEN_SPEFPUOP_ARITH2_64_64(efdsub);
GEN_SPEFPUOP_ARITH2_64_64(efdmul);
GEN_SPEFPUOP_ARITH2_64_64(efddiv);
static inline void gen_efdabs(DisasContext *s)
{
    if(unlikely(!s->spe_enabled)) {
        gen_exception(s, POWERPC_EXCP_SPEU);
        return;
    }
#if defined(TARGET_PPC64)
    tcg_gen_andi_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], ~0x8000000000000000LL);
#else
    tcg_gen_mov_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)]);
    tcg_gen_andi_tl(cpu_gprh[rD(s->opcode)], cpu_gprh[rA(s->opcode)], ~0x80000000);
#endif
}
static inline void gen_efdnabs(DisasContext *s)
{
    if(unlikely(!s->spe_enabled)) {
        gen_exception(s, POWERPC_EXCP_SPEU);
        return;
    }
#if defined(TARGET_PPC64)
    tcg_gen_ori_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], 0x8000000000000000LL);
#else
    tcg_gen_mov_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)]);
    tcg_gen_ori_tl(cpu_gprh[rD(s->opcode)], cpu_gprh[rA(s->opcode)], 0x80000000);
#endif
}
static inline void gen_efdneg(DisasContext *s)
{
    if(unlikely(!s->spe_enabled)) {
        gen_exception(s, POWERPC_EXCP_SPEU);
        return;
    }
#if defined(TARGET_PPC64)
    tcg_gen_xori_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)], 0x8000000000000000LL);
#else
    tcg_gen_mov_tl(cpu_gpr[rD(s->opcode)], cpu_gpr[rA(s->opcode)]);
    tcg_gen_xori_tl(cpu_gprh[rD(s->opcode)], cpu_gprh[rA(s->opcode)], 0x80000000);
#endif
}

/* Conversion */
GEN_SPEFPUOP_CONV_64_32(efdcfui);
GEN_SPEFPUOP_CONV_64_32(efdcfsi);
GEN_SPEFPUOP_CONV_64_32(efdcfuf);
GEN_SPEFPUOP_CONV_64_32(efdcfsf);
GEN_SPEFPUOP_CONV_32_64(efdctui);
GEN_SPEFPUOP_CONV_32_64(efdctsi);
GEN_SPEFPUOP_CONV_32_64(efdctuf);
GEN_SPEFPUOP_CONV_32_64(efdctsf);
GEN_SPEFPUOP_CONV_32_64(efdctuiz);
GEN_SPEFPUOP_CONV_32_64(efdctsiz);
GEN_SPEFPUOP_CONV_64_32(efdcfs);
GEN_SPEFPUOP_CONV_64_64(efdcfuid);
GEN_SPEFPUOP_CONV_64_64(efdcfsid);
GEN_SPEFPUOP_CONV_64_64(efdctuidz);
GEN_SPEFPUOP_CONV_64_64(efdctsidz);

/* Comparison */
GEN_SPEFPUOP_COMP_64(efdcmpgt);
GEN_SPEFPUOP_COMP_64(efdcmplt);
GEN_SPEFPUOP_COMP_64(efdcmpeq);
GEN_SPEFPUOP_COMP_64(efdtstgt);
GEN_SPEFPUOP_COMP_64(efdtstlt);
GEN_SPEFPUOP_COMP_64(efdtsteq);

/* Opcodes definitions */
GEN_SPE(efdadd, efdsub, 0x10, 0x0B, 0x00000000, 0x00000000, PPC_SPE_DOUBLE);
GEN_SPE(efdcfuid, efdcfsid, 0x11, 0x0B, 0x00180000, 0x00180000, PPC_SPE_DOUBLE);
GEN_SPE(efdabs, efdnabs, 0x12, 0x0B, 0x0000F800, 0x0000F800, PPC_SPE_DOUBLE);
GEN_SPE(efdneg, speundef, 0x13, 0x0B, 0x0000F800, 0xFFFFFFFF, PPC_SPE_DOUBLE);
GEN_SPE(efdmul, efddiv, 0x14, 0x0B, 0x00000000, 0x00000000, PPC_SPE_DOUBLE);
GEN_SPE(efdctuidz, efdctsidz, 0x15, 0x0B, 0x00180000, 0x00180000, PPC_SPE_DOUBLE);
GEN_SPE(efdcmpgt, efdcmplt, 0x16, 0x0B, 0x00600000, 0x00600000, PPC_SPE_DOUBLE);
GEN_SPE(efdcmpeq, efdcfs, 0x17, 0x0B, 0x00600000, 0x00180000, PPC_SPE_DOUBLE);
GEN_SPE(efdcfui, efdcfsi, 0x18, 0x0B, 0x00180000, 0x00180000, PPC_SPE_DOUBLE);
GEN_SPE(efdcfuf, efdcfsf, 0x19, 0x0B, 0x00180000, 0x00180000, PPC_SPE_DOUBLE);
GEN_SPE(efdctui, efdctsi, 0x1A, 0x0B, 0x00180000, 0x00180000, PPC_SPE_DOUBLE);
GEN_SPE(efdctuf, efdctsf, 0x1B, 0x0B, 0x00180000, 0x00180000, PPC_SPE_DOUBLE);
GEN_SPE(efdctuiz, speundef, 0x1C, 0x0B, 0x00180000, 0xFFFFFFFF, PPC_SPE_DOUBLE);
GEN_SPE(efdctsiz, speundef, 0x1D, 0x0B, 0x00180000, 0xFFFFFFFF, PPC_SPE_DOUBLE);
GEN_SPE(efdtstgt, efdtstlt, 0x1E, 0x0B, 0x00600000, 0x00600000, PPC_SPE_DOUBLE);
GEN_SPE(efdtsteq, speundef, 0x1F, 0x0B, 0x00600000, 0xFFFFFFFF, PPC_SPE_DOUBLE);

// clang-format off

static opcode_t opcodes[] = {
GEN_HANDLER(invalid, 0x00, 0x00, 0x00, 0xFFFFFFFF, PPC_NONE),
GEN_HANDLER(cmp, 0x1F, 0x00, 0x00, 0x00400000, PPC_INTEGER),
GEN_HANDLER(cmpi, 0x0B, 0xFF, 0xFF, 0x00400000, PPC_INTEGER),
GEN_HANDLER(cmpl, 0x1F, 0x00, 0x01, 0x00400000, PPC_INTEGER),
GEN_HANDLER(cmpli, 0x0A, 0xFF, 0xFF, 0x00400000, PPC_INTEGER),
GEN_HANDLER(isel, 0x1F, 0x0F, 0xFF, 0x00000001, PPC_ISEL),
GEN_HANDLER(addi, 0x0E, 0xFF, 0xFF, 0x00000000, PPC_INTEGER),
GEN_HANDLER(addic, 0x0C, 0xFF, 0xFF, 0x00000000, PPC_INTEGER),
GEN_HANDLER2(addic_, "addic.", 0x0D, 0xFF, 0xFF, 0x00000000, PPC_INTEGER),
GEN_HANDLER(addis, 0x0F, 0xFF, 0xFF, 0x00000000, PPC_INTEGER),
GEN_HANDLER(mulhw, 0x1F, 0x0B, 0x02, 0x00000400, PPC_INTEGER),
GEN_HANDLER(mulhwu, 0x1F, 0x0B, 0x00, 0x00000400, PPC_INTEGER),
GEN_HANDLER(mullw, 0x1F, 0x0B, 0x07, 0x00000000, PPC_INTEGER),
GEN_HANDLER(mullwo, 0x1F, 0x0B, 0x17, 0x00000000, PPC_INTEGER),
GEN_HANDLER(mulli, 0x07, 0xFF, 0xFF, 0x00000000, PPC_INTEGER),
#if defined(TARGET_PPC64)
GEN_HANDLER(mulld, 0x1F, 0x09, 0x07, 0x00000000, PPC_64B),
#endif
GEN_HANDLER(neg, 0x1F, 0x08, 0x03, 0x0000F800, PPC_INTEGER),
GEN_HANDLER(nego, 0x1F, 0x08, 0x13, 0x0000F800, PPC_INTEGER),
GEN_HANDLER(subfic, 0x08, 0xFF, 0xFF, 0x00000000, PPC_INTEGER),
GEN_HANDLER2(andi_, "andi.", 0x1C, 0xFF, 0xFF, 0x00000000, PPC_INTEGER),
GEN_HANDLER2(andis_, "andis.", 0x1D, 0xFF, 0xFF, 0x00000000, PPC_INTEGER),
GEN_HANDLER(cntlzw, 0x1F, 0x1A, 0x00, 0x00000000, PPC_INTEGER),
GEN_HANDLER(or, 0x1F, 0x1C, 0x0D, 0x00000000, PPC_INTEGER),
GEN_HANDLER(xor, 0x1F, 0x1C, 0x09, 0x00000000, PPC_INTEGER),
GEN_HANDLER(ori, 0x18, 0xFF, 0xFF, 0x00000000, PPC_INTEGER),
GEN_HANDLER(oris, 0x19, 0xFF, 0xFF, 0x00000000, PPC_INTEGER),
GEN_HANDLER(xori, 0x1A, 0xFF, 0xFF, 0x00000000, PPC_INTEGER),
GEN_HANDLER(xoris, 0x1B, 0xFF, 0xFF, 0x00000000, PPC_INTEGER),
GEN_HANDLER(popcntb, 0x1F, 0x03, 0x03, 0x0000F801, PPC_POPCNTB),
GEN_HANDLER(popcntw, 0x1F, 0x1A, 0x0b, 0x0000F801, PPC_POPCNTWD),
#if defined(TARGET_PPC64)
GEN_HANDLER(popcntd, 0x1F, 0x1A, 0x0F, 0x0000F801, PPC_POPCNTWD),
GEN_HANDLER(cntlzd, 0x1F, 0x1A, 0x01, 0x00000000, PPC_64B),
#endif
GEN_HANDLER(rlwimi, 0x14, 0xFF, 0xFF, 0x00000000, PPC_INTEGER),
GEN_HANDLER(rlwinm, 0x15, 0xFF, 0xFF, 0x00000000, PPC_INTEGER),
GEN_HANDLER(rlwnm, 0x17, 0xFF, 0xFF, 0x00000000, PPC_INTEGER),
GEN_HANDLER(slw, 0x1F, 0x18, 0x00, 0x00000000, PPC_INTEGER),
GEN_HANDLER(sraw, 0x1F, 0x18, 0x18, 0x00000000, PPC_INTEGER),
GEN_HANDLER(srawi, 0x1F, 0x18, 0x19, 0x00000000, PPC_INTEGER),
GEN_HANDLER(srw, 0x1F, 0x18, 0x10, 0x00000000, PPC_INTEGER),
#if defined(TARGET_PPC64)
GEN_HANDLER(sld, 0x1F, 0x1B, 0x00, 0x00000000, PPC_64B),
GEN_HANDLER(srad, 0x1F, 0x1A, 0x18, 0x00000000, PPC_64B),
GEN_HANDLER2(sradi0, "sradi", 0x1F, 0x1A, 0x19, 0x00000000, PPC_64B),
GEN_HANDLER2(sradi1, "sradi", 0x1F, 0x1B, 0x19, 0x00000000, PPC_64B),
GEN_HANDLER(srd, 0x1F, 0x1B, 0x10, 0x00000000, PPC_64B),
#endif
GEN_HANDLER(frsqrtes, 0x3B, 0x1A, 0xFF, 0x001F07C0, PPC_FLOAT_FRSQRTES),
GEN_HANDLER(fsqrt, 0x3F, 0x16, 0xFF, 0x001F07C0, PPC_FLOAT_FSQRT),
GEN_HANDLER(fsqrts, 0x3B, 0x16, 0xFF, 0x001F07C0, PPC_FLOAT_FSQRT),
GEN_HANDLER(fcmpo, 0x3F, 0x00, 0x01, 0x00600001, PPC_FLOAT),
GEN_HANDLER(fcmpu, 0x3F, 0x00, 0x00, 0x00600001, PPC_FLOAT),
GEN_HANDLER(fmr, 0x3F, 0x08, 0x02, 0x001F0000, PPC_FLOAT),
GEN_HANDLER(mcrfs, 0x3F, 0x00, 0x02, 0x0063F801, PPC_FLOAT),
GEN_HANDLER(mffs, 0x3F, 0x07, 0x12, 0x001FF800, PPC_FLOAT),
GEN_HANDLER(mtfsb0, 0x3F, 0x06, 0x02, 0x001FF800, PPC_FLOAT),
GEN_HANDLER(mtfsb1, 0x3F, 0x06, 0x01, 0x001FF800, PPC_FLOAT),
GEN_HANDLER(mtfsf, 0x3F, 0x07, 0x16, 0x00010000, PPC_FLOAT),
GEN_HANDLER(mtfsfi, 0x3F, 0x06, 0x04, 0x006f0800, PPC_FLOAT),
#if defined(TARGET_PPC64)
GEN_HANDLER(ld, 0x3A, 0xFF, 0xFF, 0x00000000, PPC_64B),
GEN_HANDLER(lq, 0x38, 0xFF, 0xFF, 0x00000000, PPC_64BX),
GEN_HANDLER(std, 0x3E, 0xFF, 0xFF, 0x00000000, PPC_64B),
#endif
GEN_HANDLER(lmw, 0x2E, 0xFF, 0xFF, 0x00000000, PPC_INTEGER),
GEN_HANDLER(stmw, 0x2F, 0xFF, 0xFF, 0x00000000, PPC_INTEGER),
GEN_HANDLER(lswi, 0x1F, 0x15, 0x12, 0x00000001, PPC_STRING),
GEN_HANDLER(lswx, 0x1F, 0x15, 0x10, 0x00000001, PPC_STRING),
GEN_HANDLER(stswi, 0x1F, 0x15, 0x16, 0x00000001, PPC_STRING),
GEN_HANDLER(stswx, 0x1F, 0x15, 0x14, 0x00000001, PPC_STRING),
GEN_HANDLER(eieio, 0x1F, 0x16, 0x1A, 0x03FFF801, PPC_MEM_EIEIO),
GEN_HANDLER(isync, 0x13, 0x16, 0x04, 0x03FFF801, PPC_MEM),
GEN_HANDLER(lwarx, 0x1F, 0x14, 0x00, 0x00000000, PPC_RES),
GEN_HANDLER2(stwcx_, "stwcx.", 0x1F, 0x16, 0x04, 0x00000000, PPC_RES),
#if defined(TARGET_PPC64)
GEN_HANDLER(ldarx, 0x1F, 0x14, 0x02, 0x00000000, PPC_64B),
GEN_HANDLER2(stdcx_, "stdcx.", 0x1F, 0x16, 0x06, 0x00000000, PPC_64B),
#endif
GEN_HANDLER(sync, 0x1F, 0x16, 0x12, 0x039FF801, PPC_MEM_SYNC),
GEN_HANDLER(wait, 0x1F, 0x1E, 0x01, 0x03FFF801, PPC_WAIT),
GEN_HANDLER(b, 0x12, 0xFF, 0xFF, 0x00000000, PPC_FLOW),
GEN_HANDLER(bc, 0x10, 0xFF, 0xFF, 0x00000000, PPC_FLOW),
GEN_HANDLER(bcctr, 0x13, 0x10, 0x10, 0x00000000, PPC_FLOW),
GEN_HANDLER(bclr, 0x13, 0x10, 0x00, 0x00000000, PPC_FLOW),
GEN_HANDLER(mcrf, 0x13, 0x00, 0xFF, 0x00000001, PPC_INTEGER),
GEN_HANDLER(rfi, 0x13, 0x12, 0x01, 0x03FF8001, PPC_FLOW),
#if defined(TARGET_PPC64)
GEN_HANDLER(rfid, 0x13, 0x12, 0x00, 0x03FF8001, PPC_64B),
GEN_HANDLER(hrfid, 0x13, 0x12, 0x08, 0x03FF8001, PPC_64H),
#endif
GEN_HANDLER(sc, 0x11, 0xFF, 0xFF, 0x03FFF01D, PPC_FLOW),
GEN_HANDLER(tw, 0x1F, 0x04, 0x00, 0x00000001, PPC_FLOW),
GEN_HANDLER(twi, 0x03, 0xFF, 0xFF, 0x00000000, PPC_FLOW),
#if defined(TARGET_PPC64)
GEN_HANDLER(td, 0x1F, 0x04, 0x02, 0x00000001, PPC_64B),
GEN_HANDLER(tdi, 0x02, 0xFF, 0xFF, 0x00000000, PPC_64B),
#endif
GEN_HANDLER(mcrxr, 0x1F, 0x00, 0x10, 0x007FF801, PPC_MISC),
GEN_HANDLER(mfcr, 0x1F, 0x13, 0x00, 0x00000801, PPC_MISC),
GEN_HANDLER(mfmsr, 0x1F, 0x13, 0x02, 0x001FF801, PPC_MISC),
GEN_HANDLER(mfspr, 0x1F, 0x13, 0x0A, 0x00000001, PPC_MISC),
GEN_HANDLER(mftb, 0x1F, 0x13, 0x0B, 0x00000001, PPC_MFTB),
GEN_HANDLER(mtcrf, 0x1F, 0x10, 0x04, 0x00000801, PPC_MISC),
#if defined(TARGET_PPC64)
GEN_HANDLER(mtmsrd, 0x1F, 0x12, 0x05, 0x001EF801, PPC_64B),
#endif
GEN_HANDLER(mtmsr, 0x1F, 0x12, 0x04, 0x001FF801, PPC_MISC),
GEN_HANDLER(mtspr, 0x1F, 0x13, 0x0E, 0x00000001, PPC_MISC),
GEN_HANDLER(dcbf, 0x1F, 0x16, 0x02, 0x03C00001, PPC_CACHE),
GEN_HANDLER(dcbi, 0x1F, 0x16, 0x0E, 0x03E00001, PPC_CACHE),
GEN_HANDLER(dcbst, 0x1F, 0x16, 0x01, 0x03E00001, PPC_CACHE),
GEN_HANDLER(dcbt, 0x1F, 0x16, 0x08, 0x02000001, PPC_CACHE),
GEN_HANDLER(dcbtst, 0x1F, 0x16, 0x07, 0x02000001, PPC_CACHE),
GEN_HANDLER(dcbz, 0x1F, 0x16, 0x1F, 0x03E00001, PPC_CACHE_DCBZ),
GEN_HANDLER2(dcbz_970, "dcbz", 0x1F, 0x16, 0x1F, 0x03C00001, PPC_CACHE_DCBZT),
GEN_HANDLER(dst, 0x1F, 0x16, 0x0A, 0x01800001, PPC_ALTIVEC),
GEN_HANDLER(dstst, 0x1F, 0x16, 0x0B, 0x02000001, PPC_ALTIVEC),
GEN_HANDLER(dss, 0x1F, 0x16, 0x19, 0x019FF801, PPC_ALTIVEC),
GEN_HANDLER(icbi, 0x1F, 0x16, 0x1E, 0x03E00001, PPC_CACHE_ICBI),
GEN_HANDLER(dcba, 0x1F, 0x16, 0x17, 0x03E00001, PPC_CACHE_DCBA),
GEN_HANDLER(mfsr, 0x1F, 0x13, 0x12, 0x0010F801, PPC_SEGMENT),
GEN_HANDLER(mfsrin, 0x1F, 0x13, 0x14, 0x001F0001, PPC_SEGMENT),
GEN_HANDLER(mtsr, 0x1F, 0x12, 0x06, 0x0010F801, PPC_SEGMENT),
GEN_HANDLER(mtsrin, 0x1F, 0x12, 0x07, 0x001F0001, PPC_SEGMENT),
#if defined(TARGET_PPC64)
GEN_HANDLER2(mfsr_64b, "mfsr", 0x1F, 0x13, 0x12, 0x0010F801, PPC_SEGMENT_64B),
GEN_HANDLER2(mfsrin_64b, "mfsrin", 0x1F, 0x13, 0x14, 0x001F0001,
             PPC_SEGMENT_64B),
GEN_HANDLER2(mtsr_64b, "mtsr", 0x1F, 0x12, 0x06, 0x0010F801, PPC_SEGMENT_64B),
GEN_HANDLER2(mtsrin_64b, "mtsrin", 0x1F, 0x12, 0x07, 0x001F0001,
             PPC_SEGMENT_64B),
GEN_HANDLER2(slbmte, "slbmte", 0x1F, 0x12, 0x0C, 0x001F0001, PPC_SEGMENT_64B),
GEN_HANDLER2(slbmfee, "slbmfee", 0x1F, 0x13, 0x1C, 0x001F0001, PPC_SEGMENT_64B),
GEN_HANDLER2(slbmfev, "slbmfev", 0x1F, 0x13, 0x1A, 0x001F0001, PPC_SEGMENT_64B),
#endif
GEN_HANDLER(tlbia, 0x1F, 0x12, 0x0B, 0x03FFFC01, PPC_MEM_TLBIA),
GEN_HANDLER(tlbiel, 0x1F, 0x12, 0x08, 0x03FF0001, PPC_MEM_TLBIE),
GEN_HANDLER(tlbie, 0x1F, 0x12, 0x09, 0x03FF0001, PPC_MEM_TLBIE),
GEN_HANDLER(tlbsync, 0x1F, 0x16, 0x11, 0x03FFF801, PPC_MEM_TLBSYNC),
#if defined(TARGET_PPC64)
GEN_HANDLER(slbia, 0x1F, 0x12, 0x0F, 0x03FFFC01, PPC_SLBI),
GEN_HANDLER(slbie, 0x1F, 0x12, 0x0D, 0x03FF0001, PPC_SLBI),
#endif
GEN_HANDLER(eciwx, 0x1F, 0x16, 0x0D, 0x00000001, PPC_EXTERN),
GEN_HANDLER(ecowx, 0x1F, 0x16, 0x09, 0x00000001, PPC_EXTERN),
GEN_HANDLER(abs, 0x1F, 0x08, 0x0B, 0x0000F800, PPC_POWER_BR),
GEN_HANDLER(abso, 0x1F, 0x08, 0x1B, 0x0000F800, PPC_POWER_BR),
GEN_HANDLER(clcs, 0x1F, 0x10, 0x13, 0x0000F800, PPC_POWER_BR),
GEN_HANDLER(div, 0x1F, 0x0B, 0x0A, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(divo, 0x1F, 0x0B, 0x1A, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(divs, 0x1F, 0x0B, 0x0B, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(divso, 0x1F, 0x0B, 0x1B, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(doz, 0x1F, 0x08, 0x08, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(dozo, 0x1F, 0x08, 0x18, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(dozi, 0x09, 0xFF, 0xFF, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(lscbx, 0x1F, 0x15, 0x08, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(maskg, 0x1F, 0x1D, 0x00, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(maskir, 0x1F, 0x1D, 0x10, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(mul, 0x1F, 0x0B, 0x03, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(mulo, 0x1F, 0x0B, 0x13, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(nabs, 0x1F, 0x08, 0x0F, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(nabso, 0x1F, 0x08, 0x1F, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(rlmi, 0x16, 0xFF, 0xFF, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(rrib, 0x1F, 0x19, 0x10, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(sle, 0x1F, 0x19, 0x04, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(sleq, 0x1F, 0x19, 0x06, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(sliq, 0x1F, 0x18, 0x05, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(slliq, 0x1F, 0x18, 0x07, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(sllq, 0x1F, 0x18, 0x06, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(slq, 0x1F, 0x18, 0x04, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(sraiq, 0x1F, 0x18, 0x1D, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(sraq, 0x1F, 0x18, 0x1C, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(sre, 0x1F, 0x19, 0x14, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(srea, 0x1F, 0x19, 0x1C, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(sreq, 0x1F, 0x19, 0x16, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(sriq, 0x1F, 0x18, 0x15, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(srliq, 0x1F, 0x18, 0x17, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(srlq, 0x1F, 0x18, 0x16, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(srq, 0x1F, 0x18, 0x14, 0x00000000, PPC_POWER_BR),
GEN_HANDLER(dsa, 0x1F, 0x14, 0x13, 0x03FFF801, PPC_602_SPEC),
GEN_HANDLER(esa, 0x1F, 0x14, 0x12, 0x03FFF801, PPC_602_SPEC),
GEN_HANDLER(mfrom, 0x1F, 0x09, 0x08, 0x03E0F801, PPC_602_SPEC),
GEN_HANDLER2(tlbld_6xx, "tlbld", 0x1F, 0x12, 0x1E, 0x03FF0001, PPC_6xx_TLB),
GEN_HANDLER2(tlbli_6xx, "tlbli", 0x1F, 0x12, 0x1F, 0x03FF0001, PPC_6xx_TLB),
GEN_HANDLER2(tlbld_74xx, "tlbld", 0x1F, 0x12, 0x1E, 0x03FF0001, PPC_74xx_TLB),
GEN_HANDLER2(tlbli_74xx, "tlbli", 0x1F, 0x12, 0x1F, 0x03FF0001, PPC_74xx_TLB),
GEN_HANDLER(clf, 0x1F, 0x16, 0x03, 0x03E00000, PPC_POWER),
GEN_HANDLER(cli, 0x1F, 0x16, 0x0F, 0x03E00000, PPC_POWER),
GEN_HANDLER(dclst, 0x1F, 0x16, 0x13, 0x03E00000, PPC_POWER),
GEN_HANDLER(mfsri, 0x1F, 0x13, 0x13, 0x00000001, PPC_POWER),
GEN_HANDLER(rac, 0x1F, 0x12, 0x19, 0x00000001, PPC_POWER),
GEN_HANDLER(rfsvc, 0x13, 0x12, 0x02, 0x03FFF0001, PPC_POWER),
GEN_HANDLER(lfq, 0x38, 0xFF, 0xFF, 0x00000003, PPC_POWER2),
GEN_HANDLER(lfqu, 0x39, 0xFF, 0xFF, 0x00000003, PPC_POWER2),
GEN_HANDLER(lfqux, 0x1F, 0x17, 0x19, 0x00000001, PPC_POWER2),
GEN_HANDLER(lfqx, 0x1F, 0x17, 0x18, 0x00000001, PPC_POWER2),
GEN_HANDLER(stfq, 0x3C, 0xFF, 0xFF, 0x00000003, PPC_POWER2),
GEN_HANDLER(stfqu, 0x3D, 0xFF, 0xFF, 0x00000003, PPC_POWER2),
GEN_HANDLER(stfqux, 0x1F, 0x17, 0x1D, 0x00000001, PPC_POWER2),
GEN_HANDLER(stfqx, 0x1F, 0x17, 0x1C, 0x00000001, PPC_POWER2),
GEN_HANDLER(mfapidi, 0x1F, 0x13, 0x08, 0x0000F801, PPC_MFAPIDI),
GEN_HANDLER(tlbiva, 0x1F, 0x12, 0x18, 0x03FFF801, PPC_TLBIVA),
GEN_HANDLER(mfdcr, 0x1F, 0x03, 0x0A, 0x00000001, PPC_DCR),
GEN_HANDLER(mtdcr, 0x1F, 0x03, 0x0E, 0x00000001, PPC_DCR),
GEN_HANDLER(mfdcrx, 0x1F, 0x03, 0x08, 0x00000000, PPC_DCRX),
GEN_HANDLER(mtdcrx, 0x1F, 0x03, 0x0C, 0x00000000, PPC_DCRX),
GEN_HANDLER(mfdcrux, 0x1F, 0x03, 0x09, 0x00000000, PPC_DCRUX),
GEN_HANDLER(mtdcrux, 0x1F, 0x03, 0x0D, 0x00000000, PPC_DCRUX),
GEN_HANDLER(dccci, 0x1F, 0x06, 0x0E, 0x03E00001, PPC_4xx_COMMON),
GEN_HANDLER(dcread, 0x1F, 0x06, 0x0F, 0x00000001, PPC_4xx_COMMON),
GEN_HANDLER2(icbt_40x, "icbt", 0x1F, 0x06, 0x08, 0x03E00001, PPC_40x_ICBT),
GEN_HANDLER(iccci, 0x1F, 0x06, 0x1E, 0x00000001, PPC_4xx_COMMON),
GEN_HANDLER(icread, 0x1F, 0x06, 0x1F, 0x03E00001, PPC_4xx_COMMON),
GEN_HANDLER2(rfci_40x, "rfci", 0x13, 0x13, 0x01, 0x03FF8001, PPC_40x_EXCP),
GEN_HANDLER_E(rfci, 0x13, 0x13, 0x01, 0x03FF8001, PPC_BOOKE, PPC2_BOOKE206),
GEN_HANDLER(rfdi, 0x13, 0x07, 0x01, 0x03FF8001, PPC_RFDI),
GEN_HANDLER(rfmci, 0x13, 0x06, 0x01, 0x03FF8001, PPC_RFMCI),
GEN_HANDLER2(tlbre_40x, "tlbre", 0x1F, 0x12, 0x1D, 0x00000001, PPC_40x_TLB),
GEN_HANDLER2(tlbsx_40x, "tlbsx", 0x1F, 0x12, 0x1C, 0x00000000, PPC_40x_TLB),
GEN_HANDLER2(tlbwe_40x, "tlbwe", 0x1F, 0x12, 0x1E, 0x00000001, PPC_40x_TLB),
GEN_HANDLER2(tlbre_440, "tlbre", 0x1F, 0x12, 0x1D, 0x00000001, PPC_BOOKE),
GEN_HANDLER2(tlbsx_440, "tlbsx", 0x1F, 0x12, 0x1C, 0x00000000, PPC_BOOKE),
GEN_HANDLER2(tlbwe_440, "tlbwe", 0x1F, 0x12, 0x1E, 0x00000001, PPC_BOOKE),
GEN_HANDLER2_E(tlbre_booke206, "tlbre", 0x1F, 0x12, 0x1D, 0x00000001,
               PPC_NONE, PPC2_BOOKE206),
GEN_HANDLER2_E(tlbsx_booke206, "tlbsx", 0x1F, 0x12, 0x1C, 0x00000000,
               PPC_NONE, PPC2_BOOKE206),
GEN_HANDLER2_E(tlbwe_booke206, "tlbwe", 0x1F, 0x12, 0x1E, 0x00000001,
               PPC_NONE, PPC2_BOOKE206),
GEN_HANDLER2_E(tlbivax_booke206, "tlbivax", 0x1F, 0x12, 0x18, 0x00000001,
               PPC_NONE, PPC2_BOOKE206),
GEN_HANDLER(wrtee, 0x1F, 0x03, 0x04, 0x000FFC01, PPC_WRTEE),
GEN_HANDLER(wrteei, 0x1F, 0x03, 0x05, 0x000E7C01, PPC_WRTEE),
GEN_HANDLER(dlmzb, 0x1F, 0x0E, 0x02, 0x00000000, PPC_440_SPEC),
GEN_HANDLER_E(mbar, 0x1F, 0x16, 0x1a, 0x001FF801,
              PPC_BOOKE, PPC2_BOOKE206),
GEN_HANDLER_E(msync, 0x1F, 0x16, 0x12, 0x03FFF801,
              PPC_BOOKE, PPC2_BOOKE206),
GEN_HANDLER2_E(icbt_440, "icbt", 0x1F, 0x16, 0x00, 0x03E00001,
               PPC_BOOKE, PPC2_BOOKE206),
GEN_HANDLER(lvsl, 0x1f, 0x06, 0x00, 0x00000001, PPC_ALTIVEC),
GEN_HANDLER(lvsr, 0x1f, 0x06, 0x01, 0x00000001, PPC_ALTIVEC),
GEN_HANDLER(mfvscr, 0x04, 0x2, 0x18, 0x001ff800, PPC_ALTIVEC),
GEN_HANDLER(mtvscr, 0x04, 0x2, 0x19, 0x03ff0000, PPC_ALTIVEC),
GEN_HANDLER(vsldoi, 0x04, 0x16, 0xFF, 0x00000400, PPC_ALTIVEC),
GEN_HANDLER(vmladduhm, 0x04, 0x11, 0xFF, 0x00000000, PPC_ALTIVEC),
GEN_HANDLER2(evsel0, "evsel", 0x04, 0x1c, 0x09, 0x00000000, PPC_SPE),
GEN_HANDLER2(evsel1, "evsel", 0x04, 0x1d, 0x09, 0x00000000, PPC_SPE),
GEN_HANDLER2(evsel2, "evsel", 0x04, 0x1e, 0x09, 0x00000000, PPC_SPE),
GEN_HANDLER2(evsel3, "evsel", 0x04, 0x1f, 0x09, 0x00000000, PPC_SPE),

#undef GEN_INT_ARITH_ADD
#undef GEN_INT_ARITH_ADD_CONST
#define GEN_INT_ARITH_ADD(name, opc3, add_ca, compute_ca, compute_ov)         \
GEN_HANDLER(name, 0x1F, 0x0A, opc3, 0x00000000, PPC_INTEGER),
#define GEN_INT_ARITH_ADD_CONST(name, opc3, const_val,                        \
                                add_ca, compute_ca, compute_ov)               \
GEN_HANDLER(name, 0x1F, 0x0A, opc3, 0x0000F800, PPC_INTEGER),
GEN_INT_ARITH_ADD(add, 0x08, 0, 0, 0)
GEN_INT_ARITH_ADD(addo, 0x18, 0, 0, 1)
GEN_INT_ARITH_ADD(addc, 0x00, 0, 1, 0)
GEN_INT_ARITH_ADD(addco, 0x10, 0, 1, 1)
GEN_INT_ARITH_ADD(adde, 0x04, 1, 1, 0)
GEN_INT_ARITH_ADD(addeo, 0x14, 1, 1, 1)
GEN_INT_ARITH_ADD_CONST(addme, 0x07, -1LL, 1, 1, 0)
GEN_INT_ARITH_ADD_CONST(addmeo, 0x17, -1LL, 1, 1, 1)
GEN_INT_ARITH_ADD_CONST(addze, 0x06, 0, 1, 1, 0)
GEN_INT_ARITH_ADD_CONST(addzeo, 0x16, 0, 1, 1, 1)

#undef GEN_INT_ARITH_DIVW
#define GEN_INT_ARITH_DIVW(name, opc3, sign, compute_ov)                      \
GEN_HANDLER(name, 0x1F, 0x0B, opc3, 0x00000000, PPC_INTEGER)
GEN_INT_ARITH_DIVW(divwu, 0x0E, 0, 0),
GEN_INT_ARITH_DIVW(divwuo, 0x1E, 0, 1),
GEN_INT_ARITH_DIVW(divw, 0x0F, 1, 0),
GEN_INT_ARITH_DIVW(divwo, 0x1F, 1, 1),

#if defined(TARGET_PPC64)
#undef GEN_INT_ARITH_DIVD
#define GEN_INT_ARITH_DIVD(name, opc3, sign, compute_ov)                      \
GEN_HANDLER(name, 0x1F, 0x09, opc3, 0x00000000, PPC_64B)
GEN_INT_ARITH_DIVD(divdu, 0x0E, 0, 0),
GEN_INT_ARITH_DIVD(divduo, 0x1E, 0, 1),
GEN_INT_ARITH_DIVD(divd, 0x0F, 1, 0),
GEN_INT_ARITH_DIVD(divdo, 0x1F, 1, 1),

#undef GEN_INT_ARITH_MUL_HELPER
#define GEN_INT_ARITH_MUL_HELPER(name, opc3)                                  \
GEN_HANDLER(name, 0x1F, 0x09, opc3, 0x00000000, PPC_64B)
GEN_INT_ARITH_MUL_HELPER(mulhdu, 0x00),
GEN_INT_ARITH_MUL_HELPER(mulhd, 0x02),
GEN_INT_ARITH_MUL_HELPER(mulldo, 0x17),
#endif

#undef GEN_INT_ARITH_SUBF
#undef GEN_INT_ARITH_SUBF_CONST
#define GEN_INT_ARITH_SUBF(name, opc3, add_ca, compute_ca, compute_ov)        \
GEN_HANDLER(name, 0x1F, 0x08, opc3, 0x00000000, PPC_INTEGER),
#define GEN_INT_ARITH_SUBF_CONST(name, opc3, const_val,                       \
                                add_ca, compute_ca, compute_ov)               \
GEN_HANDLER(name, 0x1F, 0x08, opc3, 0x0000F800, PPC_INTEGER),
GEN_INT_ARITH_SUBF(subf, 0x01, 0, 0, 0)
GEN_INT_ARITH_SUBF(subfo, 0x11, 0, 0, 1)
GEN_INT_ARITH_SUBF(subfc, 0x00, 0, 1, 0)
GEN_INT_ARITH_SUBF(subfco, 0x10, 0, 1, 1)
GEN_INT_ARITH_SUBF(subfe, 0x04, 1, 1, 0)
GEN_INT_ARITH_SUBF(subfeo, 0x14, 1, 1, 1)
GEN_INT_ARITH_SUBF_CONST(subfme, 0x07, -1LL, 1, 1, 0)
GEN_INT_ARITH_SUBF_CONST(subfmeo, 0x17, -1LL, 1, 1, 1)
GEN_INT_ARITH_SUBF_CONST(subfze, 0x06, 0, 1, 1, 0)
GEN_INT_ARITH_SUBF_CONST(subfzeo, 0x16, 0, 1, 1, 1)

#undef GEN_LOGICAL1
#undef GEN_LOGICAL2
#define GEN_LOGICAL2(name, tcg_op, opc, type)                                 \
GEN_HANDLER(name, 0x1F, 0x1C, opc, 0x00000000, type)
#define GEN_LOGICAL1(name, tcg_op, opc, type)                                 \
GEN_HANDLER(name, 0x1F, 0x1A, opc, 0x00000000, type)
GEN_LOGICAL2(and, tcg_gen_and_tl, 0x00, PPC_INTEGER),
GEN_LOGICAL2(andc, tcg_gen_andc_tl, 0x01, PPC_INTEGER),
GEN_LOGICAL2(eqv, tcg_gen_eqv_tl, 0x08, PPC_INTEGER),
GEN_LOGICAL1(extsb, tcg_gen_ext8s_tl, 0x1D, PPC_INTEGER),
GEN_LOGICAL1(extsh, tcg_gen_ext16s_tl, 0x1C, PPC_INTEGER),
GEN_LOGICAL2(nand, tcg_gen_nand_tl, 0x0E, PPC_INTEGER),
GEN_LOGICAL2(nor, tcg_gen_nor_tl, 0x03, PPC_INTEGER),
GEN_LOGICAL2(orc, tcg_gen_orc_tl, 0x0C, PPC_INTEGER),
#if defined(TARGET_PPC64)
GEN_LOGICAL1(extsw, tcg_gen_ext32s_tl, 0x1E, PPC_64B),
#endif

#if defined(TARGET_PPC64)
#undef GEN_PPC64_R2
#undef GEN_PPC64_R4
#define GEN_PPC64_R2(name, opc1, opc2)                                        \
GEN_HANDLER2(name##0, stringify(name), opc1, opc2, 0xFF, 0x00000000, PPC_64B),\
GEN_HANDLER2(name##1, stringify(name), opc1, opc2 | 0x10, 0xFF, 0x00000000,   \
             PPC_64B)
#define GEN_PPC64_R4(name, opc1, opc2)                                        \
GEN_HANDLER2(name##0, stringify(name), opc1, opc2, 0xFF, 0x00000000, PPC_64B),\
GEN_HANDLER2(name##1, stringify(name), opc1, opc2 | 0x01, 0xFF, 0x00000000,   \
             PPC_64B),                                                        \
GEN_HANDLER2(name##2, stringify(name), opc1, opc2 | 0x10, 0xFF, 0x00000000,   \
             PPC_64B),                                                        \
GEN_HANDLER2(name##3, stringify(name), opc1, opc2 | 0x11, 0xFF, 0x00000000,   \
             PPC_64B)
GEN_PPC64_R4(rldicl, 0x1E, 0x00),
GEN_PPC64_R4(rldicr, 0x1E, 0x02),
GEN_PPC64_R4(rldic, 0x1E, 0x04),
GEN_PPC64_R2(rldcl, 0x1E, 0x08),
GEN_PPC64_R2(rldcr, 0x1E, 0x09),
GEN_PPC64_R4(rldimi, 0x1E, 0x06),
#endif

#undef _GEN_FLOAT_ACB
#undef GEN_FLOAT_ACB
#undef _GEN_FLOAT_AB
#undef GEN_FLOAT_AB
#undef _GEN_FLOAT_AC
#undef GEN_FLOAT_AC
#undef GEN_FLOAT_B
#undef GEN_FLOAT_BS
#define _GEN_FLOAT_ACB(name, op, op1, op2, isfloat, set_fprf, type)           \
GEN_HANDLER(f##name, op1, op2, 0xFF, 0x00000000, type)
#define GEN_FLOAT_ACB(name, op2, set_fprf, type)                              \
_GEN_FLOAT_ACB(name, name, 0x3F, op2, 0, set_fprf, type),                     \
_GEN_FLOAT_ACB(name##s, name, 0x3B, op2, 1, set_fprf, type)
#define _GEN_FLOAT_AB(name, op, op1, op2, inval, isfloat, set_fprf, type)     \
GEN_HANDLER(f##name, op1, op2, 0xFF, inval, type)
#define GEN_FLOAT_AB(name, op2, inval, set_fprf, type)                        \
_GEN_FLOAT_AB(name, name, 0x3F, op2, inval, 0, set_fprf, type),               \
_GEN_FLOAT_AB(name##s, name, 0x3B, op2, inval, 1, set_fprf, type)
#define _GEN_FLOAT_AC(name, op, op1, op2, inval, isfloat, set_fprf, type)     \
GEN_HANDLER(f##name, op1, op2, 0xFF, inval, type)
#define GEN_FLOAT_AC(name, op2, inval, set_fprf, type)                        \
_GEN_FLOAT_AC(name, name, 0x3F, op2, inval, 0, set_fprf, type),               \
_GEN_FLOAT_AC(name##s, name, 0x3B, op2, inval, 1, set_fprf, type)
#define GEN_FLOAT_B(name, op2, op3, set_fprf, type)                           \
GEN_HANDLER(f##name, 0x3F, op2, op3, 0x001F0000, type)
#define GEN_FLOAT_BS(name, op1, op2, set_fprf, type)                          \
GEN_HANDLER(f##name, op1, op2, 0xFF, 0x001F07C0, type)

GEN_FLOAT_AB(add, 0x15, 0x000007C0, 1, PPC_FLOAT),
GEN_FLOAT_AB(div, 0x12, 0x000007C0, 1, PPC_FLOAT),
GEN_FLOAT_AC(mul, 0x19, 0x0000F800, 1, PPC_FLOAT),
GEN_FLOAT_BS(re, 0x3F, 0x18, 1, PPC_FLOAT_EXT),
GEN_FLOAT_BS(res, 0x3B, 0x18, 1, PPC_FLOAT_FRES),
GEN_FLOAT_BS(rsqrte, 0x3F, 0x1A, 1, PPC_FLOAT_FRSQRTE),
_GEN_FLOAT_ACB(sel, sel, 0x3F, 0x17, 0, 0, PPC_FLOAT_FSEL),
GEN_FLOAT_AB(sub, 0x14, 0x000007C0, 1, PPC_FLOAT),
GEN_FLOAT_ACB(madd, 0x1D, 1, PPC_FLOAT),
GEN_FLOAT_ACB(msub, 0x1C, 1, PPC_FLOAT),
GEN_FLOAT_ACB(nmadd, 0x1F, 1, PPC_FLOAT),
GEN_FLOAT_ACB(nmsub, 0x1E, 1, PPC_FLOAT),
GEN_FLOAT_B(ctiw, 0x0E, 0x00, 0, PPC_FLOAT),
GEN_FLOAT_B(ctiwz, 0x0F, 0x00, 0, PPC_FLOAT),
GEN_FLOAT_B(rsp, 0x0C, 0x00, 1, PPC_FLOAT),
#if defined(TARGET_PPC64)
GEN_FLOAT_B(cfid, 0x0E, 0x1A, 1, PPC_64B),
GEN_FLOAT_B(ctid, 0x0E, 0x19, 0, PPC_64B),
GEN_FLOAT_B(ctidz, 0x0F, 0x19, 0, PPC_64B),
#endif
GEN_FLOAT_B(rin, 0x08, 0x0C, 1, PPC_FLOAT_EXT),
GEN_FLOAT_B(riz, 0x08, 0x0D, 1, PPC_FLOAT_EXT),
GEN_FLOAT_B(rip, 0x08, 0x0E, 1, PPC_FLOAT_EXT),
GEN_FLOAT_B(rim, 0x08, 0x0F, 1, PPC_FLOAT_EXT),
GEN_FLOAT_B(abs, 0x08, 0x08, 0, PPC_FLOAT),
GEN_FLOAT_B(nabs, 0x08, 0x04, 0, PPC_FLOAT),
GEN_FLOAT_B(neg, 0x08, 0x01, 0, PPC_FLOAT),

#undef GEN_LD
#undef GEN_LDU
#undef GEN_LDUX
#undef GEN_LDX
#undef GEN_LDS
#define GEN_LD(name, ldop, opc, type)                                         \
GEN_HANDLER(name, opc, 0xFF, 0xFF, 0x00000000, type),
#define GEN_LDU(name, ldop, opc, type)                                        \
GEN_HANDLER(name##u, opc, 0xFF, 0xFF, 0x00000000, type),
#define GEN_LDUX(name, ldop, opc2, opc3, type)                                \
GEN_HANDLER(name##ux, 0x1F, opc2, opc3, 0x00000001, type),
#define GEN_LDX(name, ldop, opc2, opc3, type)                                 \
GEN_HANDLER(name##x, 0x1F, opc2, opc3, 0x00000001, type),
#define GEN_LDS(name, ldop, op, type)                                         \
GEN_LD(name, ldop, op | 0x20, type)                                           \
GEN_LDU(name, ldop, op | 0x21, type)                                          \
GEN_LDUX(name, ldop, 0x17, op | 0x01, type)                                   \
GEN_LDX(name, ldop, 0x17, op | 0x00, type)

GEN_LDS(lbz, ld8u, 0x02, PPC_INTEGER)
GEN_LDS(lha, ld16s, 0x0A, PPC_INTEGER)
GEN_LDS(lhz, ld16u, 0x08, PPC_INTEGER)
GEN_LDS(lwz, ld32u, 0x00, PPC_INTEGER)
#if defined(TARGET_PPC64)
GEN_LDUX(lwa, ld32s, 0x15, 0x0B, PPC_64B)
GEN_LDX(lwa, ld32s, 0x15, 0x0A, PPC_64B)
GEN_LDUX(ld, ld64, 0x15, 0x01, PPC_64B)
GEN_LDX(ld, ld64, 0x15, 0x00, PPC_64B)
#endif
GEN_LDX(lhbr, ld16ur, 0x16, 0x18, PPC_INTEGER)
GEN_LDX(lwbr, ld32ur, 0x16, 0x10, PPC_INTEGER)

#undef GEN_ST
#undef GEN_STU
#undef GEN_STUX
#undef GEN_STX
#undef GEN_STS
#define GEN_ST(name, stop, opc, type)                                         \
GEN_HANDLER(name, opc, 0xFF, 0xFF, 0x00000000, type),
#define GEN_STU(name, stop, opc, type)                                        \
GEN_HANDLER(stop##u, opc, 0xFF, 0xFF, 0x00000000, type),
#define GEN_STUX(name, stop, opc2, opc3, type)                                \
GEN_HANDLER(name##ux, 0x1F, opc2, opc3, 0x00000001, type),
#define GEN_STX(name, stop, opc2, opc3, type)                                 \
GEN_HANDLER(name##x, 0x1F, opc2, opc3, 0x00000001, type),
#define GEN_STS(name, stop, op, type)                                         \
GEN_ST(name, stop, op | 0x20, type)                                           \
GEN_STU(name, stop, op | 0x21, type)                                          \
GEN_STUX(name, stop, 0x17, op | 0x01, type)                                   \
GEN_STX(name, stop, 0x17, op | 0x00, type)

GEN_STS(stb, st8, 0x06, PPC_INTEGER)
GEN_STS(sth, st16, 0x0C, PPC_INTEGER)
GEN_STS(stw, st32, 0x04, PPC_INTEGER)
#if defined(TARGET_PPC64)
GEN_STUX(std, st64, 0x15, 0x05, PPC_64B)
GEN_STX(std, st64, 0x15, 0x04, PPC_64B)
#endif
GEN_STX(sthbr, st16r, 0x16, 0x1C, PPC_INTEGER)
GEN_STX(stwbr, st32r, 0x16, 0x14, PPC_INTEGER)

#undef GEN_LDF
#undef GEN_LDUF
#undef GEN_LDUXF
#undef GEN_LDXF
#undef GEN_LDFS
#define GEN_LDF(name, ldop, opc, type)                                        \
GEN_HANDLER(name, opc, 0xFF, 0xFF, 0x00000000, type),
#define GEN_LDUF(name, ldop, opc, type)                                       \
GEN_HANDLER(name##u, opc, 0xFF, 0xFF, 0x00000000, type),
#define GEN_LDUXF(name, ldop, opc, type)                                      \
GEN_HANDLER(name##ux, 0x1F, 0x17, opc, 0x00000001, type),
#define GEN_LDXF(name, ldop, opc2, opc3, type)                                \
GEN_HANDLER(name##x, 0x1F, opc2, opc3, 0x00000001, type),
#define GEN_LDFS(name, ldop, op, type)                                        \
GEN_LDF(name, ldop, op | 0x20, type)                                          \
GEN_LDUF(name, ldop, op | 0x21, type)                                         \
GEN_LDUXF(name, ldop, op | 0x01, type)                                        \
GEN_LDXF(name, ldop, 0x17, op | 0x00, type)

GEN_LDFS(lfd, ld64, 0x12, PPC_FLOAT)
GEN_LDFS(lfs, ld32fs, 0x10, PPC_FLOAT)

#undef GEN_STF
#undef GEN_STUF
#undef GEN_STUXF
#undef GEN_STXF
#undef GEN_STFS
#define GEN_STF(name, stop, opc, type)                                        \
GEN_HANDLER(name, opc, 0xFF, 0xFF, 0x00000000, type),
#define GEN_STUF(name, stop, opc, type)                                       \
GEN_HANDLER(name##u, opc, 0xFF, 0xFF, 0x00000000, type),
#define GEN_STUXF(name, stop, opc, type)                                      \
GEN_HANDLER(name##ux, 0x1F, 0x17, opc, 0x00000001, type),
#define GEN_STXF(name, stop, opc2, opc3, type)                                \
GEN_HANDLER(name##x, 0x1F, opc2, opc3, 0x00000001, type),
#define GEN_STFS(name, stop, op, type)                                        \
GEN_STF(name, stop, op | 0x20, type)                                          \
GEN_STUF(name, stop, op | 0x21, type)                                         \
GEN_STUXF(name, stop, op | 0x01, type)                                        \
GEN_STXF(name, stop, 0x17, op | 0x00, type)

GEN_STFS(stfd, st64, 0x16, PPC_FLOAT)
GEN_STFS(stfs, st32fs, 0x14, PPC_FLOAT)
GEN_STXF(stfiw, st32fiw, 0x17, 0x1E, PPC_FLOAT_STFIWX)

#undef GEN_CRLOGIC
#define GEN_CRLOGIC(name, tcg_op, opc)                                        \
GEN_HANDLER(name, 0x13, 0x01, opc, 0x00000001, PPC_INTEGER)
GEN_CRLOGIC(crand, tcg_gen_and_i32, 0x08),
GEN_CRLOGIC(crandc, tcg_gen_andc_i32, 0x04),
GEN_CRLOGIC(creqv, tcg_gen_eqv_i32, 0x09),
GEN_CRLOGIC(crnand, tcg_gen_nand_i32, 0x07),
GEN_CRLOGIC(crnor, tcg_gen_nor_i32, 0x01),
GEN_CRLOGIC(cror, tcg_gen_or_i32, 0x0E),
GEN_CRLOGIC(crorc, tcg_gen_orc_i32, 0x0D),
GEN_CRLOGIC(crxor, tcg_gen_xor_i32, 0x06),

#undef GEN_MAC_HANDLER
#define GEN_MAC_HANDLER(name, opc2, opc3)                                     \
GEN_HANDLER(name, 0x04, opc2, opc3, 0x00000000, PPC_405_MAC)
GEN_MAC_HANDLER(macchw, 0x0C, 0x05),
GEN_MAC_HANDLER(macchwo, 0x0C, 0x15),
GEN_MAC_HANDLER(macchws, 0x0C, 0x07),
GEN_MAC_HANDLER(macchwso, 0x0C, 0x17),
GEN_MAC_HANDLER(macchwsu, 0x0C, 0x06),
GEN_MAC_HANDLER(macchwsuo, 0x0C, 0x16),
GEN_MAC_HANDLER(macchwu, 0x0C, 0x04),
GEN_MAC_HANDLER(macchwuo, 0x0C, 0x14),
GEN_MAC_HANDLER(machhw, 0x0C, 0x01),
GEN_MAC_HANDLER(machhwo, 0x0C, 0x11),
GEN_MAC_HANDLER(machhws, 0x0C, 0x03),
GEN_MAC_HANDLER(machhwso, 0x0C, 0x13),
GEN_MAC_HANDLER(machhwsu, 0x0C, 0x02),
GEN_MAC_HANDLER(machhwsuo, 0x0C, 0x12),
GEN_MAC_HANDLER(machhwu, 0x0C, 0x00),
GEN_MAC_HANDLER(machhwuo, 0x0C, 0x10),
GEN_MAC_HANDLER(maclhw, 0x0C, 0x0D),
GEN_MAC_HANDLER(maclhwo, 0x0C, 0x1D),
GEN_MAC_HANDLER(maclhws, 0x0C, 0x0F),
GEN_MAC_HANDLER(maclhwso, 0x0C, 0x1F),
GEN_MAC_HANDLER(maclhwu, 0x0C, 0x0C),
GEN_MAC_HANDLER(maclhwuo, 0x0C, 0x1C),
GEN_MAC_HANDLER(maclhwsu, 0x0C, 0x0E),
GEN_MAC_HANDLER(maclhwsuo, 0x0C, 0x1E),
GEN_MAC_HANDLER(nmacchw, 0x0E, 0x05),
GEN_MAC_HANDLER(nmacchwo, 0x0E, 0x15),
GEN_MAC_HANDLER(nmacchws, 0x0E, 0x07),
GEN_MAC_HANDLER(nmacchwso, 0x0E, 0x17),
GEN_MAC_HANDLER(nmachhw, 0x0E, 0x01),
GEN_MAC_HANDLER(nmachhwo, 0x0E, 0x11),
GEN_MAC_HANDLER(nmachhws, 0x0E, 0x03),
GEN_MAC_HANDLER(nmachhwso, 0x0E, 0x13),
GEN_MAC_HANDLER(nmaclhw, 0x0E, 0x0D),
GEN_MAC_HANDLER(nmaclhwo, 0x0E, 0x1D),
GEN_MAC_HANDLER(nmaclhws, 0x0E, 0x0F),
GEN_MAC_HANDLER(nmaclhwso, 0x0E, 0x1F),
GEN_MAC_HANDLER(mulchw, 0x08, 0x05),
GEN_MAC_HANDLER(mulchwu, 0x08, 0x04),
GEN_MAC_HANDLER(mulhhw, 0x08, 0x01),
GEN_MAC_HANDLER(mulhhwu, 0x08, 0x00),
GEN_MAC_HANDLER(mullhw, 0x08, 0x0D),
GEN_MAC_HANDLER(mullhwu, 0x08, 0x0C),

#undef GEN_VR_LDX
#undef GEN_VR_STX
#undef GEN_VR_LVE
#undef GEN_VR_STVE
#define GEN_VR_LDX(name, opc2, opc3)                                          \
GEN_HANDLER(name, 0x1F, opc2, opc3, 0x00000001, PPC_ALTIVEC)
#define GEN_VR_STX(name, opc2, opc3)                                          \
GEN_HANDLER(st##name, 0x1F, opc2, opc3, 0x00000001, PPC_ALTIVEC)
#define GEN_VR_LVE(name, opc2, opc3)                                          \
    GEN_HANDLER(lve##name, 0x1F, opc2, opc3, 0x00000001, PPC_ALTIVEC)
#define GEN_VR_STVE(name, opc2, opc3)                                         \
    GEN_HANDLER(stve##name, 0x1F, opc2, opc3, 0x00000001, PPC_ALTIVEC)
GEN_VR_LDX(lvx, 0x07, 0x03),
GEN_VR_LDX(lvxl, 0x07, 0x0B),
GEN_VR_LVE(bx, 0x07, 0x00),
GEN_VR_LVE(hx, 0x07, 0x01),
GEN_VR_LVE(wx, 0x07, 0x02),
GEN_VR_STX(svx, 0x07, 0x07),
GEN_VR_STX(svxl, 0x07, 0x0F),
GEN_VR_STVE(bx, 0x07, 0x04),
GEN_VR_STVE(hx, 0x07, 0x05),
GEN_VR_STVE(wx, 0x07, 0x06),

#undef GEN_VX_LOGICAL
#define GEN_VX_LOGICAL(name, tcg_op, opc2, opc3)                              \
GEN_HANDLER(name, 0x04, opc2, opc3, 0x00000000, PPC_ALTIVEC)
GEN_VX_LOGICAL(vand, tcg_gen_and_i64, 2, 16),
GEN_VX_LOGICAL(vandc, tcg_gen_andc_i64, 2, 17),
GEN_VX_LOGICAL(vor, tcg_gen_or_i64, 2, 18),
GEN_VX_LOGICAL(vxor, tcg_gen_xor_i64, 2, 19),
GEN_VX_LOGICAL(vnor, tcg_gen_nor_i64, 2, 20),

#undef GEN_VXFORM
#define GEN_VXFORM(name, opc2, opc3)                                          \
GEN_HANDLER(name, 0x04, opc2, opc3, 0x00000000, PPC_ALTIVEC)
GEN_VXFORM(vaddubm, 0, 0),
GEN_VXFORM(vadduhm, 0, 1),
GEN_VXFORM(vadduwm, 0, 2),
GEN_VXFORM(vsububm, 0, 16),
GEN_VXFORM(vsubuhm, 0, 17),
GEN_VXFORM(vsubuwm, 0, 18),
GEN_VXFORM(vmaxub, 1, 0),
GEN_VXFORM(vmaxuh, 1, 1),
GEN_VXFORM(vmaxuw, 1, 2),
GEN_VXFORM(vmaxsb, 1, 4),
GEN_VXFORM(vmaxsh, 1, 5),
GEN_VXFORM(vmaxsw, 1, 6),
GEN_VXFORM(vminub, 1, 8),
GEN_VXFORM(vminuh, 1, 9),
GEN_VXFORM(vminuw, 1, 10),
GEN_VXFORM(vminsb, 1, 12),
GEN_VXFORM(vminsh, 1, 13),
GEN_VXFORM(vminsw, 1, 14),
GEN_VXFORM(vavgub, 1, 16),
GEN_VXFORM(vavguh, 1, 17),
GEN_VXFORM(vavguw, 1, 18),
GEN_VXFORM(vavgsb, 1, 20),
GEN_VXFORM(vavgsh, 1, 21),
GEN_VXFORM(vavgsw, 1, 22),
GEN_VXFORM(vmrghb, 6, 0),
GEN_VXFORM(vmrghh, 6, 1),
GEN_VXFORM(vmrghw, 6, 2),
GEN_VXFORM(vmrglb, 6, 4),
GEN_VXFORM(vmrglh, 6, 5),
GEN_VXFORM(vmrglw, 6, 6),
GEN_VXFORM(vmuloub, 4, 0),
GEN_VXFORM(vmulouh, 4, 1),
GEN_VXFORM(vmulosb, 4, 4),
GEN_VXFORM(vmulosh, 4, 5),
GEN_VXFORM(vmuleub, 4, 8),
GEN_VXFORM(vmuleuh, 4, 9),
GEN_VXFORM(vmulesb, 4, 12),
GEN_VXFORM(vmulesh, 4, 13),
GEN_VXFORM(vslb, 2, 4),
GEN_VXFORM(vslh, 2, 5),
GEN_VXFORM(vslw, 2, 6),
GEN_VXFORM(vsrb, 2, 8),
GEN_VXFORM(vsrh, 2, 9),
GEN_VXFORM(vsrw, 2, 10),
GEN_VXFORM(vsrab, 2, 12),
GEN_VXFORM(vsrah, 2, 13),
GEN_VXFORM(vsraw, 2, 14),
GEN_VXFORM(vslo, 6, 16),
GEN_VXFORM(vsro, 6, 17),
GEN_VXFORM(vaddcuw, 0, 6),
GEN_VXFORM(vsubcuw, 0, 22),
GEN_VXFORM(vaddubs, 0, 8),
GEN_VXFORM(vadduhs, 0, 9),
GEN_VXFORM(vadduws, 0, 10),
GEN_VXFORM(vaddsbs, 0, 12),
GEN_VXFORM(vaddshs, 0, 13),
GEN_VXFORM(vaddsws, 0, 14),
GEN_VXFORM(vsububs, 0, 24),
GEN_VXFORM(vsubuhs, 0, 25),
GEN_VXFORM(vsubuws, 0, 26),
GEN_VXFORM(vsubsbs, 0, 28),
GEN_VXFORM(vsubshs, 0, 29),
GEN_VXFORM(vsubsws, 0, 30),
GEN_VXFORM(vrlb, 2, 0),
GEN_VXFORM(vrlh, 2, 1),
GEN_VXFORM(vrlw, 2, 2),
GEN_VXFORM(vsl, 2, 7),
GEN_VXFORM(vsr, 2, 11),
GEN_VXFORM(vpkuhum, 7, 0),
GEN_VXFORM(vpkuwum, 7, 1),
GEN_VXFORM(vpkuhus, 7, 2),
GEN_VXFORM(vpkuwus, 7, 3),
GEN_VXFORM(vpkshus, 7, 4),
GEN_VXFORM(vpkswus, 7, 5),
GEN_VXFORM(vpkshss, 7, 6),
GEN_VXFORM(vpkswss, 7, 7),
GEN_VXFORM(vpkpx, 7, 12),
GEN_VXFORM(vsum4ubs, 4, 24),
GEN_VXFORM(vsum4sbs, 4, 28),
GEN_VXFORM(vsum4shs, 4, 25),
GEN_VXFORM(vsum2sws, 4, 26),
GEN_VXFORM(vsumsws, 4, 30),
GEN_VXFORM(vaddfp, 5, 0),
GEN_VXFORM(vsubfp, 5, 1),
GEN_VXFORM(vmaxfp, 5, 16),
GEN_VXFORM(vminfp, 5, 17),

#undef GEN_VXRFORM1
#undef GEN_VXRFORM
#define GEN_VXRFORM1(opname, name, str, opc2, opc3)                            \
    GEN_HANDLER2(name, str, 0x4, opc2, opc3, 0x00000000, PPC_ALTIVEC),
#define GEN_VXRFORM(name, opc2, opc3)                                          \
    GEN_VXRFORM1(name, name, #name, opc2, opc3)                                \
    GEN_VXRFORM1(name##_dot, name##_, #name ".", opc2, (opc3 | (0x1 << 4)))
GEN_VXRFORM(vcmpequb, 3, 0)
GEN_VXRFORM(vcmpequh, 3, 1)
GEN_VXRFORM(vcmpequw, 3, 2)
GEN_VXRFORM(vcmpgtsb, 3, 12)
GEN_VXRFORM(vcmpgtsh, 3, 13)
GEN_VXRFORM(vcmpgtsw, 3, 14)
GEN_VXRFORM(vcmpgtub, 3, 8)
GEN_VXRFORM(vcmpgtuh, 3, 9)
GEN_VXRFORM(vcmpgtuw, 3, 10)
GEN_VXRFORM(vcmpeqfp, 3, 3)
GEN_VXRFORM(vcmpgefp, 3, 7)
GEN_VXRFORM(vcmpgtfp, 3, 11)
GEN_VXRFORM(vcmpbfp, 3, 15)

#undef GEN_VXFORM_SIMM
#define GEN_VXFORM_SIMM(name, opc2, opc3)                                      \
    GEN_HANDLER(name, 0x04, opc2, opc3, 0x00000000, PPC_ALTIVEC)
GEN_VXFORM_SIMM(vspltisb, 6, 12),
GEN_VXFORM_SIMM(vspltish, 6, 13),
GEN_VXFORM_SIMM(vspltisw, 6, 14),

#undef GEN_VXFORM_NOA
#define GEN_VXFORM_NOA(name, opc2, opc3)                                       \
    GEN_HANDLER(name, 0x04, opc2, opc3, 0x001f0000, PPC_ALTIVEC)
GEN_VXFORM_NOA(vupkhsb, 7, 8),
GEN_VXFORM_NOA(vupkhsh, 7, 9),
GEN_VXFORM_NOA(vupklsb, 7, 10),
GEN_VXFORM_NOA(vupklsh, 7, 11),
GEN_VXFORM_NOA(vupkhpx, 7, 13),
GEN_VXFORM_NOA(vupklpx, 7, 15),
GEN_VXFORM_NOA(vrefp, 5, 4),
GEN_VXFORM_NOA(vrsqrtefp, 5, 5),
GEN_VXFORM_NOA(vexptefp, 5, 6),
GEN_VXFORM_NOA(vlogefp, 5, 7),
GEN_VXFORM_NOA(vrfim, 5, 8),
GEN_VXFORM_NOA(vrfin, 5, 9),
GEN_VXFORM_NOA(vrfip, 5, 10),
GEN_VXFORM_NOA(vrfiz, 5, 11),

#undef GEN_VXFORM_UIMM
#define GEN_VXFORM_UIMM(name, opc2, opc3)                                      \
    GEN_HANDLER(name, 0x04, opc2, opc3, 0x00000000, PPC_ALTIVEC)
GEN_VXFORM_UIMM(vspltb, 6, 8),
GEN_VXFORM_UIMM(vsplth, 6, 9),
GEN_VXFORM_UIMM(vspltw, 6, 10),
GEN_VXFORM_UIMM(vcfux, 5, 12),
GEN_VXFORM_UIMM(vcfsx, 5, 13),
GEN_VXFORM_UIMM(vctuxs, 5, 14),
GEN_VXFORM_UIMM(vctsxs, 5, 15),

#undef GEN_VAFORM_PAIRED
#define GEN_VAFORM_PAIRED(name0, name1, opc2)                                  \
    GEN_HANDLER(name0##_##name1, 0x04, opc2, 0xFF, 0x00000000, PPC_ALTIVEC)
GEN_VAFORM_PAIRED(vmhaddshs, vmhraddshs, 16),
GEN_VAFORM_PAIRED(vmsumubm, vmsummbm, 18),
GEN_VAFORM_PAIRED(vmsumuhm, vmsumuhs, 19),
GEN_VAFORM_PAIRED(vmsumshm, vmsumshs, 20),
GEN_VAFORM_PAIRED(vsel, vperm, 21),
GEN_VAFORM_PAIRED(vmaddfp, vnmsubfp, 23),

#undef GEN_SPE
#define GEN_SPE(name0, name1, opc2, opc3, inval0, inval1, type)                \
    GEN_OPCODE_DUAL(name0##_##name1, 0x04, opc2, opc3, inval0, inval1, type, PPC_NONE, 4)
GEN_SPE(evaddw,      speundef,    0x00, 0x08, 0x00000000, 0xFFFFFFFF, PPC_SPE),
GEN_SPE(evaddiw,     speundef,    0x01, 0x08, 0x00000000, 0xFFFFFFFF, PPC_SPE),
GEN_SPE(evsubfw,     speundef,    0x02, 0x08, 0x00000000, 0xFFFFFFFF, PPC_SPE),
GEN_SPE(evsubifw,    speundef,    0x03, 0x08, 0x00000000, 0xFFFFFFFF, PPC_SPE),
GEN_SPE(evabs,       evneg,       0x04, 0x08, 0x0000F800, 0x0000F800, PPC_SPE),
GEN_SPE(evextsb,     evextsh,     0x05, 0x08, 0x0000F800, 0x0000F800, PPC_SPE),
GEN_SPE(evrndw,      evcntlzw,    0x06, 0x08, 0x0000F800, 0x0000F800, PPC_SPE),
GEN_SPE(evcntlsw,    brinc,       0x07, 0x08, 0x0000F800, 0x00000000, PPC_SPE),
GEN_SPE(evmra,       speundef,    0x02, 0x13, 0x0000F800, 0xFFFFFFFF, PPC_SPE),
GEN_SPE(speundef,    evand,       0x08, 0x08, 0xFFFFFFFF, 0x00000000, PPC_SPE),
GEN_SPE(evandc,      speundef,    0x09, 0x08, 0x00000000, 0xFFFFFFFF, PPC_SPE),
GEN_SPE(evxor,       evor,        0x0B, 0x08, 0x00000000, 0x00000000, PPC_SPE),
GEN_SPE(evnor,       eveqv,       0x0C, 0x08, 0x00000000, 0x00000000, PPC_SPE),
GEN_SPE(evmwumi,     evmwsmi,     0x0C, 0x11, 0x00000000, 0x00000000, PPC_SPE),
GEN_SPE(evmwumia,    evmwsmia,    0x1C, 0x11, 0x00000000, 0x00000000, PPC_SPE),
GEN_SPE(evmwumiaa,   evmwsmiaa,   0x0C, 0x15, 0x00000000, 0x00000000, PPC_SPE),
GEN_SPE(speundef,    evorc,       0x0D, 0x08, 0xFFFFFFFF, 0x00000000, PPC_SPE),
GEN_SPE(evnand,      speundef,    0x0F, 0x08, 0x00000000, 0xFFFFFFFF, PPC_SPE),
GEN_SPE(evsrwu,      evsrws,      0x10, 0x08, 0x00000000, 0x00000000, PPC_SPE),
GEN_SPE(evsrwiu,     evsrwis,     0x11, 0x08, 0x00000000, 0x00000000, PPC_SPE),
GEN_SPE(evslw,       speundef,    0x12, 0x08, 0x00000000, 0xFFFFFFFF, PPC_SPE),
GEN_SPE(evslwi,      speundef,    0x13, 0x08, 0x00000000, 0xFFFFFFFF, PPC_SPE),
GEN_SPE(evrlw,       evsplati,    0x14, 0x08, 0x00000000, 0x0000F800, PPC_SPE),
GEN_SPE(evrlwi,      evsplatfi,   0x15, 0x08, 0x00000000, 0x0000F800, PPC_SPE),
GEN_SPE(evmergehi,   evmergelo,   0x16, 0x08, 0x00000000, 0x00000000, PPC_SPE),
GEN_SPE(evmergehilo, evmergelohi, 0x17, 0x08, 0x00000000, 0x00000000, PPC_SPE),
GEN_SPE(evcmpgtu,    evcmpgts,    0x18, 0x08, 0x00600000, 0x00600000, PPC_SPE),
GEN_SPE(evcmpltu,    evcmplts,    0x19, 0x08, 0x00600000, 0x00600000, PPC_SPE),
GEN_SPE(evcmpeq,     speundef,    0x1A, 0x08, 0x00600000, 0xFFFFFFFF, PPC_SPE),

GEN_SPE(evfsadd,     evfssub,     0x00, 0x0A, 0x00000000, 0x00000000, PPC_SPE_SINGLE),
GEN_SPE(evfsabs,     evfsnabs,    0x02, 0x0A, 0x0000F800, 0x0000F800, PPC_SPE_SINGLE),
GEN_SPE(evfsneg,     speundef,    0x03, 0x0A, 0x0000F800, 0xFFFFFFFF, PPC_SPE_SINGLE),
GEN_SPE(evfsmul,     evfsdiv,     0x04, 0x0A, 0x00000000, 0x00000000, PPC_SPE_SINGLE),
GEN_SPE(evfscmpgt,   evfscmplt,   0x06, 0x0A, 0x00600000, 0x00600000, PPC_SPE_SINGLE),
GEN_SPE(evfscmpeq,   speundef,    0x07, 0x0A, 0x00600000, 0xFFFFFFFF, PPC_SPE_SINGLE),
GEN_SPE(evfscfui,    evfscfsi,    0x08, 0x0A, 0x00180000, 0x00180000, PPC_SPE_SINGLE),
GEN_SPE(evfscfuf,    evfscfsf,    0x09, 0x0A, 0x00180000, 0x00180000, PPC_SPE_SINGLE),
GEN_SPE(evfsctui,    evfsctsi,    0x0A, 0x0A, 0x00180000, 0x00180000, PPC_SPE_SINGLE),
GEN_SPE(evfsctuf,    evfsctsf,    0x0B, 0x0A, 0x00180000, 0x00180000, PPC_SPE_SINGLE),
GEN_SPE(evfsctuiz,   speundef,    0x0C, 0x0A, 0x00180000, 0xFFFFFFFF, PPC_SPE_SINGLE),
GEN_SPE(evfsctsiz,   speundef,    0x0D, 0x0A, 0x00180000, 0xFFFFFFFF, PPC_SPE_SINGLE),
GEN_SPE(evfststgt,   evfststlt,   0x0E, 0x0A, 0x00600000, 0x00600000, PPC_SPE_SINGLE),
GEN_SPE(evfststeq,   speundef,    0x0F, 0x0A, 0x00600000, 0xFFFFFFFF, PPC_SPE_SINGLE),

GEN_SPE(efsadd,      efssub,      0x00, 0x0B, 0x00000000, 0x00000000, PPC_SPE_SINGLE),
GEN_SPE(efsabs,      efsnabs,     0x02, 0x0B, 0x0000F800, 0x0000F800, PPC_SPE_SINGLE),
GEN_SPE(efsneg,      speundef,    0x03, 0x0B, 0x0000F800, 0xFFFFFFFF, PPC_SPE_SINGLE),
GEN_SPE(efsmul,      efsdiv,      0x04, 0x0B, 0x00000000, 0x00000000, PPC_SPE_SINGLE),
GEN_SPE(efscmpgt,    efscmplt,    0x06, 0x0B, 0x00600000, 0x00600000, PPC_SPE_SINGLE),
GEN_SPE(efscmpeq,    efscfd,      0x07, 0x0B, 0x00600000, 0x00180000, PPC_SPE_SINGLE),
GEN_SPE(efscfui,     efscfsi,     0x08, 0x0B, 0x00180000, 0x00180000, PPC_SPE_SINGLE),
GEN_SPE(efscfuf,     efscfsf,     0x09, 0x0B, 0x00180000, 0x00180000, PPC_SPE_SINGLE),
GEN_SPE(efsctui,     efsctsi,     0x0A, 0x0B, 0x00180000, 0x00180000, PPC_SPE_SINGLE),
GEN_SPE(efsctuf,     efsctsf,     0x0B, 0x0B, 0x00180000, 0x00180000, PPC_SPE_SINGLE),
GEN_SPE(efsctuiz,    speundef,    0x0C, 0x0B, 0x00180000, 0xFFFFFFFF, PPC_SPE_SINGLE),
GEN_SPE(efsctsiz,    speundef,    0x0D, 0x0B, 0x00180000, 0xFFFFFFFF, PPC_SPE_SINGLE),
GEN_SPE(efststgt,    efststlt,    0x0E, 0x0B, 0x00600000, 0x00600000, PPC_SPE_SINGLE),
GEN_SPE(efststeq,    speundef,    0x0F, 0x0B, 0x00600000, 0xFFFFFFFF, PPC_SPE_SINGLE),

GEN_SPE(efdadd,      efdsub,      0x10, 0x0B, 0x00000000, 0x00000000, PPC_SPE_DOUBLE),
GEN_SPE(efdcfuid,    efdcfsid,    0x11, 0x0B, 0x00180000, 0x00180000, PPC_SPE_DOUBLE),
GEN_SPE(efdabs,      efdnabs,     0x12, 0x0B, 0x0000F800, 0x0000F800, PPC_SPE_DOUBLE),
GEN_SPE(efdneg,      speundef,    0x13, 0x0B, 0x0000F800, 0xFFFFFFFF, PPC_SPE_DOUBLE),
GEN_SPE(efdmul,      efddiv,      0x14, 0x0B, 0x00000000, 0x00000000, PPC_SPE_DOUBLE),
GEN_SPE(efdctuidz,   efdctsidz,   0x15, 0x0B, 0x00180000, 0x00180000, PPC_SPE_DOUBLE),
GEN_SPE(efdcmpgt,    efdcmplt,    0x16, 0x0B, 0x00600000, 0x00600000, PPC_SPE_DOUBLE),
GEN_SPE(efdcmpeq,    efdcfs,      0x17, 0x0B, 0x00600000, 0x00180000, PPC_SPE_DOUBLE),
GEN_SPE(efdcfui,     efdcfsi,     0x18, 0x0B, 0x00180000, 0x00180000, PPC_SPE_DOUBLE),
GEN_SPE(efdcfuf,     efdcfsf,     0x19, 0x0B, 0x00180000, 0x00180000, PPC_SPE_DOUBLE),
GEN_SPE(efdctui,     efdctsi,     0x1A, 0x0B, 0x00180000, 0x00180000, PPC_SPE_DOUBLE),
GEN_SPE(efdctuf,     efdctsf,     0x1B, 0x0B, 0x00180000, 0x00180000, PPC_SPE_DOUBLE),
GEN_SPE(efdctuiz,    speundef,    0x1C, 0x0B, 0x00180000, 0xFFFFFFFF, PPC_SPE_DOUBLE),
GEN_SPE(efdctsiz,    speundef,    0x1D, 0x0B, 0x00180000, 0xFFFFFFFF, PPC_SPE_DOUBLE),
GEN_SPE(efdtstgt,    efdtstlt,    0x1E, 0x0B, 0x00600000, 0x00600000, PPC_SPE_DOUBLE),
GEN_SPE(efdtsteq,    speundef,    0x1F, 0x0B, 0x00600000, 0xFFFFFFFF, PPC_SPE_DOUBLE),

#undef GEN_SPEOP_LDST
#define GEN_SPEOP_LDST(name, opc2, sh)                                         \
GEN_HANDLER(name, 0x04, opc2, 0x0C, 0x00000000, PPC_SPE)
GEN_SPEOP_LDST(evldd, 0x00, 3),
GEN_SPEOP_LDST(evldw, 0x01, 3),
GEN_SPEOP_LDST(evldh, 0x02, 3),
GEN_SPEOP_LDST(evlhhesplat, 0x04, 1),
GEN_SPEOP_LDST(evlhhousplat, 0x06, 1),
GEN_SPEOP_LDST(evlhhossplat, 0x07, 1),
GEN_SPEOP_LDST(evlwhe, 0x08, 2),
GEN_SPEOP_LDST(evlwhou, 0x0A, 2),
GEN_SPEOP_LDST(evlwhos, 0x0B, 2),
GEN_SPEOP_LDST(evlwwsplat, 0x0C, 2),
GEN_SPEOP_LDST(evlwhsplat, 0x0E, 2),

GEN_SPEOP_LDST(evstdd, 0x10, 3),
GEN_SPEOP_LDST(evstdw, 0x11, 3),
GEN_SPEOP_LDST(evstdh, 0x12, 3),
GEN_SPEOP_LDST(evstwhe, 0x18, 2),
GEN_SPEOP_LDST(evstwho, 0x1A, 2),
GEN_SPEOP_LDST(evstwwe, 0x1C, 2),
GEN_SPEOP_LDST(evstwwo, 0x1E, 2),
};

// clang-format on

EXTRACT_HELPER(RR_RY, 20, 4);
EXTRACT_HELPER(RR_RX, 16, 4);

static inline uint32_t I16A_SI(uint32_t opcode)
{
    return ((opcode >> 10) & (((1 << 5) - 1) << 11)) | (opcode & ((1 << 11) - 1));
}
EXTRACT_HELPER(I16A_RA, 16, 5);

EXTRACT_HELPER(D_RD, 21, 5);
EXTRACT_HELPER(D_RA, 16, 5);
EXTRACT_HELPER(D_SI, 0, 16);

EXTRACT_HELPER(SCI8_RD, 21, 5);
EXTRACT_HELPER(SCI8_RA, 16, 5);
EXTRACT_HELPER(SCI8_RC, 11, 1);
EXTRACT_HELPER(SCI8_F, 10, 1);
EXTRACT_HELPER(SCI8_SCL, 8, 2);
EXTRACT_HELPER(SCI8_UI8, 0, 8);

static target_long SCI8(uint32_t opcode)
{
    uint32_t f = SCI8_F(opcode);
    uint32_t scl = SCI8_SCL(opcode);
    uint32_t ui8 = SCI8_UI8(opcode);
    target_long shift_value = (ui8 << (8 * scl));

    if(f) {
        switch(scl) {
            case 0:
                return shift_value | 0xFFFFFF00;
            case 1:
                return shift_value | 0xFFFF00FF;
            case 2:
                return shift_value | 0xFF00FFFF;
            default:
                return shift_value | 0x00FFFFFF;
        }
    }
    return shift_value;
}

static void gen_se_add(DisasContext *dc)
{
    gen_op_arith_add(dc, cpu_gpr[RR_RX(dc->opcode)], cpu_gpr[RR_RY(dc->opcode)], cpu_gpr[RR_RX(dc->opcode)], 0, 0, 0);
}

static void gen_se_addi(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_add16i(DisasContext *dc)
{
    tcg_gen_addi_tl(cpu_gpr[D_RD(dc->opcode)], cpu_gpr[D_RA(dc->opcode)], D_SI(dc->opcode));
}

static void gen_e_add2i(DisasContext *dc)
{
    tcg_gen_addi_tl(cpu_gpr[I16A_RA(dc->opcode)], cpu_gpr[I16A_RA(dc->opcode)], I16A_SI(dc->opcode));
    gen_set_Rc0(dc, cpu_gpr[I16A_RA(dc->opcode)]);
}

static void gen_e_add2is(DisasContext *dc)
{
    tcg_gen_addi_tl(cpu_gpr[I16A_RA(dc->opcode)], cpu_gpr[I16A_RA(dc->opcode)], I16A_SI(dc->opcode) << 16);
}

static void gen_e_addic(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_and(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_andc(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_andi(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_andi(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_and2i(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_and2is(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_b(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_b(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_bc(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_bc(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_bclri(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_bctr(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_bgeni(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_blr(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_bmaski(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_bseti(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_btsti(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_cmp(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_cmph(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_cmph(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_cmphl(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_cmphl(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_cmph16i(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_cmp16i(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_cmphl16i(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_cmpl16i(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_cmpi_or_cmpli(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_cmpi(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_cmpl(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_cmpli(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_crand(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_crandc(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_creqv(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_crnand(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_crnor(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_cror(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_crorc(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_crxor(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_extsb(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_extsh(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_extzb(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_extzh(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_illegal(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_isync(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_lbz(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_lbz(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_lbzu(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_lha(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_lhau(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_lhz(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_lhz(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_lhzu(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_li(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_lis(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_li(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_lmw(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_lwz(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_lwz(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_lwzu(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_mcrf(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_mfar(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_mfctr(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_mflr(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_mr(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_mtar(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_mtctr(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_mtlr(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_mulli(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_mull2i(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_mullw(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_neg(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_not(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_or(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_or2i(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_or2is(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_ori(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_rfci(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_rfdi(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_rfi(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_rfmci(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_rlw(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_rlwi(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_rlwimi(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_rlwinm(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_sc(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_slwi(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_slw(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_slwi(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_sraw(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_srawi(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_srwi(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_srw(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_srwi(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_stb(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_stb(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_stbu(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_sth(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_sth(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_sthu(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_stmw(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_stw(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_stw(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_stwu(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_sub(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_subf(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_subfic(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_se_subi(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_xori(DisasContext *dc)
{
    ABORT_UNSUPPORTED_FEATURE;
}

static void gen_e_addi(DisasContext *dc)
{
    target_long imm = SCI8(dc->opcode);
    tcg_gen_addi_tl(cpu_gpr[SCI8_RD(dc->opcode)], cpu_gpr[SCI8_RA(dc->opcode)], imm);
    if(unlikely(SCI8_RC(dc->opcode))) {
        gen_set_Rc0(dc, cpu_gpr[SCI8_RD(dc->opcode)]);
    }
}

#define GEN_LONG_VLE_HANDLER(name, opc1, opc2, opc3) GEN_HANDLER(name, opc1, opc2, opc3, 0, PPC_VLE)

#define GEN_SHORT_VLE_HANDLER(name, opc1, opc2, opc3) GEN_OPCODE(name, opc1, opc2, opc3, 0, PPC_VLE, PPC_NONE, 2)

// clang-format off

static opcode_t vle_opcodes[] = {
GEN_SHORT_VLE_HANDLER(se_illegal, 0x0, 0x0, 0x0),
GEN_SHORT_VLE_HANDLER(se_isync, 0x0, 0x0, 0x1),
GEN_SHORT_VLE_HANDLER(se_sc, 0x0, 0x0, 0x2),
GEN_SHORT_VLE_HANDLER(se_blr, 0x0, 0x0, 0x4),
GEN_SHORT_VLE_HANDLER(se_blr, 0x0, 0x0, 0x5),
GEN_SHORT_VLE_HANDLER(se_bctr, 0x0, 0x0, 0x6),
GEN_SHORT_VLE_HANDLER(se_bctr, 0x0, 0x0, 0x7),
GEN_SHORT_VLE_HANDLER(se_rfi, 0x0, 0x0, 0x8),
GEN_SHORT_VLE_HANDLER(se_rfci, 0x0, 0x0, 0x9),
GEN_SHORT_VLE_HANDLER(se_rfdi, 0x0, 0x0, 0xA),
GEN_SHORT_VLE_HANDLER(se_rfmci, 0x0, 0x0, 0xB),
GEN_SHORT_VLE_HANDLER(se_not, 0x0, 0x2, 0xFF),
GEN_SHORT_VLE_HANDLER(se_neg, 0x0, 0x3, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mflr, 0x0, 0x8, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mtlr, 0x0, 0x9, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mfctr, 0x0, 0xA, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mtctr, 0x0, 0xB, 0xFF),
GEN_SHORT_VLE_HANDLER(se_extzb, 0x0, 0xC, 0xFF),
GEN_SHORT_VLE_HANDLER(se_extsb, 0x0, 0xD, 0xFF),
GEN_SHORT_VLE_HANDLER(se_extzh, 0x0, 0xE, 0xFF),
GEN_SHORT_VLE_HANDLER(se_extsh, 0x0, 0xF, 0xFF),

//Due to the shorter opcode, we need to register it multiple times.
GEN_SHORT_VLE_HANDLER(se_mr, 0x0, 0x10, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mr, 0x0, 0x11, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mr, 0x0, 0x12, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mr, 0x0, 0x13, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mr, 0x0, 0x14, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mr, 0x0, 0x15, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mr, 0x0, 0x16, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mr, 0x0, 0x17, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mr, 0x0, 0x18, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mr, 0x0, 0x19, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mr, 0x0, 0x1A, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mr, 0x0, 0x1B, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mr, 0x0, 0x1C, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mr, 0x0, 0x1D, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mr, 0x0, 0x1E, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mr, 0x0, 0x1F, 0xFF),

//Due to the shorter opcode, we need to register it multiple times.
GEN_SHORT_VLE_HANDLER(se_mtar, 0x0, 0x20, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mtar, 0x0, 0x21, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mtar, 0x0, 0x22, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mtar, 0x0, 0x23, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mtar, 0x0, 0x24, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mtar, 0x0, 0x25, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mtar, 0x0, 0x26, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mtar, 0x0, 0x27, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mtar, 0x0, 0x28, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mtar, 0x0, 0x29, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mtar, 0x0, 0x2A, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mtar, 0x0, 0x2B, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mtar, 0x0, 0x2C, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mtar, 0x0, 0x2D, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mtar, 0x0, 0x2E, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mtar, 0x0, 0x2F, 0xFF),

//Due to the shorter opcode, we need to register it multiple times.
GEN_SHORT_VLE_HANDLER(se_mfar, 0x0, 0x30, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mfar, 0x0, 0x31, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mfar, 0x0, 0x32, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mfar, 0x0, 0x33, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mfar, 0x0, 0x34, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mfar, 0x0, 0x35, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mfar, 0x0, 0x36, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mfar, 0x0, 0x37, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mfar, 0x0, 0x38, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mfar, 0x0, 0x39, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mfar, 0x0, 0x3A, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mfar, 0x0, 0x3B, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mfar, 0x0, 0x3C, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mfar, 0x0, 0x3D, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mfar, 0x0, 0x3E, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mfar, 0x0, 0x3F, 0xFF),

GEN_SHORT_VLE_HANDLER(se_add, 0x1, 0x0, 0xFF),
GEN_SHORT_VLE_HANDLER(se_mullw, 0x1, 0x1, 0xFF),
GEN_SHORT_VLE_HANDLER(se_sub, 0x1, 0x2, 0xFF),
GEN_SHORT_VLE_HANDLER(se_subf, 0x1, 0x3, 0xFF),

GEN_SHORT_VLE_HANDLER(se_cmp, 0x3, 0x0, 0xFF),
GEN_SHORT_VLE_HANDLER(se_cmpl, 0x3, 0x1, 0xFF),
GEN_SHORT_VLE_HANDLER(se_cmph, 0x3, 0x2, 0xFF),
GEN_SHORT_VLE_HANDLER(se_cmphl, 0x3, 0x3, 0xFF),

GEN_LONG_VLE_HANDLER(e_lbzu, 0x6, 0x0, 0x0),
GEN_LONG_VLE_HANDLER(e_lhzu, 0x6, 0x0, 0x1),
GEN_LONG_VLE_HANDLER(e_lwzu, 0x6, 0x0, 0x2),
GEN_LONG_VLE_HANDLER(e_lhau, 0x6, 0x0, 0x3),
GEN_LONG_VLE_HANDLER(e_stbu, 0x6, 0x0, 0x4),
GEN_LONG_VLE_HANDLER(e_sthu, 0x6, 0x0, 0x5),
GEN_LONG_VLE_HANDLER(e_stwu, 0x6, 0x0, 0x6),
GEN_LONG_VLE_HANDLER(e_lmw, 0x6, 0x0, 0x8),
GEN_LONG_VLE_HANDLER(e_stmw, 0x6, 0x0, 0x9),
GEN_LONG_VLE_HANDLER(e_addi, 0x6, 0x8, 0xFF),
GEN_LONG_VLE_HANDLER(e_addic, 0x6, 0x9, 0xFF),
GEN_LONG_VLE_HANDLER(e_mulli, 0x6, 0xA, 0x0),
GEN_LONG_VLE_HANDLER(e_mulli, 0x6, 0xA, 0x1),
GEN_LONG_VLE_HANDLER(e_mulli, 0x6, 0xA, 0x2),
GEN_LONG_VLE_HANDLER(e_mulli, 0x6, 0xA, 0x3),
GEN_LONG_VLE_HANDLER(e_mulli, 0x6, 0xA, 0x4),
GEN_LONG_VLE_HANDLER(e_mulli, 0x6, 0xA, 0x5),
GEN_LONG_VLE_HANDLER(e_mulli, 0x6, 0xA, 0x6),
GEN_LONG_VLE_HANDLER(e_mulli, 0x6, 0xA, 0x7),
GEN_LONG_VLE_HANDLER(e_cmpi_or_cmpli, 0x6, 0xA, 0x8), //this one has a specific opcode, so let's treat it here
GEN_LONG_VLE_HANDLER(e_cmpi_or_cmpli, 0x6, 0xA, 0x9),
GEN_LONG_VLE_HANDLER(e_cmpi_or_cmpli, 0x6, 0xA, 0xA),
GEN_LONG_VLE_HANDLER(e_cmpi_or_cmpli, 0x6, 0xA, 0xB),
GEN_LONG_VLE_HANDLER(e_cmpi_or_cmpli, 0x6, 0xA, 0xC),
GEN_LONG_VLE_HANDLER(e_cmpi_or_cmpli, 0x6, 0xA, 0xD),
GEN_LONG_VLE_HANDLER(e_cmpi_or_cmpli, 0x6, 0xA, 0xE),
GEN_LONG_VLE_HANDLER(e_cmpi_or_cmpli, 0x6, 0xA, 0xF),
GEN_LONG_VLE_HANDLER(e_subfic, 0x6, 0xB, 0xFF),
GEN_LONG_VLE_HANDLER(e_andi, 0x6, 0xC, 0xFF),
GEN_LONG_VLE_HANDLER(e_ori, 0x6, 0xD, 0xFF),
GEN_LONG_VLE_HANDLER(e_xori, 0x6, 0xE, 0xFF),

GEN_LONG_VLE_HANDLER(e_add16i, 0x7, 0xFF, 0xFF),

GEN_SHORT_VLE_HANDLER(se_addi, 0x8, 0x0, 0xFF),
GEN_SHORT_VLE_HANDLER(se_cmpli, 0x8, 0x1, 0xFF),

GEN_SHORT_VLE_HANDLER(se_subi, 0x9, 0xFF, 0xFF),

GEN_SHORT_VLE_HANDLER(se_cmpi, 0xA, 0x1, 0xFF),

GEN_SHORT_VLE_HANDLER(se_bmaski, 0xB, 0x0, 0xFF),
GEN_SHORT_VLE_HANDLER(se_andi, 0xB, 0x1, 0xFF),

GEN_LONG_VLE_HANDLER(e_lbz, 0xC, 0xFF, 0xFF),

GEN_LONG_VLE_HANDLER(e_stb, 0xD, 0xFF, 0xFF),

GEN_LONG_VLE_HANDLER(e_lha, 0xE, 0xFF, 0xFF),

GEN_SHORT_VLE_HANDLER(se_srw, 0x10, 0x0, 0xFF),
GEN_SHORT_VLE_HANDLER(se_sraw, 0x10, 0x1, 0xFF),
GEN_SHORT_VLE_HANDLER(se_slw, 0x10, 0x2, 0xFF),

GEN_SHORT_VLE_HANDLER(se_or, 0x11, 0x0, 0xFF),
GEN_SHORT_VLE_HANDLER(se_andc, 0x11, 0x1, 0xFF),
GEN_SHORT_VLE_HANDLER(se_and, 0x11, 0x2, 0xFF),
GEN_SHORT_VLE_HANDLER(se_and, 0x11, 0x3, 0xFF),

GEN_SHORT_VLE_HANDLER(se_li, 0x12, 0xFF, 0xFF),
GEN_SHORT_VLE_HANDLER(se_li, 0x13, 0xFF, 0xFF),

GEN_LONG_VLE_HANDLER(e_lwz, 0x14, 0xFF, 0xFF),

GEN_LONG_VLE_HANDLER(e_stw, 0x15, 0xFF, 0xFF),

GEN_LONG_VLE_HANDLER(e_lhz, 0x16, 0xFF, 0xFF),

GEN_LONG_VLE_HANDLER(e_sth, 0x17, 0xFF, 0xFF),

GEN_SHORT_VLE_HANDLER(se_bclri, 0x18, 0x0, 0xFF),
GEN_SHORT_VLE_HANDLER(se_bgeni, 0x18, 0x1, 0xFF),

GEN_SHORT_VLE_HANDLER(se_bseti, 0x19, 0x0, 0xFF),
GEN_SHORT_VLE_HANDLER(se_btsti, 0x19, 0x1, 0xFF),

GEN_SHORT_VLE_HANDLER(se_srwi, 0x1A, 0x0, 0xFF),
GEN_SHORT_VLE_HANDLER(se_srawi, 0x1A, 0x1, 0xFF),

GEN_SHORT_VLE_HANDLER(se_slwi, 0x1B, 0x0, 0xFF),

GEN_LONG_VLE_HANDLER(e_li, 0x1C, 0x0, 0xFF),
GEN_LONG_VLE_HANDLER(e_add2i, 0x1C, 0x1, 0x01),
GEN_LONG_VLE_HANDLER(e_add2is, 0x1C, 0x1, 0x02),
GEN_LONG_VLE_HANDLER(e_cmp16i, 0x1C, 0x1, 0x03),
GEN_LONG_VLE_HANDLER(e_mull2i, 0x1C, 0x1, 0x04),
GEN_LONG_VLE_HANDLER(e_cmpl16i, 0x1C, 0x1, 0x05),
GEN_LONG_VLE_HANDLER(e_cmph16i, 0x1C, 0x1, 0x06),
GEN_LONG_VLE_HANDLER(e_cmphl16i, 0x1C, 0x1, 0x07),
GEN_LONG_VLE_HANDLER(e_or2i, 0x1C, 0x1, 0x08),
GEN_LONG_VLE_HANDLER(e_and2i, 0x1C, 0x1, 0x09),
GEN_LONG_VLE_HANDLER(e_or2is, 0x1C, 0x1, 0x0A),
GEN_LONG_VLE_HANDLER(e_lis, 0x1C, 0x1, 0x0C),
GEN_LONG_VLE_HANDLER(e_and2is, 0x1C, 0x1, 0x0D),

GEN_LONG_VLE_HANDLER(e_rlwimi, 0x1D, 0x0, 0xFF),
GEN_LONG_VLE_HANDLER(e_rlwinm, 0x1D, 0x1, 0xFF),

GEN_LONG_VLE_HANDLER(e_b, 0x1E, 0x0, 0xFF),
GEN_LONG_VLE_HANDLER(e_bc, 0x1E, 0x1, 0xFF),

GEN_LONG_VLE_HANDLER(e_cmph, 0x1F, 0x0, 0xE),
GEN_LONG_VLE_HANDLER(e_mcrf, 0x1F, 0x1, 0x0),
GEN_LONG_VLE_HANDLER(e_crnor, 0x1F, 0x1, 0x1),
GEN_LONG_VLE_HANDLER(e_cmphl, 0x1F, 0x2, 0xE),
GEN_LONG_VLE_HANDLER(e_slwi, 0x1F, 0x3, 0x8),
GEN_LONG_VLE_HANDLER(e_crandc, 0x1F, 0x8, 0x1),
GEN_LONG_VLE_HANDLER(e_crxor, 0x1F, 0xC, 0x1),
GEN_LONG_VLE_HANDLER(e_crnand, 0x1F, 0xE, 0x1),
GEN_LONG_VLE_HANDLER(e_crand, 0x1F, 0x10, 0x1),
GEN_LONG_VLE_HANDLER(e_rlw, 0x1F, 0x11, 0x8),
GEN_LONG_VLE_HANDLER(e_creqv, 0x1F, 0x12, 0x1),
GEN_LONG_VLE_HANDLER(e_rlwi, 0x1F, 0x13, 0x8),
GEN_LONG_VLE_HANDLER(e_crorc, 0x1F, 0x1A, 0x1),
GEN_LONG_VLE_HANDLER(e_cror, 0x1F, 0x1C, 0x1),
GEN_LONG_VLE_HANDLER(e_srwi, 0x1F, 0x23, 0x8),

GEN_SHORT_VLE_HANDLER(se_lbz, 0x20, 0xFF, 0xFF),
GEN_SHORT_VLE_HANDLER(se_lbz, 0x21, 0xFF, 0xFF),
GEN_SHORT_VLE_HANDLER(se_lbz, 0x22, 0xFF, 0xFF),
GEN_SHORT_VLE_HANDLER(se_lbz, 0x23, 0xFF, 0xFF),

GEN_SHORT_VLE_HANDLER(se_stb, 0x24, 0xFF, 0xFF),
GEN_SHORT_VLE_HANDLER(se_stb, 0x25, 0xFF, 0xFF),
GEN_SHORT_VLE_HANDLER(se_stb, 0x26, 0xFF, 0xFF),
GEN_SHORT_VLE_HANDLER(se_stb, 0x27, 0xFF, 0xFF),

GEN_SHORT_VLE_HANDLER(se_lhz, 0x28, 0xFF, 0xFF),
GEN_SHORT_VLE_HANDLER(se_lhz, 0x29, 0xFF, 0xFF),
GEN_SHORT_VLE_HANDLER(se_lhz, 0x2A, 0xFF, 0xFF),
GEN_SHORT_VLE_HANDLER(se_lhz, 0x2B, 0xFF, 0xFF),

GEN_SHORT_VLE_HANDLER(se_sth, 0x2C, 0xFF, 0xFF),
GEN_SHORT_VLE_HANDLER(se_sth, 0x2D, 0xFF, 0xFF),
GEN_SHORT_VLE_HANDLER(se_sth, 0x2E, 0xFF, 0xFF),
GEN_SHORT_VLE_HANDLER(se_sth, 0x2F, 0xFF, 0xFF),

GEN_SHORT_VLE_HANDLER(se_lwz, 0x30, 0xFF, 0xFF),
GEN_SHORT_VLE_HANDLER(se_lwz, 0x31, 0xFF, 0xFF),
GEN_SHORT_VLE_HANDLER(se_lwz, 0x32, 0xFF, 0xFF),
GEN_SHORT_VLE_HANDLER(se_lwz, 0x33, 0xFF, 0xFF),

GEN_SHORT_VLE_HANDLER(se_stw, 0x34, 0xFF, 0xFF),
GEN_SHORT_VLE_HANDLER(se_stw, 0x35, 0xFF, 0xFF),
GEN_SHORT_VLE_HANDLER(se_stw, 0x36, 0xFF, 0xFF),
GEN_SHORT_VLE_HANDLER(se_stw, 0x37, 0xFF, 0xFF),

GEN_SHORT_VLE_HANDLER(se_bc, 0x38, 0xFF, 0xFF),
GEN_SHORT_VLE_HANDLER(se_bc, 0x39, 0xFF, 0xFF),

GEN_SHORT_VLE_HANDLER(se_b, 0x3A, 0xFF, 0xFF),
};

// clang-format on

#include "translate_init.inc"
#include "helper_regs.h"

//  This function decodes a VLE instruction and returns its 3 opcodes.
static void decode_vle_instruction(DisasContext *dc, uint32_t *op1, uint32_t *op2, uint32_t *op3)
{
    uint32_t opcode = dc->opcode;
    ;
    uint32_t o1 = (opcode >> 26) & ((1 << 6) - 1);
    uint32_t op2_shift = 0, op2_len = 0, op3_shift = 0, op3_len = 0;

    switch(o1) {
        case 0x00:
            op2_len = 6;     //  however, se_mfar, se_mtar and se_mr use 2bit long op2
            op2_shift = 20;  //  (if two MSBs are zero, op2 is 6bits long, otherwise it's 2)
            op3_len = 4;
            op3_shift = 16;
            break;
        case 0x01:
        case 0x03:
        case 0x10:
        case 0x11:
            op2_len = 2;
            op2_shift = 24;
            break;
        case 0x06:
            op2_len = 4;  //  this will allow more sensible grouping than 6-2
            op2_shift = 12;
            op3_len = 4;
            op3_shift = 8;
            break;
        case 0x08:
        case 0x0A:
        case 0x0B:
        case 0x18:
        case 0x19:
        case 0x1A:
        case 0x1B:
        case 0x1E:  //  this might cause problems, as the docs suggest op2_len = 4 for e_bc.
                    //  For this table it does not matter though.
            op2_len = 1;
            op2_shift = 25;
            break;
        case 0x1C:
            op2_len = 1;
            op2_shift = 15;
            op3_len = 4;
            op3_shift = 11;
            break;
        case 0x1D:
            op2_len = 1;
            op2_shift = 0;
            break;
        case 0x1F:
            op2_len = 6;
            op2_shift = 5;
            op3_len = 4;
            op3_shift = 1;
            break;
        default:  //  other instructions are verified to have no other opodes.
            break;
    }

    if(op1) {
        *op1 = o1;
    }
    if(op2) {
        *op2 = (opcode >> op2_shift) & ((1 << op2_len) - 1);
    }
    if(op3) {
        *op3 = (opcode >> op3_shift) & ((1 << op3_len) - 1);
    }
}

int disas_insn(CPUState *env, DisasContext *dc)
{
    opc_handler_t **table, *handler;
    uint32_t op1, op2, op3;

    if(unlikely(dc->le_mode)) {
        dc->opcode = bswap32(ldl_code(dc->base.pc));
    } else {
        dc->opcode = ldl_code(dc->base.pc);
    }

    tcg_gen_insn_start(dc->base.pc);

    if(env->count_opcodes) {
        generate_opcode_count_increment(env, dc->opcode);
    }

    if(dc->vle_enabled) {  //  use the vle decoding function to obtain the opcodes
        decode_vle_instruction(dc, &op1, &op2, &op3);
        table = env->vle_opcodes;
    } else {  //  use standard decoding macros
        op1 = opc1(dc->opcode);
        op2 = opc2(dc->opcode);
        op3 = opc3(dc->opcode);
        table = env->opcodes;
    }

    handler = table[op1];
    if(is_indirect_opcode(handler)) {
        table = ind_table(handler);
        handler = table[op2];
        if(is_indirect_opcode(handler)) {
            table = ind_table(handler);
            handler = table[op3];
        }
    }
    dc->base.pc += handler->length;

    /* Is opcode *REALLY* valid ? */
    if(likely(handler->handler != &gen_invalid)) {
        uint32_t inval;

        if(unlikely(handler->type & (PPC_SPE | PPC_SPE_SINGLE | PPC_SPE_DOUBLE) && Rc(dc->opcode))) {
            inval = handler->inval2;
        } else {
            inval = handler->inval1;
        }

        if(unlikely((dc->opcode & inval) != 0)) {
            gen_inval_exception(dc, POWERPC_EXCP_INVAL_INVAL);
            //  break; // TODO
        }
    }
    (*(handler->handler))(dc);
    return handler->length;
}

void setup_disas_context(DisasContextBase *base, CPUState *env)
{
    DisasContext *dc = (DisasContext *)base;
    dc->exception = POWERPC_EXCP_NONE;
    dc->spr_cb = env->spr_cb;
    dc->base.mem_idx = env->mmu_idx;
    dc->access_type = -1;
    dc->le_mode = env->hflags & (1 << MSR_LE) ? 1 : 0;
#if defined(TARGET_PPC64)
    dc->sf_mode = msr_sf;
    dc->has_cfar = !!(env->flags & POWERPC_FLAG_CFAR);
#endif
    dc->fpu_enabled = msr_fp;
    dc->vle_enabled = tlib_is_vle_enabled();
    if((env->flags & POWERPC_FLAG_SPE) && msr_spe) {
        dc->spe_enabled = msr_spe;
    } else {
        dc->spe_enabled = 0;
    }
    if((env->flags & POWERPC_FLAG_VRE) && msr_vr) {
        dc->altivec_enabled = msr_vr;
    } else {
        dc->altivec_enabled = 0;
    }
}

int gen_breakpoint(DisasContextBase *base, CPUBreakpoint *bp)
{
    gen_debug_exception((DisasContext *)base);
    return 1;
}

/*****************************************************************************/
int gen_intermediate_code(CPUState *env, DisasContextBase *base)
{
    DisasContext *dc = (DisasContext *)base;

    base->tb->size += disas_insn(env, (DisasContext *)base);

    if(unlikely((base->pc & (TARGET_PAGE_SIZE - 1)) == 0)) {
        /* if we reach a page boundary, stop generation */
        return 0;
    }
    if(dc->exception != POWERPC_EXCP_NONE) {
        return 0;
    }
    return 1;
}

uint32_t gen_intermediate_code_epilogue(CPUState *env, DisasContextBase *base)
{
    DisasContext *dc = (DisasContext *)base;
    if(dc->exception == POWERPC_EXCP_NONE) {
        gen_goto_tb(dc, 0, dc->base.pc);
    } else if(dc->exception != POWERPC_EXCP_BRANCH) {
        /* Generate the return instruction */
        gen_exit_tb_no_chaining(dc->base.tb);
    }
    return env->bfd_mach | dc->le_mode << 16;
}

void restore_state_to_opc(CPUState *env, TranslationBlock *tb, target_ulong *data)
{
    env->nip = data[0];
}

void cpu_exec_prologue(CPUState *env)
{
    env->reserve_addr = -1;
}

int process_interrupt(int interrupt_request, CPUState *env)
{
    if(tlib_is_in_debug_mode()) {
        return 0;
    }

    if(interrupt_request & CPU_INTERRUPT_HARD) {
        ppc_hw_interrupt(env);
        if(env->pending_interrupts == 0) {
            clear_interrupt_pending(env, CPU_INTERRUPT_HARD);
        }
        return 1;
    }
    return 0;
}

//  TODO: These empty implementations are required due to problems with weak attribute.
//  Remove this after #7035.
void cpu_exec_epilogue(CPUState *env) { }
