/*
   SPARC translation

   Copyright (C) 2003 Thomas M. Ogrisegg <tom@fnord.at>
   Copyright (C) 2003-2005 Fabrice Bellard

   This library is free software; you can redistribute it and/or
   modify it under the terms of the GNU Lesser General Public
   License as published by the Free Software Foundation; either
   version 2 of the License, or (at your option) any later version.

   This library is distributed in the hope that it will be useful,
   but WITHOUT ANY WARRANTY; without even the implied warranty of
   MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
   Lesser General Public License for more details.

   You should have received a copy of the GNU Lesser General Public
   License along with this library; if not, see <http://www.gnu.org/licenses/>.
 */

#include <stdarg.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <inttypes.h>

#include "cpu.h"
#include "arch_callbacks.h"

#include "tb-helper.h"

#include "debug.h"

#define DYNAMIC_PC 1 /* dynamic pc value */
#define JUMP_PC                                       \
    2 /* dynamic pc value which takes only two values \
         according to jump_pc[T2] */

/* global register indexes */
static TCGv_ptr cpu_regwptr;
static TCGv cpu_cc_src, cpu_cc_src2, cpu_cc_dst;
static TCGv_i32 cpu_cc_op;
static TCGv_i32 cpu_psr;
static TCGv cpu_fsr, cpu_pc, cpu_npc, cpu_gregs[8];
static TCGv cpu_y;
static TCGv cpu_asr[16] = { 0, 0x107, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0 };
static TCGv cpu_tbr;
static TCGv cpu_cond, cpu_dst, cpu_addr, cpu_val;
static TCGv cpu_wim;
/* local register indexes (only used inside old micro ops) */
static TCGv cpu_tmp0;
static TCGv_i32 cpu_tmp32;
static TCGv_i64 cpu_tmp64;
/* Floating point registers */
static TCGv_i32 cpu_fpr[TARGET_FPREGS];

void translate_init()
{
    unsigned int i;
    static const char *const gregnames[8] = {
        "g0", "g1", "g2", "g3", "g4", "g5", "g6", "g7",
    };
    static const char *const asrnames[16] = {
        "asr16", "asr17", "asr18", "asr19", "asr20", "asr21", "asr22", "asr23",
        "asr24", "asr25", "asr26", "asr27", "asr28", "asr29", "asr30", "asr31",
    };
    static const char *const fregnames[64] = {
        "f0",  "f1",  "f2",  "f3",  "f4",  "f5",  "f6",  "f7",  "f8",  "f9",  "f10", "f11", "f12", "f13", "f14", "f15",
        "f16", "f17", "f18", "f19", "f20", "f21", "f22", "f23", "f24", "f25", "f26", "f27", "f28", "f29", "f30", "f31",
        "f32", "f33", "f34", "f35", "f36", "f37", "f38", "f39", "f40", "f41", "f42", "f43", "f44", "f45", "f46", "f47",
        "f48", "f49", "f50", "f51", "f52", "f53", "f54", "f55", "f56", "f57", "f58", "f59", "f60", "f61", "f62", "f63",
    };

    /* init various static tables */
    cpu_regwptr = tcg_global_mem_new_ptr(TCG_AREG0, offsetof(CPUState, regwptr), "regwptr");
    cpu_wim = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, wim), "wim");
    cpu_cond = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, cond), "cond");
    cpu_cc_src = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, cc_src), "cc_src");
    cpu_cc_src2 = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, cc_src2), "cc_src2");
    cpu_cc_dst = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, cc_dst), "cc_dst");
    cpu_cc_op = tcg_global_mem_new_i32(TCG_AREG0, offsetof(CPUState, cc_op), "cc_op");
    cpu_psr = tcg_global_mem_new_i32(TCG_AREG0, offsetof(CPUState, psr), "psr");
    cpu_fsr = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, fsr), "fsr");
    cpu_pc = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, pc), "pc");
    cpu_npc = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, npc), "npc");
    cpu_y = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, y), "y");
    cpu_tbr = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, tbr), "tbr");
    for(i = 0; i < 16; i++) {
        cpu_asr[i] = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, asr[i]), asrnames[i]);
    }
    for(i = 1; i < 8; i++) {
        cpu_gregs[i] = tcg_global_mem_new(TCG_AREG0, offsetof(CPUState, gregs[i]), gregnames[i]);
    }
    for(i = 0; i < TARGET_FPREGS; i++) {
        cpu_fpr[i] = tcg_global_mem_new_i32(TCG_AREG0, offsetof(CPUState, fpr[i]), fregnames[i]);
    }
}

//  This function uses non-native bit order
#define GET_FIELD(X, FROM, TO) ((X) >> (31 - (TO)) & ((1 << ((TO) - (FROM) + 1)) - 1))

//  This function uses the order in the manuals, i.e. bit 0 is 2^0
#define GET_FIELD_SP(X, FROM, TO) GET_FIELD(X, 31 - (TO), 31 - (FROM))

#define GET_FIELDs(x, a, b)    sign_extend(GET_FIELD(x, a, b), (b) - (a) + 1)
#define GET_FIELD_SPs(x, a, b) sign_extend(GET_FIELD_SP(x, a, b), ((b) - (a) + 1))

#define DFPREG(r) (r & 0x1e)
#define QFPREG(r) (r & 0x1c)

#define UA2005_HTRAP_MASK 0xff
#define V8_TRAP_MASK      0x7f

static int sign_extend(int x, int len)
{
    len = 32 - len;
    return (x << len) >> len;
}

#define IS_IMM (insn & (1 << 13))

void gen_sync_pc(DisasContext *dc)
{
    /* if dc->base.pc is DYNAMIC_PC we are already updating the PC in gen_op_next_insn */
    if(dc->base.pc != DYNAMIC_PC) {
        tcg_gen_movi_tl(cpu_pc, dc->base.pc);
    }
}

/* floating point registers moves */
static void gen_op_load_fpr_DT0(unsigned int src)
{
    tcg_gen_st_i32(cpu_fpr[src], cpu_env, offsetof(CPUState, dt0) + offsetof(CPU_DoubleU, l.upper));
    tcg_gen_st_i32(cpu_fpr[src + 1], cpu_env, offsetof(CPUState, dt0) + offsetof(CPU_DoubleU, l.lower));
}

static void gen_op_load_fpr_DT1(unsigned int src)
{
    tcg_gen_st_i32(cpu_fpr[src], cpu_env, offsetof(CPUState, dt1) + offsetof(CPU_DoubleU, l.upper));
    tcg_gen_st_i32(cpu_fpr[src + 1], cpu_env, offsetof(CPUState, dt1) + offsetof(CPU_DoubleU, l.lower));
}

static void gen_op_store_DT0_fpr(unsigned int dst)
{
    tcg_gen_ld_i32(cpu_fpr[dst], cpu_env, offsetof(CPUState, dt0) + offsetof(CPU_DoubleU, l.upper));
    tcg_gen_ld_i32(cpu_fpr[dst + 1], cpu_env, offsetof(CPUState, dt0) + offsetof(CPU_DoubleU, l.lower));
}

static void gen_op_load_fpr_QT0(unsigned int src)
{
    tcg_gen_st_i32(cpu_fpr[src], cpu_env, offsetof(CPUState, qt0) + offsetof(CPU_QuadU, l.upmost));
    tcg_gen_st_i32(cpu_fpr[src + 1], cpu_env, offsetof(CPUState, qt0) + offsetof(CPU_QuadU, l.upper));
    tcg_gen_st_i32(cpu_fpr[src + 2], cpu_env, offsetof(CPUState, qt0) + offsetof(CPU_QuadU, l.lower));
    tcg_gen_st_i32(cpu_fpr[src + 3], cpu_env, offsetof(CPUState, qt0) + offsetof(CPU_QuadU, l.lowest));
}

static void gen_op_load_fpr_QT1(unsigned int src)
{
    tcg_gen_st_i32(cpu_fpr[src], cpu_env, offsetof(CPUState, qt1) + offsetof(CPU_QuadU, l.upmost));
    tcg_gen_st_i32(cpu_fpr[src + 1], cpu_env, offsetof(CPUState, qt1) + offsetof(CPU_QuadU, l.upper));
    tcg_gen_st_i32(cpu_fpr[src + 2], cpu_env, offsetof(CPUState, qt1) + offsetof(CPU_QuadU, l.lower));
    tcg_gen_st_i32(cpu_fpr[src + 3], cpu_env, offsetof(CPUState, qt1) + offsetof(CPU_QuadU, l.lowest));
}

static void gen_op_store_QT0_fpr(unsigned int dst)
{
    tcg_gen_ld_i32(cpu_fpr[dst], cpu_env, offsetof(CPUState, qt0) + offsetof(CPU_QuadU, l.upmost));
    tcg_gen_ld_i32(cpu_fpr[dst + 1], cpu_env, offsetof(CPUState, qt0) + offsetof(CPU_QuadU, l.upper));
    tcg_gen_ld_i32(cpu_fpr[dst + 2], cpu_env, offsetof(CPUState, qt0) + offsetof(CPU_QuadU, l.lower));
    tcg_gen_ld_i32(cpu_fpr[dst + 3], cpu_env, offsetof(CPUState, qt0) + offsetof(CPU_QuadU, l.lowest));
}

/* moves */
#define supervisor(dc) (dc->base.mem_idx >= MMU_KERNEL_IDX)

static inline void gen_movl_reg_TN(int reg, TCGv tn)
{
    if(reg == 0) {
        tcg_gen_movi_tl(tn, 0);
    } else if(reg < 8) {
        tcg_gen_mov_tl(tn, cpu_gregs[reg]);
    } else {
        tcg_gen_ld_tl(tn, cpu_regwptr, (reg - 8) * sizeof(target_ulong));
    }
}

static inline void gen_movl_TN_reg(int reg, TCGv tn)
{
    if(reg == 0) {
        return;
    } else if(reg < 8) {
        tcg_gen_mov_tl(cpu_gregs[reg], tn);
    } else {
        tcg_gen_st_tl(tn, cpu_regwptr, (reg - 8) * sizeof(target_ulong));
    }
}

static inline void gen_goto_tb(DisasContext *s, int tb_num, target_ulong pc, target_ulong npc)
{
    TranslationBlock *tb;

    tb = s->base.tb;
    if((pc & TARGET_PAGE_MASK) == (tb->pc & TARGET_PAGE_MASK) && (npc & TARGET_PAGE_MASK) == (tb->pc & TARGET_PAGE_MASK)) {
        /* jump to same page: we can use a direct jump */
        tcg_gen_goto_tb(tb_num);
        tcg_gen_movi_tl(cpu_pc, pc);
        tcg_gen_movi_tl(cpu_npc, npc);
        gen_exit_tb(tb, tb_num);
    } else {
        /* jump to another page: currently not optimized */
        tcg_gen_movi_tl(cpu_pc, pc);
        tcg_gen_movi_tl(cpu_npc, npc);
        gen_exit_tb_no_chaining(tb);
    }
}

//  XXX suboptimal
static inline void gen_mov_reg_N(TCGv reg, TCGv_i32 src)
{
    tcg_gen_extu_i32_tl(reg, src);
    tcg_gen_shri_tl(reg, reg, PSR_NEG_SHIFT);
    tcg_gen_andi_tl(reg, reg, 0x1);
}

static inline void gen_mov_reg_Z(TCGv reg, TCGv_i32 src)
{
    tcg_gen_extu_i32_tl(reg, src);
    tcg_gen_shri_tl(reg, reg, PSR_ZERO_SHIFT);
    tcg_gen_andi_tl(reg, reg, 0x1);
}

static inline void gen_mov_reg_V(TCGv reg, TCGv_i32 src)
{
    tcg_gen_extu_i32_tl(reg, src);
    tcg_gen_shri_tl(reg, reg, PSR_OVF_SHIFT);
    tcg_gen_andi_tl(reg, reg, 0x1);
}

static inline void gen_mov_reg_C(TCGv reg, TCGv_i32 src)
{
    tcg_gen_extu_i32_tl(reg, src);
    tcg_gen_shri_tl(reg, reg, PSR_CARRY_SHIFT);
    tcg_gen_andi_tl(reg, reg, 0x1);
}

static inline void gen_add_tv(TCGv dst, TCGv src1, TCGv src2)
{
    TCGv r_temp;
    TCGv_i32 r_const;
    int l1;

    l1 = gen_new_label();

    r_temp = tcg_temp_new();
    tcg_gen_xor_tl(r_temp, src1, src2);
    tcg_gen_not_tl(r_temp, r_temp);
    tcg_gen_xor_tl(cpu_tmp0, src1, dst);
    tcg_gen_and_tl(r_temp, r_temp, cpu_tmp0);
    tcg_gen_andi_tl(r_temp, r_temp, (1ULL << 31));
    tcg_gen_brcondi_tl(TCG_COND_EQ, r_temp, 0, l1);
    r_const = tcg_const_i32(TT_TOVF);
    gen_helper_raise_exception(r_const);
    tcg_temp_free_i32(r_const);
    gen_set_label(l1);
    tcg_temp_free(r_temp);
}

static inline void gen_tag_tv(TCGv src1, TCGv src2)
{
    int l1;
    TCGv_i32 r_const;

    l1 = gen_new_label();
    tcg_gen_or_tl(cpu_tmp0, src1, src2);
    tcg_gen_andi_tl(cpu_tmp0, cpu_tmp0, 0x3);
    tcg_gen_brcondi_tl(TCG_COND_EQ, cpu_tmp0, 0, l1);
    r_const = tcg_const_i32(TT_TOVF);
    gen_helper_raise_exception(r_const);
    tcg_temp_free_i32(r_const);
    gen_set_label(l1);
}

static inline void gen_op_addi_cc(TCGv dst, TCGv src1, target_long src2)
{
    tcg_gen_mov_tl(cpu_cc_src, src1);
    tcg_gen_movi_tl(cpu_cc_src2, src2);
    tcg_gen_addi_tl(cpu_cc_dst, cpu_cc_src, src2);
    tcg_gen_mov_tl(dst, cpu_cc_dst);
}

static inline void gen_op_add_cc(TCGv dst, TCGv src1, TCGv src2)
{
    tcg_gen_mov_tl(cpu_cc_src, src1);
    tcg_gen_mov_tl(cpu_cc_src2, src2);
    tcg_gen_add_tl(cpu_cc_dst, cpu_cc_src, cpu_cc_src2);
    tcg_gen_mov_tl(dst, cpu_cc_dst);
}

static TCGv_i32 gen_add32_carry32(void)
{
    TCGv_i32 carry_32, cc_src1_32, cc_src2_32;

    /* Carry is computed from a previous add: (dst < src)  */
#if TARGET_LONG_BITS == 64
    cc_src1_32 = tcg_temp_new_i32();
    cc_src2_32 = tcg_temp_new_i32();
    tcg_gen_trunc_i64_i32(cc_src1_32, cpu_cc_dst);
    tcg_gen_trunc_i64_i32(cc_src2_32, cpu_cc_src);
#else
    cc_src1_32 = cpu_cc_dst;
    cc_src2_32 = cpu_cc_src;
#endif

    carry_32 = tcg_temp_new_i32();
    tcg_gen_setcond_i32(TCG_COND_LTU, carry_32, cc_src1_32, cc_src2_32);

#if TARGET_LONG_BITS == 64
    tcg_temp_free_i32(cc_src1_32);
    tcg_temp_free_i32(cc_src2_32);
#endif

    return carry_32;
}

static TCGv_i32 gen_sub32_carry32(void)
{
    TCGv_i32 carry_32, cc_src1_32, cc_src2_32;

    /* Carry is computed from a previous borrow: (src1 < src2)  */
#if TARGET_LONG_BITS == 64
    cc_src1_32 = tcg_temp_new_i32();
    cc_src2_32 = tcg_temp_new_i32();
    tcg_gen_trunc_i64_i32(cc_src1_32, cpu_cc_src);
    tcg_gen_trunc_i64_i32(cc_src2_32, cpu_cc_src2);
#else
    cc_src1_32 = cpu_cc_src;
    cc_src2_32 = cpu_cc_src2;
#endif

    carry_32 = tcg_temp_new_i32();
    tcg_gen_setcond_i32(TCG_COND_LTU, carry_32, cc_src1_32, cc_src2_32);

#if TARGET_LONG_BITS == 64
    tcg_temp_free_i32(cc_src1_32);
    tcg_temp_free_i32(cc_src2_32);
#endif

    return carry_32;
}

static void gen_op_addx_int(DisasContext *dc, TCGv dst, TCGv src1, TCGv src2, int update_cc)
{
    TCGv_i32 carry_32;
    TCGv carry;

    switch(dc->cc_op) {
        case CC_OP_DIV:
        case CC_OP_LOGIC:
            /* Carry is known to be zero.  Fall back to plain ADD.  */
            if(update_cc) {
                gen_op_add_cc(dst, src1, src2);
            } else {
                tcg_gen_add_tl(dst, src1, src2);
            }
            return;

        case CC_OP_ADD:
        case CC_OP_TADD:
        case CC_OP_TADDTV:
#if TCG_TARGET_REG_BITS == 32 && TARGET_LONG_BITS == 32
        {
            /* For 32-bit hosts, we can re-use the host's hardware carry
               generation by using an ADD2 opcode.  We discard the low
               part of the output.  Ideally we'd combine this operation
               with the add that generated the carry in the first place.  */
            TCGv dst_low = tcg_temp_new();
            tcg_gen_op6_i32(INDEX_op_add2_i32, dst_low, dst, cpu_cc_src, src1, cpu_cc_src2, src2);
            tcg_temp_free(dst_low);
            goto add_done;
        }
#endif
            carry_32 = gen_add32_carry32();
            break;

        case CC_OP_SUB:
        case CC_OP_TSUB:
        case CC_OP_TSUBTV:
            carry_32 = gen_sub32_carry32();
            break;

        default:
            /* We need external help to produce the carry.  */
            carry_32 = tcg_temp_new_i32();
            gen_helper_compute_C_icc(carry_32);
            break;
    }

#if TARGET_LONG_BITS == 64
    carry = tcg_temp_new();
    tcg_gen_extu_i32_i64(carry, carry_32);
#else
    carry = carry_32;
#endif

    tcg_gen_add_tl(dst, src1, src2);
    tcg_gen_add_tl(dst, dst, carry);

    tcg_temp_free_i32(carry_32);
#if TARGET_LONG_BITS == 64
    tcg_temp_free(carry);
#endif

#if TCG_TARGET_REG_BITS == 32 && TARGET_LONG_BITS == 32
add_done:
#endif
    if(update_cc) {
        tcg_gen_mov_tl(cpu_cc_src, src1);
        tcg_gen_mov_tl(cpu_cc_src2, src2);
        tcg_gen_mov_tl(cpu_cc_dst, dst);
        tcg_gen_movi_i32(cpu_cc_op, CC_OP_ADDX);
        dc->cc_op = CC_OP_ADDX;
    }
}

static inline void gen_op_tadd_cc(TCGv dst, TCGv src1, TCGv src2)
{
    tcg_gen_mov_tl(cpu_cc_src, src1);
    tcg_gen_mov_tl(cpu_cc_src2, src2);
    tcg_gen_add_tl(cpu_cc_dst, cpu_cc_src, cpu_cc_src2);
    tcg_gen_mov_tl(dst, cpu_cc_dst);
}

static inline void gen_op_tadd_ccTV(TCGv dst, TCGv src1, TCGv src2)
{
    tcg_gen_mov_tl(cpu_cc_src, src1);
    tcg_gen_mov_tl(cpu_cc_src2, src2);
    gen_tag_tv(cpu_cc_src, cpu_cc_src2);
    tcg_gen_add_tl(cpu_cc_dst, cpu_cc_src, cpu_cc_src2);
    gen_add_tv(cpu_cc_dst, cpu_cc_src, cpu_cc_src2);
    tcg_gen_mov_tl(dst, cpu_cc_dst);
}

static inline void gen_sub_tv(TCGv dst, TCGv src1, TCGv src2)
{
    TCGv r_temp;
    TCGv_i32 r_const;
    int l1;

    l1 = gen_new_label();

    r_temp = tcg_temp_new();
    tcg_gen_xor_tl(r_temp, src1, src2);
    tcg_gen_xor_tl(cpu_tmp0, src1, dst);
    tcg_gen_and_tl(r_temp, r_temp, cpu_tmp0);
    tcg_gen_andi_tl(r_temp, r_temp, (1ULL << 31));
    tcg_gen_brcondi_tl(TCG_COND_EQ, r_temp, 0, l1);
    r_const = tcg_const_i32(TT_TOVF);
    gen_helper_raise_exception(r_const);
    tcg_temp_free_i32(r_const);
    gen_set_label(l1);
    tcg_temp_free(r_temp);
}

static inline void gen_op_subi_cc(TCGv dst, TCGv src1, target_long src2, DisasContext *dc)
{
    tcg_gen_mov_tl(cpu_cc_src, src1);
    tcg_gen_movi_tl(cpu_cc_src2, src2);
    if(src2 == 0) {
        tcg_gen_mov_tl(cpu_cc_dst, src1);
        tcg_gen_movi_i32(cpu_cc_op, CC_OP_LOGIC);
        dc->cc_op = CC_OP_LOGIC;
    } else {
        tcg_gen_subi_tl(cpu_cc_dst, cpu_cc_src, src2);
        tcg_gen_movi_i32(cpu_cc_op, CC_OP_SUB);
        dc->cc_op = CC_OP_SUB;
    }
    tcg_gen_mov_tl(dst, cpu_cc_dst);
}

static inline void gen_op_sub_cc(TCGv dst, TCGv src1, TCGv src2)
{
    tcg_gen_mov_tl(cpu_cc_src, src1);
    tcg_gen_mov_tl(cpu_cc_src2, src2);
    tcg_gen_sub_tl(cpu_cc_dst, cpu_cc_src, cpu_cc_src2);
    tcg_gen_mov_tl(dst, cpu_cc_dst);
}

static void gen_op_subx_int(DisasContext *dc, TCGv dst, TCGv src1, TCGv src2, int update_cc)
{
    TCGv_i32 carry_32;
    TCGv carry;

    switch(dc->cc_op) {
        case CC_OP_DIV:
        case CC_OP_LOGIC:
            /* Carry is known to be zero.  Fall back to plain SUB.  */
            if(update_cc) {
                gen_op_sub_cc(dst, src1, src2);
            } else {
                tcg_gen_sub_tl(dst, src1, src2);
            }
            return;

        case CC_OP_ADD:
        case CC_OP_TADD:
        case CC_OP_TADDTV:
            carry_32 = gen_add32_carry32();
            break;

        case CC_OP_SUB:
        case CC_OP_TSUB:
        case CC_OP_TSUBTV:
#if TCG_TARGET_REG_BITS == 32 && TARGET_LONG_BITS == 32
        {
            /* For 32-bit hosts, we can re-use the host's hardware carry
               generation by using a SUB2 opcode.  We discard the low
               part of the output.  Ideally we'd combine this operation
               with the add that generated the carry in the first place.  */
            TCGv dst_low = tcg_temp_new();
            tcg_gen_op6_i32(INDEX_op_sub2_i32, dst_low, dst, cpu_cc_src, src1, cpu_cc_src2, src2);
            tcg_temp_free(dst_low);
            goto sub_done;
        }
#endif
            carry_32 = gen_sub32_carry32();
            break;

        default:
            /* We need external help to produce the carry.  */
            carry_32 = tcg_temp_new_i32();
            gen_helper_compute_C_icc(carry_32);
            break;
    }

#if TARGET_LONG_BITS == 64
    carry = tcg_temp_new();
    tcg_gen_extu_i32_i64(carry, carry_32);
#else
    carry = carry_32;
#endif

    tcg_gen_sub_tl(dst, src1, src2);
    tcg_gen_sub_tl(dst, dst, carry);

    tcg_temp_free_i32(carry_32);
#if TARGET_LONG_BITS == 64
    tcg_temp_free(carry);
#endif

#if TCG_TARGET_REG_BITS == 32 && TARGET_LONG_BITS == 32
sub_done:
#endif
    if(update_cc) {
        tcg_gen_mov_tl(cpu_cc_src, src1);
        tcg_gen_mov_tl(cpu_cc_src2, src2);
        tcg_gen_mov_tl(cpu_cc_dst, dst);
        tcg_gen_movi_i32(cpu_cc_op, CC_OP_SUBX);
        dc->cc_op = CC_OP_SUBX;
    }
}

static inline void gen_op_tsub_cc(TCGv dst, TCGv src1, TCGv src2)
{
    tcg_gen_mov_tl(cpu_cc_src, src1);
    tcg_gen_mov_tl(cpu_cc_src2, src2);
    tcg_gen_sub_tl(cpu_cc_dst, cpu_cc_src, cpu_cc_src2);
    tcg_gen_mov_tl(dst, cpu_cc_dst);
}

static inline void gen_op_tsub_ccTV(TCGv dst, TCGv src1, TCGv src2)
{
    tcg_gen_mov_tl(cpu_cc_src, src1);
    tcg_gen_mov_tl(cpu_cc_src2, src2);
    gen_tag_tv(cpu_cc_src, cpu_cc_src2);
    tcg_gen_sub_tl(cpu_cc_dst, cpu_cc_src, cpu_cc_src2);
    gen_sub_tv(cpu_cc_dst, cpu_cc_src, cpu_cc_src2);
    tcg_gen_mov_tl(dst, cpu_cc_dst);
}

static inline void gen_op_mulscc(TCGv dst, TCGv src1, TCGv src2)
{
    TCGv r_temp;
    int l1;

    l1 = gen_new_label();
    r_temp = tcg_temp_new();

    /* old op:
       if (!(env->y & 1))
        T1 = 0;
     */
    tcg_gen_andi_tl(cpu_cc_src, src1, 0xffffffff);
    tcg_gen_andi_tl(r_temp, cpu_y, 0x1);
    tcg_gen_andi_tl(cpu_cc_src2, src2, 0xffffffff);
    tcg_gen_brcondi_tl(TCG_COND_NE, r_temp, 0, l1);
    tcg_gen_movi_tl(cpu_cc_src2, 0);
    gen_set_label(l1);

    //  b2 = T0 & 1;
    //  env->y = (b2 << 31) | (env->y >> 1);
    tcg_gen_andi_tl(r_temp, cpu_cc_src, 0x1);
    tcg_gen_shli_tl(r_temp, r_temp, 31);
    tcg_gen_shri_tl(cpu_tmp0, cpu_y, 1);
    tcg_gen_andi_tl(cpu_tmp0, cpu_tmp0, 0x7fffffff);
    tcg_gen_or_tl(cpu_tmp0, cpu_tmp0, r_temp);
    tcg_gen_andi_tl(cpu_y, cpu_tmp0, 0xffffffff);

    //  b1 = N ^ V;
    gen_mov_reg_N(cpu_tmp0, cpu_psr);
    gen_mov_reg_V(r_temp, cpu_psr);
    tcg_gen_xor_tl(cpu_tmp0, cpu_tmp0, r_temp);
    tcg_temp_free(r_temp);

    //  T0 = (b1 << 31) | (T0 >> 1);
    //  src1 = T0;
    tcg_gen_shli_tl(cpu_tmp0, cpu_tmp0, 31);
    tcg_gen_shri_tl(cpu_cc_src, cpu_cc_src, 1);
    tcg_gen_or_tl(cpu_cc_src, cpu_cc_src, cpu_tmp0);

    tcg_gen_add_tl(cpu_cc_dst, cpu_cc_src, cpu_cc_src2);

    tcg_gen_mov_tl(dst, cpu_cc_dst);
}

static inline void gen_op_multiply(TCGv dst, TCGv src1, TCGv src2, int sign_ext)
{
    TCGv_i32 r_src1, r_src2;
    TCGv_i64 r_temp, r_temp2;

    r_src1 = tcg_temp_new_i32();
    r_src2 = tcg_temp_new_i32();

    tcg_gen_trunc_tl_i32(r_src1, src1);
    tcg_gen_trunc_tl_i32(r_src2, src2);

    r_temp = tcg_temp_new_i64();
    r_temp2 = tcg_temp_new_i64();

    if(sign_ext) {
        tcg_gen_ext_i32_i64(r_temp, r_src2);
        tcg_gen_ext_i32_i64(r_temp2, r_src1);
    } else {
        tcg_gen_extu_i32_i64(r_temp, r_src2);
        tcg_gen_extu_i32_i64(r_temp2, r_src1);
    }

    tcg_gen_mul_i64(r_temp2, r_temp, r_temp2);

    tcg_gen_shri_i64(r_temp, r_temp2, 32);
    tcg_gen_trunc_i64_tl(cpu_tmp0, r_temp);
    tcg_temp_free_i64(r_temp);
    tcg_gen_andi_tl(cpu_y, cpu_tmp0, 0xffffffff);

    tcg_gen_trunc_i64_tl(dst, r_temp2);

    tcg_temp_free_i64(r_temp2);

    tcg_temp_free_i32(r_src1);
    tcg_temp_free_i32(r_src2);
}

static inline void gen_op_umul(TCGv dst, TCGv src1, TCGv src2)
{
    /* zero-extend truncated operands before multiplication */
    gen_op_multiply(dst, src1, src2, 0);
}

static inline void gen_op_smul(TCGv dst, TCGv src1, TCGv src2)
{
    /* sign-extend truncated operands before multiplication */
    gen_op_multiply(dst, src1, src2, 1);
}

//  1
static inline void gen_op_eval_ba(TCGv dst)
{
    tcg_gen_movi_tl(dst, 1);
}

//  Z
static inline void gen_op_eval_be(TCGv dst, TCGv_i32 src)
{
    gen_mov_reg_Z(dst, src);
}

//  Z | (N ^ V)
static inline void gen_op_eval_ble(TCGv dst, TCGv_i32 src)
{
    gen_mov_reg_N(cpu_tmp0, src);
    gen_mov_reg_V(dst, src);
    tcg_gen_xor_tl(dst, dst, cpu_tmp0);
    gen_mov_reg_Z(cpu_tmp0, src);
    tcg_gen_or_tl(dst, dst, cpu_tmp0);
}

//  N ^ V
static inline void gen_op_eval_bl(TCGv dst, TCGv_i32 src)
{
    gen_mov_reg_V(cpu_tmp0, src);
    gen_mov_reg_N(dst, src);
    tcg_gen_xor_tl(dst, dst, cpu_tmp0);
}

//  C | Z
static inline void gen_op_eval_bleu(TCGv dst, TCGv_i32 src)
{
    gen_mov_reg_Z(cpu_tmp0, src);
    gen_mov_reg_C(dst, src);
    tcg_gen_or_tl(dst, dst, cpu_tmp0);
}

//  C
static inline void gen_op_eval_bcs(TCGv dst, TCGv_i32 src)
{
    gen_mov_reg_C(dst, src);
}

//  V
static inline void gen_op_eval_bvs(TCGv dst, TCGv_i32 src)
{
    gen_mov_reg_V(dst, src);
}

//  0
static inline void gen_op_eval_bn(TCGv dst)
{
    tcg_gen_movi_tl(dst, 0);
}

//  N
static inline void gen_op_eval_bneg(TCGv dst, TCGv_i32 src)
{
    gen_mov_reg_N(dst, src);
}

//  !Z
static inline void gen_op_eval_bne(TCGv dst, TCGv_i32 src)
{
    gen_mov_reg_Z(dst, src);
    tcg_gen_xori_tl(dst, dst, 0x1);
}

//  !(Z | (N ^ V))
static inline void gen_op_eval_bg(TCGv dst, TCGv_i32 src)
{
    gen_mov_reg_N(cpu_tmp0, src);
    gen_mov_reg_V(dst, src);
    tcg_gen_xor_tl(dst, dst, cpu_tmp0);
    gen_mov_reg_Z(cpu_tmp0, src);
    tcg_gen_or_tl(dst, dst, cpu_tmp0);
    tcg_gen_xori_tl(dst, dst, 0x1);
}

//  !(N ^ V)
static inline void gen_op_eval_bge(TCGv dst, TCGv_i32 src)
{
    gen_mov_reg_V(cpu_tmp0, src);
    gen_mov_reg_N(dst, src);
    tcg_gen_xor_tl(dst, dst, cpu_tmp0);
    tcg_gen_xori_tl(dst, dst, 0x1);
}

//  !(C | Z)
static inline void gen_op_eval_bgu(TCGv dst, TCGv_i32 src)
{
    gen_mov_reg_Z(cpu_tmp0, src);
    gen_mov_reg_C(dst, src);
    tcg_gen_or_tl(dst, dst, cpu_tmp0);
    tcg_gen_xori_tl(dst, dst, 0x1);
}

//  !C
static inline void gen_op_eval_bcc(TCGv dst, TCGv_i32 src)
{
    gen_mov_reg_C(dst, src);
    tcg_gen_xori_tl(dst, dst, 0x1);
}

//  !N
static inline void gen_op_eval_bpos(TCGv dst, TCGv_i32 src)
{
    gen_mov_reg_N(dst, src);
    tcg_gen_xori_tl(dst, dst, 0x1);
}

//  !V
static inline void gen_op_eval_bvc(TCGv dst, TCGv_i32 src)
{
    gen_mov_reg_V(dst, src);
    tcg_gen_xori_tl(dst, dst, 0x1);
}

/*
   FPSR bit field FCC1 | FCC0:
   0 =
   1 <
   2 >
   3 unordered
 */
static inline void gen_mov_reg_FCC0(TCGv reg, TCGv src, unsigned int fcc_offset)
{
    tcg_gen_shri_tl(reg, src, FSR_FCC0_SHIFT + fcc_offset);
    tcg_gen_andi_tl(reg, reg, 0x1);
}

static inline void gen_mov_reg_FCC1(TCGv reg, TCGv src, unsigned int fcc_offset)
{
    tcg_gen_shri_tl(reg, src, FSR_FCC1_SHIFT + fcc_offset);
    tcg_gen_andi_tl(reg, reg, 0x1);
}

//  !0: FCC0 | FCC1
static inline void gen_op_eval_fbne(TCGv dst, TCGv src, unsigned int fcc_offset)
{
    gen_mov_reg_FCC0(dst, src, fcc_offset);
    gen_mov_reg_FCC1(cpu_tmp0, src, fcc_offset);
    tcg_gen_or_tl(dst, dst, cpu_tmp0);
}

//  1 or 2: FCC0 ^ FCC1
static inline void gen_op_eval_fblg(TCGv dst, TCGv src, unsigned int fcc_offset)
{
    gen_mov_reg_FCC0(dst, src, fcc_offset);
    gen_mov_reg_FCC1(cpu_tmp0, src, fcc_offset);
    tcg_gen_xor_tl(dst, dst, cpu_tmp0);
}

//  1 or 3: FCC0
static inline void gen_op_eval_fbul(TCGv dst, TCGv src, unsigned int fcc_offset)
{
    gen_mov_reg_FCC0(dst, src, fcc_offset);
}

//  1: FCC0 & !FCC1
static inline void gen_op_eval_fbl(TCGv dst, TCGv src, unsigned int fcc_offset)
{
    gen_mov_reg_FCC0(dst, src, fcc_offset);
    gen_mov_reg_FCC1(cpu_tmp0, src, fcc_offset);
    tcg_gen_xori_tl(cpu_tmp0, cpu_tmp0, 0x1);
    tcg_gen_and_tl(dst, dst, cpu_tmp0);
}

//  2 or 3: FCC1
static inline void gen_op_eval_fbug(TCGv dst, TCGv src, unsigned int fcc_offset)
{
    gen_mov_reg_FCC1(dst, src, fcc_offset);
}

//  2: !FCC0 & FCC1
static inline void gen_op_eval_fbg(TCGv dst, TCGv src, unsigned int fcc_offset)
{
    gen_mov_reg_FCC0(dst, src, fcc_offset);
    tcg_gen_xori_tl(dst, dst, 0x1);
    gen_mov_reg_FCC1(cpu_tmp0, src, fcc_offset);
    tcg_gen_and_tl(dst, dst, cpu_tmp0);
}

//  3: FCC0 & FCC1
static inline void gen_op_eval_fbu(TCGv dst, TCGv src, unsigned int fcc_offset)
{
    gen_mov_reg_FCC0(dst, src, fcc_offset);
    gen_mov_reg_FCC1(cpu_tmp0, src, fcc_offset);
    tcg_gen_and_tl(dst, dst, cpu_tmp0);
}

//  0: !(FCC0 | FCC1)
static inline void gen_op_eval_fbe(TCGv dst, TCGv src, unsigned int fcc_offset)
{
    gen_mov_reg_FCC0(dst, src, fcc_offset);
    gen_mov_reg_FCC1(cpu_tmp0, src, fcc_offset);
    tcg_gen_or_tl(dst, dst, cpu_tmp0);
    tcg_gen_xori_tl(dst, dst, 0x1);
}

//  0 or 3: !(FCC0 ^ FCC1)
static inline void gen_op_eval_fbue(TCGv dst, TCGv src, unsigned int fcc_offset)
{
    gen_mov_reg_FCC0(dst, src, fcc_offset);
    gen_mov_reg_FCC1(cpu_tmp0, src, fcc_offset);
    tcg_gen_xor_tl(dst, dst, cpu_tmp0);
    tcg_gen_xori_tl(dst, dst, 0x1);
}

//  0 or 2: !FCC0
static inline void gen_op_eval_fbge(TCGv dst, TCGv src, unsigned int fcc_offset)
{
    gen_mov_reg_FCC0(dst, src, fcc_offset);
    tcg_gen_xori_tl(dst, dst, 0x1);
}

//  !1: !(FCC0 & !FCC1)
static inline void gen_op_eval_fbuge(TCGv dst, TCGv src, unsigned int fcc_offset)
{
    gen_mov_reg_FCC0(dst, src, fcc_offset);
    gen_mov_reg_FCC1(cpu_tmp0, src, fcc_offset);
    tcg_gen_xori_tl(cpu_tmp0, cpu_tmp0, 0x1);
    tcg_gen_and_tl(dst, dst, cpu_tmp0);
    tcg_gen_xori_tl(dst, dst, 0x1);
}

//  0 or 1: !FCC1
static inline void gen_op_eval_fble(TCGv dst, TCGv src, unsigned int fcc_offset)
{
    gen_mov_reg_FCC1(dst, src, fcc_offset);
    tcg_gen_xori_tl(dst, dst, 0x1);
}

//  !2: !(!FCC0 & FCC1)
static inline void gen_op_eval_fbule(TCGv dst, TCGv src, unsigned int fcc_offset)
{
    gen_mov_reg_FCC0(dst, src, fcc_offset);
    tcg_gen_xori_tl(dst, dst, 0x1);
    gen_mov_reg_FCC1(cpu_tmp0, src, fcc_offset);
    tcg_gen_and_tl(dst, dst, cpu_tmp0);
    tcg_gen_xori_tl(dst, dst, 0x1);
}

//  !3: !(FCC0 & FCC1)
static inline void gen_op_eval_fbo(TCGv dst, TCGv src, unsigned int fcc_offset)
{
    gen_mov_reg_FCC0(dst, src, fcc_offset);
    gen_mov_reg_FCC1(cpu_tmp0, src, fcc_offset);
    tcg_gen_and_tl(dst, dst, cpu_tmp0);
    tcg_gen_xori_tl(dst, dst, 0x1);
}

static inline void gen_branch2(DisasContext *dc, target_ulong pc1, target_ulong pc2, TCGv r_cond)
{
    int l1;

    l1 = gen_new_label();

    tcg_gen_brcondi_tl(TCG_COND_EQ, r_cond, 0, l1);

    gen_goto_tb(dc, 0, pc1, pc1 + 4);

    gen_set_label(l1);
    gen_goto_tb(dc, 1, pc2, pc2 + 4);
}

static inline void gen_branch_a(DisasContext *dc, target_ulong pc1, target_ulong pc2, TCGv r_cond)
{
    int l1;

    l1 = gen_new_label();

    tcg_gen_brcondi_tl(TCG_COND_EQ, r_cond, 0, l1);

    gen_goto_tb(dc, 0, pc2, pc1);

    gen_set_label(l1);
    gen_goto_tb(dc, 1, pc2 + 4, pc2 + 8);
}

static inline void gen_generic_branch(target_ulong npc1, target_ulong npc2, TCGv r_cond)
{
    int l1, l2;

    l1 = gen_new_label();
    l2 = gen_new_label();

    tcg_gen_brcondi_tl(TCG_COND_EQ, r_cond, 0, l1);

    tcg_gen_movi_tl(cpu_npc, npc1);
    tcg_gen_br(l2);

    gen_set_label(l1);
    tcg_gen_movi_tl(cpu_npc, npc2);
    gen_set_label(l2);
}

/* call this function before using the condition register as it may
   have been set for a jump */
static inline void flush_cond(DisasContext *dc, TCGv cond)
{
    if(dc->npc == JUMP_PC) {
        gen_generic_branch(dc->jump_pc[0], dc->jump_pc[1], cond);
        dc->npc = DYNAMIC_PC;
    }
}

static inline void save_npc(DisasContext *dc, TCGv cond)
{
    if(dc->npc == JUMP_PC) {
        gen_generic_branch(dc->jump_pc[0], dc->jump_pc[1], cond);
        dc->npc = DYNAMIC_PC;
    } else if(dc->npc != DYNAMIC_PC) {
        tcg_gen_movi_tl(cpu_npc, dc->npc);
    }
}

static inline void save_state(DisasContext *dc, TCGv cond)
{
    tcg_gen_movi_tl(cpu_pc, dc->base.pc);
    /* flush pending conditional evaluations before exposing cpu state */
    if(dc->cc_op != CC_OP_FLAGS) {
        dc->cc_op = CC_OP_FLAGS;
        gen_helper_compute_psr();
    }
    save_npc(dc, cond);
}

static inline void gen_mov_pc_npc(DisasContext *dc, TCGv cond)
{
    if(dc->npc == JUMP_PC) {
        gen_generic_branch(dc->jump_pc[0], dc->jump_pc[1], cond);
        tcg_gen_mov_tl(cpu_pc, cpu_npc);
        dc->base.pc = DYNAMIC_PC;
    } else if(dc->npc == DYNAMIC_PC) {
        tcg_gen_mov_tl(cpu_pc, cpu_npc);
        dc->base.pc = DYNAMIC_PC;
    } else {
        dc->base.pc = dc->npc;
    }
}

static inline void gen_op_next_insn(void)
{
    tcg_gen_mov_tl(cpu_pc, cpu_npc);
    tcg_gen_addi_tl(cpu_npc, cpu_npc, 4);
}

static inline void gen_cond(TCGv r_dst, unsigned int cc, unsigned int cond, DisasContext *dc)
{
    TCGv_i32 r_src;

    r_src = cpu_psr;
    switch(dc->cc_op) {
        case CC_OP_FLAGS:
            break;
        default:
            gen_helper_compute_psr();
            dc->cc_op = CC_OP_FLAGS;
            break;
    }
    switch(cond) {
        case 0x0:
            gen_op_eval_bn(r_dst);
            break;
        case 0x1:
            gen_op_eval_be(r_dst, r_src);
            break;
        case 0x2:
            gen_op_eval_ble(r_dst, r_src);
            break;
        case 0x3:
            gen_op_eval_bl(r_dst, r_src);
            break;
        case 0x4:
            gen_op_eval_bleu(r_dst, r_src);
            break;
        case 0x5:
            gen_op_eval_bcs(r_dst, r_src);
            break;
        case 0x6:
            gen_op_eval_bneg(r_dst, r_src);
            break;
        case 0x7:
            gen_op_eval_bvs(r_dst, r_src);
            break;
        case 0x8:
            gen_op_eval_ba(r_dst);
            break;
        case 0x9:
            gen_op_eval_bne(r_dst, r_src);
            break;
        case 0xa:
            gen_op_eval_bg(r_dst, r_src);
            break;
        case 0xb:
            gen_op_eval_bge(r_dst, r_src);
            break;
        case 0xc:
            gen_op_eval_bgu(r_dst, r_src);
            break;
        case 0xd:
            gen_op_eval_bcc(r_dst, r_src);
            break;
        case 0xe:
            gen_op_eval_bpos(r_dst, r_src);
            break;
        case 0xf:
            gen_op_eval_bvc(r_dst, r_src);
            break;
    }
}

static inline void gen_fcond(TCGv r_dst, unsigned int cc, unsigned int cond)
{
    unsigned int offset;

    switch(cc) {
        default:
        case 0x0:
            offset = 0;
            break;
        case 0x1:
            offset = 32 - 10;
            break;
        case 0x2:
            offset = 34 - 10;
            break;
        case 0x3:
            offset = 36 - 10;
            break;
    }

    switch(cond) {
        case 0x0:
            gen_op_eval_bn(r_dst);
            break;
        case 0x1:
            gen_op_eval_fbne(r_dst, cpu_fsr, offset);
            break;
        case 0x2:
            gen_op_eval_fblg(r_dst, cpu_fsr, offset);
            break;
        case 0x3:
            gen_op_eval_fbul(r_dst, cpu_fsr, offset);
            break;
        case 0x4:
            gen_op_eval_fbl(r_dst, cpu_fsr, offset);
            break;
        case 0x5:
            gen_op_eval_fbug(r_dst, cpu_fsr, offset);
            break;
        case 0x6:
            gen_op_eval_fbg(r_dst, cpu_fsr, offset);
            break;
        case 0x7:
            gen_op_eval_fbu(r_dst, cpu_fsr, offset);
            break;
        case 0x8:
            gen_op_eval_ba(r_dst);
            break;
        case 0x9:
            gen_op_eval_fbe(r_dst, cpu_fsr, offset);
            break;
        case 0xa:
            gen_op_eval_fbue(r_dst, cpu_fsr, offset);
            break;
        case 0xb:
            gen_op_eval_fbge(r_dst, cpu_fsr, offset);
            break;
        case 0xc:
            gen_op_eval_fbuge(r_dst, cpu_fsr, offset);
            break;
        case 0xd:
            gen_op_eval_fble(r_dst, cpu_fsr, offset);
            break;
        case 0xe:
            gen_op_eval_fbule(r_dst, cpu_fsr, offset);
            break;
        case 0xf:
            gen_op_eval_fbo(r_dst, cpu_fsr, offset);
            break;
    }
}

/* XXX: potentially incorrect if dynamic npc */
static void do_branch(DisasContext *dc, int32_t offset, uint32_t insn, int cc, TCGv r_cond)
{
    unsigned int cond = GET_FIELD(insn, 3, 6), a = (insn & (1 << 29));
    target_ulong target = dc->base.pc + offset;

    if(cond == 0x0) {
        /* unconditional not taken */
        if(a) {
            dc->base.pc = dc->npc + 4;
            dc->npc = dc->base.pc + 4;
        } else {
            dc->base.pc = dc->npc;
            dc->npc = dc->base.pc + 4;
        }
    } else if(cond == 0x8) {
        /* unconditional taken */
        if(a) {
            dc->base.pc = target;
            dc->npc = dc->base.pc + 4;
        } else {
            dc->base.pc = dc->npc;
            dc->npc = target;
            tcg_gen_mov_tl(cpu_pc, cpu_npc);
        }
    } else {
        flush_cond(dc, r_cond);
        gen_cond(r_cond, cc, cond, dc);
        if(a) {
            gen_branch_a(dc, target, dc->npc, r_cond);
            dc->base.is_jmp = DISAS_JUMP;
        } else {
            dc->base.pc = dc->npc;
            dc->jump_pc[0] = target;
            dc->jump_pc[1] = dc->npc + 4;
            dc->npc = JUMP_PC;
        }
    }
}

/* XXX: potentially incorrect if dynamic npc */
static void do_fbranch(DisasContext *dc, int32_t offset, uint32_t insn, int cc, TCGv r_cond)
{
    unsigned int cond = GET_FIELD(insn, 3, 6), a = (insn & (1 << 29));
    target_ulong target = dc->base.pc + offset;

    if(cond == 0x0) {
        /* unconditional not taken */
        if(a) {
            dc->base.pc = dc->npc + 4;
            dc->npc = dc->base.pc + 4;
        } else {
            dc->base.pc = dc->npc;
            dc->npc = dc->base.pc + 4;
        }
    } else if(cond == 0x8) {
        /* unconditional taken */
        if(a) {
            dc->base.pc = target;
            dc->npc = dc->base.pc + 4;
        } else {
            dc->base.pc = dc->npc;
            dc->npc = target;
            tcg_gen_mov_tl(cpu_pc, cpu_npc);
        }
    } else {
        flush_cond(dc, r_cond);
        gen_fcond(r_cond, cc, cond);
        if(a) {
            gen_branch_a(dc, target, dc->npc, r_cond);
            dc->base.is_jmp = DISAS_JUMP;
        } else {
            dc->base.pc = dc->npc;
            dc->jump_pc[0] = target;
            dc->jump_pc[1] = dc->npc + 4;
            dc->npc = JUMP_PC;
        }
    }
}

static inline void gen_op_fcmps(int fccno, TCGv r_rs1, TCGv r_rs2)
{
    gen_helper_fcmps(r_rs1, r_rs2);
}

static inline void gen_op_fcmpd(int fccno)
{
    gen_helper_fcmpd();
}

static inline void gen_op_fcmpq(int fccno)
{
    gen_helper_fcmpq();
}

static inline void gen_op_fcmpes(int fccno, TCGv r_rs1, TCGv r_rs2)
{
    gen_helper_fcmpes(r_rs1, r_rs2);
}

static inline void gen_op_fcmped(int fccno)
{
    gen_helper_fcmped();
}

static inline void gen_op_fcmpeq(int fccno)
{
    gen_helper_fcmpeq();
}

static inline void gen_op_fpexception_im(int fsr_flags)
{
    TCGv_i32 r_const;

    tcg_gen_andi_tl(cpu_fsr, cpu_fsr, FSR_FTT_NMASK);
    tcg_gen_ori_tl(cpu_fsr, cpu_fsr, fsr_flags);
    r_const = tcg_const_i32(TT_FP_EXCP);
    gen_helper_raise_exception(r_const);
    tcg_temp_free_i32(r_const);
}

static int gen_trap_ifnofpu(DisasContext *dc, TCGv r_cond)
{
    if(!dc->fpu_enabled) {
        TCGv_i32 r_const;

        save_state(dc, r_cond);
        r_const = tcg_const_i32(TT_NFPU_INSN);
        gen_helper_raise_exception(r_const);
        tcg_temp_free_i32(r_const);
        dc->base.is_jmp = DISAS_JUMP;
        return 1;
    }
    return 0;
}

static inline void gen_op_clear_ieee_excp_and_FTT(void)
{
    tcg_gen_andi_tl(cpu_fsr, cpu_fsr, FSR_FTT_CEXC_NMASK);
}

static inline void gen_clear_float_exceptions(void)
{
    gen_helper_clear_float_exceptions();
}

/* asi moves */
static inline void gen_ld_asi(TCGv dst, TCGv addr, int insn, int size, int sign)
{
    TCGv_i32 r_asi, r_size, r_sign;

    r_asi = tcg_const_i32(GET_FIELD(insn, 19, 26));
    r_size = tcg_const_i32(size);
    r_sign = tcg_const_i32(sign);
    gen_helper_ld_asi(cpu_tmp64, addr, r_asi, r_size, r_sign);
    tcg_temp_free(r_sign);
    tcg_temp_free(r_size);
    tcg_temp_free(r_asi);
    tcg_gen_trunc_i64_tl(dst, cpu_tmp64);
}

static inline void gen_st_asi(TCGv src, TCGv addr, int insn, int size)
{
    TCGv_i32 r_asi, r_size;

    tcg_gen_extu_tl_i64(cpu_tmp64, src);
    r_asi = tcg_const_i32(GET_FIELD(insn, 19, 26));
    r_size = tcg_const_i32(size);
    gen_helper_st_asi(addr, cpu_tmp64, r_asi, r_size);
    tcg_temp_free(r_size);
    tcg_temp_free(r_asi);
}

static inline void gen_swap_asi(TCGv dst, TCGv addr, int insn)
{
    TCGv_i32 r_asi, r_size, r_sign;
    TCGv_i64 r_val;

    r_asi = tcg_const_i32(GET_FIELD(insn, 19, 26));
    r_size = tcg_const_i32(4);
    r_sign = tcg_const_i32(0);
    gen_helper_ld_asi(cpu_tmp64, addr, r_asi, r_size, r_sign);
    tcg_temp_free(r_sign);
    r_val = tcg_temp_new_i64();
    tcg_gen_extu_tl_i64(r_val, dst);
    gen_helper_st_asi(addr, r_val, r_asi, r_size);
    tcg_temp_free_i64(r_val);
    tcg_temp_free(r_size);
    tcg_temp_free(r_asi);
    tcg_gen_trunc_i64_tl(dst, cpu_tmp64);
}

static inline void gen_ldda_asi(TCGv hi, TCGv addr, int insn, int rd)
{
    TCGv_i32 r_asi, r_size, r_sign;

    r_asi = tcg_const_i32(GET_FIELD(insn, 19, 26));
    r_size = tcg_const_i32(8);
    r_sign = tcg_const_i32(0);
    gen_helper_ld_asi(cpu_tmp64, addr, r_asi, r_size, r_sign);
    tcg_temp_free(r_sign);
    tcg_temp_free(r_size);
    tcg_temp_free(r_asi);
    tcg_gen_trunc_i64_tl(cpu_tmp0, cpu_tmp64);
    gen_movl_TN_reg(rd + 1, cpu_tmp0);
    tcg_gen_shri_i64(cpu_tmp64, cpu_tmp64, 32);
    tcg_gen_trunc_i64_tl(hi, cpu_tmp64);
    gen_movl_TN_reg(rd, hi);
}

static inline void gen_stda_asi(TCGv hi, TCGv addr, int insn, int rd)
{
    TCGv_i32 r_asi, r_size;

    gen_movl_reg_TN(rd + 1, cpu_tmp0);
    tcg_gen_concat_tl_i64(cpu_tmp64, cpu_tmp0, hi);
    r_asi = tcg_const_i32(GET_FIELD(insn, 19, 26));
    r_size = tcg_const_i32(8);
    gen_helper_st_asi(addr, cpu_tmp64, r_asi, r_size);
    tcg_temp_free(r_size);
    tcg_temp_free(r_asi);
}

/* This function assumes that IS_IMM == 0. */
static inline void gen_cas_asi(TCGv dst, TCGv addr, TCGv val1, TCGv val2, int insn)
{
    int asi;
    TCGv r_asi;

    asi = GET_FIELD(insn, 19, 26);
    r_asi = tcg_const_i32(asi);
    gen_helper_cas_asi(dst, addr, val1, val2, r_asi);
    tcg_temp_free(r_asi);
}

static inline void gen_ldstub_asi(TCGv dst, TCGv addr, int insn)
{
    TCGv_i64 r_val;
    TCGv_i32 r_asi, r_size;

    gen_ld_asi(dst, addr, insn, 1, 0);

    r_val = tcg_const_i64(0xffULL);
    r_asi = tcg_const_i32(GET_FIELD(insn, 19, 26));
    r_size = tcg_const_i32(1);
    gen_helper_st_asi(addr, r_val, r_asi, r_size);
    tcg_temp_free_i32(r_size);
    tcg_temp_free_i32(r_asi);
    tcg_temp_free_i64(r_val);
}

static inline TCGv get_src1(unsigned int insn, TCGv def)
{
    TCGv r_rs1 = def;
    unsigned int rs1;

    rs1 = GET_FIELD(insn, 13, 17);
    if(rs1 == 0) {
        tcg_gen_movi_tl(def, 0);
    } else if(rs1 < 8) {
        r_rs1 = cpu_gregs[rs1];
    } else {
        tcg_gen_ld_tl(def, cpu_regwptr, (rs1 - 8) * sizeof(target_ulong));
    }
    return r_rs1;
}

static inline TCGv get_src2(unsigned int insn, TCGv def)
{
    TCGv r_rs2 = def;

    if(IS_IMM) { /* immediate */
        target_long simm = GET_FIELDs(insn, 19, 31);
        tcg_gen_movi_tl(def, simm);
    } else { /* register */
        unsigned int rs2 = GET_FIELD(insn, 27, 31);
        if(rs2 == 0) {
            tcg_gen_movi_tl(def, 0);
        } else if(rs2 < 8) {
            r_rs2 = cpu_gregs[rs2];
        } else {
            tcg_gen_ld_tl(def, cpu_regwptr, (rs2 - 8) * sizeof(target_ulong));
        }
    }
    return r_rs2;
}

#define CHECK_IU_FEATURE(dc, FEATURE)                  \
    if(!((dc)->def->features & CPU_FEATURE_##FEATURE)) \
        goto illegal_insn;
#define CHECK_FPU_FEATURE(dc, FEATURE)                 \
    if(!((dc)->def->features & CPU_FEATURE_##FEATURE)) \
        goto nfpu_insn;

static inline int reg_window_offset(int reg)
{
    //  Returns an offset to a particular register in the current register window,
    //  to be used with cpu_regwptr, which doesn't contain the global registers
    //  shared across all register windows.
    //  E. g. SP (R14) is at offset 14 - 8 (the number of global regs) = 6
    return reg - NB_SHARED_GLOBAL_REGS;
}

static inline void generate_stack_frame_announcement(DisasContext *dc, int type)
{
    if(unlikely(dc->base.guest_profile)) {
        TCGv frameptr = tcg_temp_new();
        TCGv stackptr = tcg_temp_new();
        tcg_gen_ld_tl(frameptr, cpu_regwptr, reg_window_offset(FP_32) * sizeof(target_ulong));
        tcg_gen_ld_tl(stackptr, cpu_regwptr, reg_window_offset(SP_32) * sizeof(target_ulong));
        if(type == STACK_FRAME_ADD) {
            gen_helper_announce_stack_pointer_change(cpu_pc, frameptr, stackptr);
        } else if(type == STACK_FRAME_POP) {
            gen_helper_announce_stack_pointer_change(cpu_pc, stackptr, frameptr);
        }
        tcg_temp_free(frameptr);
        tcg_temp_free(stackptr);
    }
}

/* before an instruction, dc->base.pc must be static */
static int disas_insn(CPUState *env, DisasContext *dc)
{
    unsigned int insn, opc, rs1, rs2, rd;
    TCGv cpu_src1, cpu_src2, cpu_tmp1, cpu_tmp2;
    target_long simm;

    insn = ldl_code(dc->base.pc);

    if(env->count_opcodes) {
        generate_opcode_count_increment(env, insn);
    }

    opc = GET_FIELD(insn, 0, 1);

    rd = GET_FIELD(insn, 2, 6);

    cpu_tmp1 = cpu_src1 = tcg_temp_new();
    cpu_tmp2 = cpu_src2 = tcg_temp_new();

    switch(opc) {
        case 0: /* branches/sethi */
        {
            unsigned int xop = GET_FIELD(insn, 7, 9);
            int32_t target;
            switch(xop) {
                case 0x7: /* CBN+x */
                {
                    goto ncp_insn;
                }
                case 0x2: /* BN+x */
                {
                    target = GET_FIELD(insn, 10, 31);
                    target = sign_extend(target, 22);
                    target <<= 2;
                    do_branch(dc, target, insn, 0, cpu_cond);
                    goto jmp_insn;
                }
                case 0x6: /* FBN+x */
                {
                    if(gen_trap_ifnofpu(dc, cpu_cond)) {
                        goto jmp_insn;
                    }
                    target = GET_FIELD(insn, 10, 31);
                    target = sign_extend(target, 22);
                    target <<= 2;
                    do_fbranch(dc, target, insn, 0, cpu_cond);
                    goto jmp_insn;
                }
                case 0x4:     /* SETHI */
                    if(rd) {  //  nop
                        uint32_t value = GET_FIELD(insn, 10, 31);
                        TCGv r_const;

                        r_const = tcg_const_tl(value << 10);
                        gen_movl_TN_reg(rd, r_const);
                        tcg_temp_free(r_const);
                    }
                    break;
                case 0x0: /* UNIMPL */
                default:
                    goto illegal_insn;
            }
            break;
        } break;
        case 1: /*CALL*/
        {
            target_long target = GET_FIELDs(insn, 2, 31) << 2;
            TCGv r_const;

            r_const = tcg_const_tl(dc->base.pc);
            gen_movl_TN_reg(15, r_const);
            tcg_temp_free(r_const);
            target += dc->base.pc;
            gen_mov_pc_npc(dc, cpu_cond);
            dc->npc = target;
        }
            goto jmp_insn;
        case 2: /* FPU & Logical Operations */
        {
            unsigned int xop = GET_FIELD(insn, 7, 12);
            if(xop == 0x3a) { /* generate trap */
                int cond;

                cpu_src1 = get_src1(insn, cpu_src1);
                if(IS_IMM) {
                    rs2 = GET_FIELD(insn, 25, 31);
                    tcg_gen_addi_tl(cpu_dst, cpu_src1, rs2);
                } else {
                    rs2 = GET_FIELD(insn, 27, 31);
                    if(rs2 != 0) {
                        gen_movl_reg_TN(rs2, cpu_src2);
                        tcg_gen_add_tl(cpu_dst, cpu_src1, cpu_src2);
                    } else {
                        tcg_gen_mov_tl(cpu_dst, cpu_src1);
                    }
                }

                cond = GET_FIELD(insn, 3, 6);
                if(cond == 0x8) { /* Trap Always */
                    save_state(dc, cpu_cond);
                    if((dc->def->features & CPU_FEATURE_HYPV) && supervisor(dc)) {
                        tcg_gen_andi_tl(cpu_dst, cpu_dst, UA2005_HTRAP_MASK);
                    } else {
                        tcg_gen_andi_tl(cpu_dst, cpu_dst, V8_TRAP_MASK);
                    }
                    tcg_gen_addi_tl(cpu_dst, cpu_dst, TT_TRAP);
                    tcg_gen_trunc_tl_i32(cpu_tmp32, cpu_dst);

                    gen_helper_raise_exception(cpu_tmp32);

                } else if(cond != 0) {
                    TCGv r_cond = tcg_temp_new();
                    int l1;
                    save_state(dc, cpu_cond);
                    gen_cond(r_cond, 0, cond, dc);
                    l1 = gen_new_label();
                    tcg_gen_brcondi_tl(TCG_COND_EQ, r_cond, 0, l1);

                    if((dc->def->features & CPU_FEATURE_HYPV) && supervisor(dc)) {
                        tcg_gen_andi_tl(cpu_dst, cpu_dst, UA2005_HTRAP_MASK);
                    } else {
                        tcg_gen_andi_tl(cpu_dst, cpu_dst, V8_TRAP_MASK);
                    }
                    tcg_gen_addi_tl(cpu_dst, cpu_dst, TT_TRAP);
                    tcg_gen_trunc_tl_i32(cpu_tmp32, cpu_dst);
                    gen_helper_raise_exception(cpu_tmp32);

                    gen_set_label(l1);
                    tcg_temp_free(r_cond);
                }
                gen_op_next_insn();
                gen_exit_tb_no_chaining(dc->base.tb);
                dc->base.is_jmp = DISAS_JUMP;
                goto jmp_insn;
            } else if(xop == 0x28) {
                rs1 = GET_FIELD(insn, 13, 17);
                switch(rs1) {
                    case 0:             /* rdy */
                    case 0x01 ... 0x0e: /* undefined in the SPARCv8
                                           manual, rdy on the microSPARC
                                           II */
                    case 0x0f:          /* stbar in the SPARCv8 manual,
                                           rdy on the microSPARC II */
                    case 0x10 ... 0x1f: /* implementation-dependent in the
                                           SPARCv8 manual, rdy on the
                                           microSPARC II */
                        /* RDASR %asr16-31 for a Leon3 processor */
                        /* Gaisler Research is assigned number 15 (0xF) */
                        /* as SPARC implementor’s identification. */
                        /* This value is hard-coded into bits 31:28 */
                        /* in the %psr register. */
                        /* The version number for LEON3 is 3, which */
                        /* is hardcoded in to bits 27:24 of the %psr. */

                        /* For an RDASR instruction with rs1 in the */
                        /* range 16...31, the following are */
                        /* implementation-dependent: the interpretation */
                        /* of bits 13:0 and 29:25 (rd) in the instruction, */
                        /* whether the instruction is privileged or not, */
                        /* and whether the instruction causes an */
                        /* illegal_instruction trap or not. */

                        if(dc->def->iu_version == 0xf3000000 && dc->def->features & CPU_FEATURE_ASR) {
                            if(rs1 > 15 && rs1 < 32) {
                                gen_movl_TN_reg(rd, cpu_asr[rs1 - 16]);
                                break;
                            }
                        }
                        /* rs1 == 0 is RDY */
                        gen_movl_TN_reg(rd, cpu_y);
                        break;
                    default:
                        goto illegal_insn;
                }
            } else if(xop == 0x29) { /* rdpsr / UA2005 rdhpr */
                if(!supervisor(dc)) {
                    goto priv_insn;
                }
                gen_helper_compute_psr();
                dc->cc_op = CC_OP_FLAGS;
                gen_helper_rdpsr(cpu_dst);
                gen_movl_TN_reg(rd, cpu_dst);
                break;
            } else if(xop == 0x2a) { /* rdwim */
                if(!supervisor(dc)) {
                    goto priv_insn;
                }
                tcg_gen_ext_i32_tl(cpu_tmp0, cpu_wim);
                gen_movl_TN_reg(rd, cpu_tmp0);
                break;
            } else if(xop == 0x2b) { /* rdtbr */
                if(!supervisor(dc)) {
                    goto priv_insn;
                }
                gen_movl_TN_reg(rd, cpu_tbr);
                break;
            } else if(xop == 0x34) { /* FPU Operations */
                if(gen_trap_ifnofpu(dc, cpu_cond)) {
                    goto jmp_insn;
                }
                gen_op_clear_ieee_excp_and_FTT();
                rs1 = GET_FIELD(insn, 13, 17);
                rs2 = GET_FIELD(insn, 27, 31);
                xop = GET_FIELD(insn, 18, 26);
                save_state(dc, cpu_cond);
                switch(xop) {
                    case 0x1: /* fmovs */
                        tcg_gen_mov_i32(cpu_fpr[rd], cpu_fpr[rs2]);
                        break;
                    case 0x5: /* fnegs */
                        gen_helper_fnegs(cpu_fpr[rd], cpu_fpr[rs2]);
                        break;
                    case 0x9: /* fabss */
                        gen_helper_fabss(cpu_fpr[rd], cpu_fpr[rs2]);
                        break;
                    case 0x29: /* fsqrts */
                        CHECK_FPU_FEATURE(dc, FSQRT);
                        gen_clear_float_exceptions();
                        gen_helper_fsqrts(cpu_tmp32, cpu_fpr[rs2]);
                        gen_helper_check_ieee_exceptions();
                        tcg_gen_mov_i32(cpu_fpr[rd], cpu_tmp32);
                        break;
                    case 0x2a: /* fsqrtd */
                        CHECK_FPU_FEATURE(dc, FSQRT);
                        gen_op_load_fpr_DT1(DFPREG(rs2));
                        gen_clear_float_exceptions();
                        gen_helper_fsqrtd();
                        gen_helper_check_ieee_exceptions();
                        gen_op_store_DT0_fpr(DFPREG(rd));
                        break;
                    case 0x2b: /* fsqrtq */
                        CHECK_FPU_FEATURE(dc, FLOAT128);
                        gen_op_load_fpr_QT1(QFPREG(rs2));
                        gen_clear_float_exceptions();
                        gen_helper_fsqrtq();
                        gen_helper_check_ieee_exceptions();
                        gen_op_store_QT0_fpr(QFPREG(rd));
                        break;
                    case 0x41: /* fadds */
                        gen_clear_float_exceptions();
                        gen_helper_fadds(cpu_tmp32, cpu_fpr[rs1], cpu_fpr[rs2]);
                        gen_helper_check_ieee_exceptions();
                        tcg_gen_mov_i32(cpu_fpr[rd], cpu_tmp32);
                        break;
                    case 0x42: /* faddd */
                        gen_op_load_fpr_DT0(DFPREG(rs1));
                        gen_op_load_fpr_DT1(DFPREG(rs2));
                        gen_clear_float_exceptions();
                        gen_helper_faddd();
                        gen_helper_check_ieee_exceptions();
                        gen_op_store_DT0_fpr(DFPREG(rd));
                        break;
                    case 0x43: /* faddq */
                        CHECK_FPU_FEATURE(dc, FLOAT128);
                        gen_op_load_fpr_QT0(QFPREG(rs1));
                        gen_op_load_fpr_QT1(QFPREG(rs2));
                        gen_clear_float_exceptions();
                        gen_helper_faddq();
                        gen_helper_check_ieee_exceptions();
                        gen_op_store_QT0_fpr(QFPREG(rd));
                        break;
                    case 0x45: /* fsubs */
                        gen_clear_float_exceptions();
                        gen_helper_fsubs(cpu_tmp32, cpu_fpr[rs1], cpu_fpr[rs2]);
                        gen_helper_check_ieee_exceptions();
                        tcg_gen_mov_i32(cpu_fpr[rd], cpu_tmp32);
                        break;
                    case 0x46: /* fsubd */
                        gen_op_load_fpr_DT0(DFPREG(rs1));
                        gen_op_load_fpr_DT1(DFPREG(rs2));
                        gen_clear_float_exceptions();
                        gen_helper_fsubd();
                        gen_helper_check_ieee_exceptions();
                        gen_op_store_DT0_fpr(DFPREG(rd));
                        break;
                    case 0x47: /* fsubq */
                        CHECK_FPU_FEATURE(dc, FLOAT128);
                        gen_op_load_fpr_QT0(QFPREG(rs1));
                        gen_op_load_fpr_QT1(QFPREG(rs2));
                        gen_clear_float_exceptions();
                        gen_helper_fsubq();
                        gen_helper_check_ieee_exceptions();
                        gen_op_store_QT0_fpr(QFPREG(rd));
                        break;
                    case 0x49: /* fmuls */
                        CHECK_FPU_FEATURE(dc, FMUL);
                        gen_clear_float_exceptions();
                        gen_helper_fmuls(cpu_tmp32, cpu_fpr[rs1], cpu_fpr[rs2]);
                        gen_helper_check_ieee_exceptions();
                        tcg_gen_mov_i32(cpu_fpr[rd], cpu_tmp32);
                        break;
                    case 0x4a: /* fmuld */
                        CHECK_FPU_FEATURE(dc, FMUL);
                        gen_op_load_fpr_DT0(DFPREG(rs1));
                        gen_op_load_fpr_DT1(DFPREG(rs2));
                        gen_clear_float_exceptions();
                        gen_helper_fmuld();
                        gen_helper_check_ieee_exceptions();
                        gen_op_store_DT0_fpr(DFPREG(rd));
                        break;
                    case 0x4b: /* fmulq */
                        CHECK_FPU_FEATURE(dc, FLOAT128);
                        CHECK_FPU_FEATURE(dc, FMUL);
                        gen_op_load_fpr_QT0(QFPREG(rs1));
                        gen_op_load_fpr_QT1(QFPREG(rs2));
                        gen_clear_float_exceptions();
                        gen_helper_fmulq();
                        gen_helper_check_ieee_exceptions();
                        gen_op_store_QT0_fpr(QFPREG(rd));
                        break;
                    case 0x4d: /* fdivs */
                        gen_clear_float_exceptions();
                        gen_helper_fdivs(cpu_tmp32, cpu_fpr[rs1], cpu_fpr[rs2]);
                        gen_helper_check_ieee_exceptions();
                        tcg_gen_mov_i32(cpu_fpr[rd], cpu_tmp32);
                        break;
                    case 0x4e: /* fdivd */
                        gen_op_load_fpr_DT0(DFPREG(rs1));
                        gen_op_load_fpr_DT1(DFPREG(rs2));
                        gen_clear_float_exceptions();
                        gen_helper_fdivd();
                        gen_helper_check_ieee_exceptions();
                        gen_op_store_DT0_fpr(DFPREG(rd));
                        break;
                    case 0x4f: /* fdivq */
                        CHECK_FPU_FEATURE(dc, FLOAT128);
                        gen_op_load_fpr_QT0(QFPREG(rs1));
                        gen_op_load_fpr_QT1(QFPREG(rs2));
                        gen_clear_float_exceptions();
                        gen_helper_fdivq();
                        gen_helper_check_ieee_exceptions();
                        gen_op_store_QT0_fpr(QFPREG(rd));
                        break;
                    case 0x69: /* fsmuld */
                        CHECK_FPU_FEATURE(dc, FSMULD);
                        gen_clear_float_exceptions();
                        gen_helper_fsmuld(cpu_fpr[rs1], cpu_fpr[rs2]);
                        gen_helper_check_ieee_exceptions();
                        gen_op_store_DT0_fpr(DFPREG(rd));
                        break;
                    case 0x6e: /* fdmulq */
                        CHECK_FPU_FEATURE(dc, FLOAT128);
                        gen_op_load_fpr_DT0(DFPREG(rs1));
                        gen_op_load_fpr_DT1(DFPREG(rs2));
                        gen_clear_float_exceptions();
                        gen_helper_fdmulq();
                        gen_helper_check_ieee_exceptions();
                        gen_op_store_QT0_fpr(QFPREG(rd));
                        break;
                    case 0xc4: /* fitos */
                        gen_clear_float_exceptions();
                        gen_helper_fitos(cpu_tmp32, cpu_fpr[rs2]);
                        gen_helper_check_ieee_exceptions();
                        tcg_gen_mov_i32(cpu_fpr[rd], cpu_tmp32);
                        break;
                    case 0xc6: /* fdtos */
                        gen_op_load_fpr_DT1(DFPREG(rs2));
                        gen_clear_float_exceptions();
                        gen_helper_fdtos(cpu_tmp32);
                        gen_helper_check_ieee_exceptions();
                        tcg_gen_mov_i32(cpu_fpr[rd], cpu_tmp32);
                        break;
                    case 0xc7: /* fqtos */
                        CHECK_FPU_FEATURE(dc, FLOAT128);
                        gen_op_load_fpr_QT1(QFPREG(rs2));
                        gen_clear_float_exceptions();
                        gen_helper_fqtos(cpu_tmp32);
                        gen_helper_check_ieee_exceptions();
                        tcg_gen_mov_i32(cpu_fpr[rd], cpu_tmp32);
                        break;
                    case 0xc8: /* fitod */
                        gen_helper_fitod(cpu_fpr[rs2]);
                        gen_op_store_DT0_fpr(DFPREG(rd));
                        break;
                    case 0xc9: /* fstod */
                        gen_helper_fstod(cpu_fpr[rs2]);
                        gen_op_store_DT0_fpr(DFPREG(rd));
                        break;
                    case 0xcb: /* fqtod */
                        CHECK_FPU_FEATURE(dc, FLOAT128);
                        gen_op_load_fpr_QT1(QFPREG(rs2));
                        gen_clear_float_exceptions();
                        gen_helper_fqtod();
                        gen_helper_check_ieee_exceptions();
                        gen_op_store_DT0_fpr(DFPREG(rd));
                        break;
                    case 0xcc: /* fitoq */
                        CHECK_FPU_FEATURE(dc, FLOAT128);
                        gen_helper_fitoq(cpu_fpr[rs2]);
                        gen_op_store_QT0_fpr(QFPREG(rd));
                        break;
                    case 0xcd: /* fstoq */
                        CHECK_FPU_FEATURE(dc, FLOAT128);
                        gen_helper_fstoq(cpu_fpr[rs2]);
                        gen_op_store_QT0_fpr(QFPREG(rd));
                        break;
                    case 0xce: /* fdtoq */
                        CHECK_FPU_FEATURE(dc, FLOAT128);
                        gen_op_load_fpr_DT1(DFPREG(rs2));
                        gen_helper_fdtoq();
                        gen_op_store_QT0_fpr(QFPREG(rd));
                        break;
                    case 0xd1: /* fstoi */
                        gen_clear_float_exceptions();
                        gen_helper_fstoi(cpu_tmp32, cpu_fpr[rs2]);
                        gen_helper_check_ieee_exceptions();
                        tcg_gen_mov_i32(cpu_fpr[rd], cpu_tmp32);
                        break;
                    case 0xd2: /* fdtoi */
                        gen_op_load_fpr_DT1(DFPREG(rs2));
                        gen_clear_float_exceptions();
                        gen_helper_fdtoi(cpu_tmp32);
                        gen_helper_check_ieee_exceptions();
                        tcg_gen_mov_i32(cpu_fpr[rd], cpu_tmp32);
                        break;
                    case 0xd3: /* fqtoi */
                        CHECK_FPU_FEATURE(dc, FLOAT128);
                        gen_op_load_fpr_QT1(QFPREG(rs2));
                        gen_clear_float_exceptions();
                        gen_helper_fqtoi(cpu_tmp32);
                        gen_helper_check_ieee_exceptions();
                        tcg_gen_mov_i32(cpu_fpr[rd], cpu_tmp32);
                        break;
                    default:
                        goto illegal_insn;
                }
            } else if(xop == 0x35) { /* FPU Operations */
                if(gen_trap_ifnofpu(dc, cpu_cond)) {
                    goto jmp_insn;
                }
                gen_op_clear_ieee_excp_and_FTT();
                rs1 = GET_FIELD(insn, 13, 17);
                rs2 = GET_FIELD(insn, 27, 31);
                xop = GET_FIELD(insn, 18, 26);
                save_state(dc, cpu_cond);
                switch(xop) {
                    case 0x51: /* fcmps */
                        gen_op_fcmps(rd & 3, cpu_fpr[rs1], cpu_fpr[rs2]);
                        break;
                    case 0x52: /* fcmpd */
                        gen_op_load_fpr_DT0(DFPREG(rs1));
                        gen_op_load_fpr_DT1(DFPREG(rs2));
                        gen_op_fcmpd(rd & 3);
                        break;
                    case 0x53: /* fcmpq */
                        CHECK_FPU_FEATURE(dc, FLOAT128);
                        gen_op_load_fpr_QT0(QFPREG(rs1));
                        gen_op_load_fpr_QT1(QFPREG(rs2));
                        gen_op_fcmpq(rd & 3);
                        break;
                    case 0x55: /* fcmpes */
                        gen_op_fcmpes(rd & 3, cpu_fpr[rs1], cpu_fpr[rs2]);
                        break;
                    case 0x56: /* fcmped */
                        gen_op_load_fpr_DT0(DFPREG(rs1));
                        gen_op_load_fpr_DT1(DFPREG(rs2));
                        gen_op_fcmped(rd & 3);
                        break;
                    case 0x57: /* fcmpeq */
                        CHECK_FPU_FEATURE(dc, FLOAT128);
                        gen_op_load_fpr_QT0(QFPREG(rs1));
                        gen_op_load_fpr_QT1(QFPREG(rs2));
                        gen_op_fcmpeq(rd & 3);
                        break;
                    default:
                        goto illegal_insn;
                }
            } else if(xop == 0x2) {
                //  clr/mov shortcut

                rs1 = GET_FIELD(insn, 13, 17);
                if(rs1 == 0) {
                    //  or %g0, x, y -> mov T0, x; mov y, T0
                    if(IS_IMM) { /* immediate */
                        TCGv r_const;

                        simm = GET_FIELDs(insn, 19, 31);
                        r_const = tcg_const_tl(simm);
                        gen_movl_TN_reg(rd, r_const);
                        tcg_temp_free(r_const);
                    } else { /* register */
                        rs2 = GET_FIELD(insn, 27, 31);
                        gen_movl_reg_TN(rs2, cpu_dst);
                        gen_movl_TN_reg(rd, cpu_dst);
                    }
                } else {
                    cpu_src1 = get_src1(insn, cpu_src1);
                    if(IS_IMM) { /* immediate */
                        simm = GET_FIELDs(insn, 19, 31);
                        tcg_gen_ori_tl(cpu_dst, cpu_src1, simm);
                        gen_movl_TN_reg(rd, cpu_dst);
                    } else { /* register */
                        //  or x, %g0, y -> mov T1, x; mov y, T1
                        rs2 = GET_FIELD(insn, 27, 31);
                        if(rs2 != 0) {
                            gen_movl_reg_TN(rs2, cpu_src2);
                            tcg_gen_or_tl(cpu_dst, cpu_src1, cpu_src2);
                            gen_movl_TN_reg(rd, cpu_dst);
                        } else {
                            gen_movl_TN_reg(rd, cpu_src1);
                        }
                    }
                }
            } else if(xop < 0x36) {
                if(xop < 0x20) {
                    cpu_src1 = get_src1(insn, cpu_src1);
                    cpu_src2 = get_src2(insn, cpu_src2);
                    switch(xop & ~0x10) {
                        case 0x0: /* add */
                            if(IS_IMM) {
                                simm = GET_FIELDs(insn, 19, 31);
                                if(xop & 0x10) {
                                    gen_op_addi_cc(cpu_dst, cpu_src1, simm);
                                    tcg_gen_movi_i32(cpu_cc_op, CC_OP_ADD);
                                    dc->cc_op = CC_OP_ADD;
                                } else {
                                    tcg_gen_addi_tl(cpu_dst, cpu_src1, simm);
                                }
                            } else {
                                if(xop & 0x10) {
                                    gen_op_add_cc(cpu_dst, cpu_src1, cpu_src2);
                                    tcg_gen_movi_i32(cpu_cc_op, CC_OP_ADD);
                                    dc->cc_op = CC_OP_ADD;
                                } else {
                                    tcg_gen_add_tl(cpu_dst, cpu_src1, cpu_src2);
                                }
                            }
                            break;
                        case 0x1: /* and */
                            if(IS_IMM) {
                                simm = GET_FIELDs(insn, 19, 31);
                                tcg_gen_andi_tl(cpu_dst, cpu_src1, simm);
                            } else {
                                tcg_gen_and_tl(cpu_dst, cpu_src1, cpu_src2);
                            }
                            if(xop & 0x10) {
                                tcg_gen_mov_tl(cpu_cc_dst, cpu_dst);
                                tcg_gen_movi_i32(cpu_cc_op, CC_OP_LOGIC);
                                dc->cc_op = CC_OP_LOGIC;
                            }
                            break;
                        case 0x2: /* or */
                            if(IS_IMM) {
                                simm = GET_FIELDs(insn, 19, 31);
                                tcg_gen_ori_tl(cpu_dst, cpu_src1, simm);
                            } else {
                                tcg_gen_or_tl(cpu_dst, cpu_src1, cpu_src2);
                            }
                            if(xop & 0x10) {
                                tcg_gen_mov_tl(cpu_cc_dst, cpu_dst);
                                tcg_gen_movi_i32(cpu_cc_op, CC_OP_LOGIC);
                                dc->cc_op = CC_OP_LOGIC;
                            }
                            break;
                        case 0x3: /* xor */
                            if(IS_IMM) {
                                simm = GET_FIELDs(insn, 19, 31);
                                tcg_gen_xori_tl(cpu_dst, cpu_src1, simm);
                            } else {
                                tcg_gen_xor_tl(cpu_dst, cpu_src1, cpu_src2);
                            }
                            if(xop & 0x10) {
                                tcg_gen_mov_tl(cpu_cc_dst, cpu_dst);
                                tcg_gen_movi_i32(cpu_cc_op, CC_OP_LOGIC);
                                dc->cc_op = CC_OP_LOGIC;
                            }
                            break;
                        case 0x4: /* sub */
                            if(IS_IMM) {
                                simm = GET_FIELDs(insn, 19, 31);
                                if(xop & 0x10) {
                                    gen_op_subi_cc(cpu_dst, cpu_src1, simm, dc);
                                } else {
                                    tcg_gen_subi_tl(cpu_dst, cpu_src1, simm);
                                }
                            } else {
                                if(xop & 0x10) {
                                    gen_op_sub_cc(cpu_dst, cpu_src1, cpu_src2);
                                    tcg_gen_movi_i32(cpu_cc_op, CC_OP_SUB);
                                    dc->cc_op = CC_OP_SUB;
                                } else {
                                    tcg_gen_sub_tl(cpu_dst, cpu_src1, cpu_src2);
                                }
                            }
                            break;
                        case 0x5: /* andn */
                            if(IS_IMM) {
                                simm = GET_FIELDs(insn, 19, 31);
                                tcg_gen_andi_tl(cpu_dst, cpu_src1, ~simm);
                            } else {
                                tcg_gen_andc_tl(cpu_dst, cpu_src1, cpu_src2);
                            }
                            if(xop & 0x10) {
                                tcg_gen_mov_tl(cpu_cc_dst, cpu_dst);
                                tcg_gen_movi_i32(cpu_cc_op, CC_OP_LOGIC);
                                dc->cc_op = CC_OP_LOGIC;
                            }
                            break;
                        case 0x6: /* orn */
                            if(IS_IMM) {
                                simm = GET_FIELDs(insn, 19, 31);
                                tcg_gen_ori_tl(cpu_dst, cpu_src1, ~simm);
                            } else {
                                tcg_gen_orc_tl(cpu_dst, cpu_src1, cpu_src2);
                            }
                            if(xop & 0x10) {
                                tcg_gen_mov_tl(cpu_cc_dst, cpu_dst);
                                tcg_gen_movi_i32(cpu_cc_op, CC_OP_LOGIC);
                                dc->cc_op = CC_OP_LOGIC;
                            }
                            break;
                        case 0x7: /* xorn */
                            if(IS_IMM) {
                                simm = GET_FIELDs(insn, 19, 31);
                                tcg_gen_xori_tl(cpu_dst, cpu_src1, ~simm);
                            } else {
                                tcg_gen_not_tl(cpu_tmp0, cpu_src2);
                                tcg_gen_xor_tl(cpu_dst, cpu_src1, cpu_tmp0);
                            }
                            if(xop & 0x10) {
                                tcg_gen_mov_tl(cpu_cc_dst, cpu_dst);
                                tcg_gen_movi_i32(cpu_cc_op, CC_OP_LOGIC);
                                dc->cc_op = CC_OP_LOGIC;
                            }
                            break;
                        case 0x8: /* addx */
                            gen_op_addx_int(dc, cpu_dst, cpu_src1, cpu_src2, (xop & 0x10));
                            break;
                        case 0xa: /* umul */
                            CHECK_IU_FEATURE(dc, MUL);
                            gen_op_umul(cpu_dst, cpu_src1, cpu_src2);
                            if(xop & 0x10) {
                                tcg_gen_mov_tl(cpu_cc_dst, cpu_dst);
                                tcg_gen_movi_i32(cpu_cc_op, CC_OP_LOGIC);
                                dc->cc_op = CC_OP_LOGIC;
                            }
                            break;
                        case 0xb: /* smul */
                            CHECK_IU_FEATURE(dc, MUL);
                            gen_op_smul(cpu_dst, cpu_src1, cpu_src2);
                            if(xop & 0x10) {
                                tcg_gen_mov_tl(cpu_cc_dst, cpu_dst);
                                tcg_gen_movi_i32(cpu_cc_op, CC_OP_LOGIC);
                                dc->cc_op = CC_OP_LOGIC;
                            }
                            break;
                        case 0xc: /* subx */
                            gen_op_subx_int(dc, cpu_dst, cpu_src1, cpu_src2, (xop & 0x10));
                            break;
                        case 0xe: /* udiv */
                            CHECK_IU_FEATURE(dc, DIV);
                            if(xop & 0x10) {
                                gen_helper_udiv_cc(cpu_dst, cpu_src1, cpu_src2);
                                dc->cc_op = CC_OP_DIV;
                            } else {
                                gen_helper_udiv(cpu_dst, cpu_src1, cpu_src2);
                            }
                            break;
                        case 0xf: /* sdiv */
                            CHECK_IU_FEATURE(dc, DIV);
                            if(xop & 0x10) {
                                gen_helper_sdiv_cc(cpu_dst, cpu_src1, cpu_src2);
                                dc->cc_op = CC_OP_DIV;
                            } else {
                                gen_helper_sdiv(cpu_dst, cpu_src1, cpu_src2);
                            }
                            break;
                        default:
                            goto illegal_insn;
                    }
                    gen_movl_TN_reg(rd, cpu_dst);
                } else {
                    cpu_src1 = get_src1(insn, cpu_src1);
                    cpu_src2 = get_src2(insn, cpu_src2);
                    switch(xop) {
                        case 0x20: /* taddcc */
                            gen_op_tadd_cc(cpu_dst, cpu_src1, cpu_src2);
                            gen_movl_TN_reg(rd, cpu_dst);
                            tcg_gen_movi_i32(cpu_cc_op, CC_OP_TADD);
                            dc->cc_op = CC_OP_TADD;
                            break;
                        case 0x21: /* tsubcc */
                            gen_op_tsub_cc(cpu_dst, cpu_src1, cpu_src2);
                            gen_movl_TN_reg(rd, cpu_dst);
                            tcg_gen_movi_i32(cpu_cc_op, CC_OP_TSUB);
                            dc->cc_op = CC_OP_TSUB;
                            break;
                        case 0x22: /* taddcctv */
                            save_state(dc, cpu_cond);
                            gen_op_tadd_ccTV(cpu_dst, cpu_src1, cpu_src2);
                            gen_movl_TN_reg(rd, cpu_dst);
                            tcg_gen_movi_i32(cpu_cc_op, CC_OP_TADDTV);
                            dc->cc_op = CC_OP_TADDTV;
                            break;
                        case 0x23: /* tsubcctv */
                            save_state(dc, cpu_cond);
                            gen_op_tsub_ccTV(cpu_dst, cpu_src1, cpu_src2);
                            gen_movl_TN_reg(rd, cpu_dst);
                            tcg_gen_movi_i32(cpu_cc_op, CC_OP_TSUBTV);
                            dc->cc_op = CC_OP_TSUBTV;
                            break;
                        case 0x24: /* mulscc */
                            gen_helper_compute_psr();
                            gen_op_mulscc(cpu_dst, cpu_src1, cpu_src2);
                            gen_movl_TN_reg(rd, cpu_dst);
                            tcg_gen_movi_i32(cpu_cc_op, CC_OP_ADD);
                            dc->cc_op = CC_OP_ADD;
                            break;
                        case 0x25:       /* sll */
                            if(IS_IMM) { /* immediate */
                                simm = GET_FIELDs(insn, 20, 31);
                                tcg_gen_shli_tl(cpu_dst, cpu_src1, simm & 0x1f);
                            } else { /* register */
                                tcg_gen_andi_tl(cpu_tmp0, cpu_src2, 0x1f);
                                tcg_gen_shl_tl(cpu_dst, cpu_src1, cpu_tmp0);
                            }
                            gen_movl_TN_reg(rd, cpu_dst);
                            break;
                        case 0x26:       /* srl */
                            if(IS_IMM) { /* immediate */
                                simm = GET_FIELDs(insn, 20, 31);
                                tcg_gen_shri_tl(cpu_dst, cpu_src1, simm & 0x1f);
                            } else { /* register */
                                tcg_gen_andi_tl(cpu_tmp0, cpu_src2, 0x1f);
                                tcg_gen_shr_tl(cpu_dst, cpu_src1, cpu_tmp0);
                            }
                            gen_movl_TN_reg(rd, cpu_dst);
                            break;
                        case 0x27:       /* sra */
                            if(IS_IMM) { /* immediate */
                                simm = GET_FIELDs(insn, 20, 31);
                                tcg_gen_sari_tl(cpu_dst, cpu_src1, simm & 0x1f);
                            } else { /* register */
                                tcg_gen_andi_tl(cpu_tmp0, cpu_src2, 0x1f);
                                tcg_gen_sar_tl(cpu_dst, cpu_src1, cpu_tmp0);
                            }
                            gen_movl_TN_reg(rd, cpu_dst);
                            break;
                        case 0x30: {
                            switch(rd) {
                                case 0: /* wry */
                                    tcg_gen_xor_tl(cpu_tmp0, cpu_src1, cpu_src2);
                                    tcg_gen_andi_tl(cpu_y, cpu_tmp0, 0xffffffff);
                                    break;
                                case 0x01 ... 0x0f: /* undefined in the
                                                       SPARCv8 manual, nop
                                                       on the microSPARC
                                                       II */
                                    break;
                                case 0x10 ... 0x1f:
                                    /* implementation-dependent
                                       in the SPARCv8
                                       manual, nop on the
                                       microSPARC II */
                                    /* WRASR %asr16-31 for a Leon3 processor */
                                    /* Gaisler Research is assigned number 15 (0xF) */
                                    /* as SPARC implementor’s identification. */
                                    /* This value is hard-coded into bits 31:28 */
                                    /* in the %psr register. */
                                    /* The version number for LEON3 is 3, which */
                                    /* is hardcoded in to bits 27:24 of the %psr. */
                                    if(dc->def->iu_version == 0xf3000000 && dc->def->features & CPU_FEATURE_ASR) {
                                        tcg_gen_xor_tl(cpu_tmp0, cpu_src1, cpu_src2);
                                        tcg_gen_andi_tl(cpu_asr[rd - 16], cpu_tmp0, 0xffffffff);

                                        /* WRASR to ASR19 */
                                        /* Power-down instruction for Leon3 */
                                        if(rd == 0x13) {
                                            save_state(dc, cpu_cond);
                                            gen_helper_power_down();
                                        }
                                    }
                                    break;
                                default:
                                    goto illegal_insn;
                            }
                        } break;
                        case 0x31: /* wrpsr */
                        {
                            if(!supervisor(dc)) {
                                goto priv_insn;
                            }
                            tcg_gen_xor_tl(cpu_dst, cpu_src1, cpu_src2);
                            gen_helper_wrpsr(cpu_dst);
                            tcg_gen_movi_i32(cpu_cc_op, CC_OP_FLAGS);
                            dc->cc_op = CC_OP_FLAGS;
                            save_state(dc, cpu_cond);
                            gen_op_next_insn();
                            gen_exit_tb_no_chaining(dc->base.tb);
                            dc->base.is_jmp = DISAS_JUMP;
                        } break;
                        case 0x32: /* wrwim */
                        {
                            if(!supervisor(dc)) {
                                goto priv_insn;
                            }
                            tcg_gen_xor_tl(cpu_tmp0, cpu_src1, cpu_src2);
                            tcg_gen_trunc_tl_i32(cpu_tmp32, cpu_tmp0);
                            if(dc->def->nwindows != 32) {
                                tcg_gen_andi_tl(cpu_tmp32, cpu_tmp32, (1 << dc->def->nwindows) - 1);
                            }
                            tcg_gen_mov_i32(cpu_wim, cpu_tmp32);
                        } break;
                        case 0x33: /* wrtbr, UA2005 wrhpr */
                        {
                            if(!supervisor(dc)) {
                                goto priv_insn;
                            }
                            tcg_gen_xor_tl(cpu_tbr, cpu_src1, cpu_src2);
                        } break;
                        default:
                            goto illegal_insn;
                    }
                }
            } else if(xop == 0x36) { /* UltraSparc shutdown, VIS, V8 CPop1 */
                goto ncp_insn;
            } else if(xop == 0x37) { /* V8 CPop2 */
                goto ncp_insn;
            } else {
                cpu_src1 = get_src1(insn, cpu_src1);
                if(IS_IMM) { /* immediate */
                    simm = GET_FIELDs(insn, 19, 31);
                    tcg_gen_addi_tl(cpu_dst, cpu_src1, simm);
                } else { /* register */
                    rs2 = GET_FIELD(insn, 27, 31);
                    if(rs2) {
                        gen_movl_reg_TN(rs2, cpu_src2);
                        tcg_gen_add_tl(cpu_dst, cpu_src1, cpu_src2);
                    } else {
                        tcg_gen_mov_tl(cpu_dst, cpu_src1);
                    }
                }
                switch(xop) {
                    case 0x38: /* jmpl */
                    {
                        TCGv r_pc;
                        TCGv_i32 r_const;

                        r_pc = tcg_const_tl(dc->base.pc);
                        gen_movl_TN_reg(rd, r_pc);
                        tcg_temp_free(r_pc);
                        gen_mov_pc_npc(dc, cpu_cond);
                        r_const = tcg_const_i32(3);
                        gen_helper_check_align(cpu_dst, r_const);
                        tcg_temp_free_i32(r_const);
                        tcg_gen_mov_tl(cpu_npc, cpu_dst);
                        dc->npc = DYNAMIC_PC;
                    }
                        goto jmp_insn;
                    case 0x39: /* rett */
                    {
                        TCGv_i32 r_const;

                        if(!supervisor(dc)) {
                            goto priv_insn;
                        }
                        gen_mov_pc_npc(dc, cpu_cond);
                        r_const = tcg_const_i32(3);
                        gen_helper_check_align(cpu_dst, r_const);
                        tcg_temp_free_i32(r_const);
                        tcg_gen_mov_tl(cpu_npc, cpu_dst);
                        dc->npc = DYNAMIC_PC;
                        gen_helper_rett();
                    }
                        goto jmp_insn;
                    case 0x3b: /* flush */
                        if(!((dc)->def->features & CPU_FEATURE_FLUSH)) {
                            goto unimp_flush;
                        }
                        /* nop */
                        break;
                    case 0x3c: /* save */
                        save_state(dc, cpu_cond);
                        gen_helper_save();
                        gen_movl_TN_reg(rd, cpu_dst);
                        generate_stack_frame_announcement(dc, STACK_FRAME_ADD);
                        break;
                    case 0x3d: /* restore */
                        generate_stack_frame_announcement(dc, STACK_FRAME_POP);
                        save_state(dc, cpu_cond);
                        gen_helper_restore();
                        gen_movl_TN_reg(rd, cpu_dst);
                        break;
                    default:
                        goto illegal_insn;
                }
            }
            break;
        } break;
        case 3: /* load/store instructions */
        {
            unsigned int xop = GET_FIELD(insn, 7, 12);

            /* flush pending conditional evaluations before exposing
               cpu state */
            if(dc->cc_op != CC_OP_FLAGS) {
                dc->cc_op = CC_OP_FLAGS;
                gen_helper_compute_psr();
            }
            cpu_src1 = get_src1(insn, cpu_src1);
            if(xop == 0x3c || xop == 0x3e) {
                rs2 = GET_FIELD(insn, 27, 31);
                gen_movl_reg_TN(rs2, cpu_src2);
                tcg_gen_mov_tl(cpu_addr, cpu_src1);
            } else if(IS_IMM) { /* immediate */
                simm = GET_FIELDs(insn, 19, 31);
                tcg_gen_addi_tl(cpu_addr, cpu_src1, simm);
            } else { /* register */
                rs2 = GET_FIELD(insn, 27, 31);
                if(rs2 != 0) {
                    gen_movl_reg_TN(rs2, cpu_src2);
                    tcg_gen_add_tl(cpu_addr, cpu_src1, cpu_src2);
                } else {
                    tcg_gen_mov_tl(cpu_addr, cpu_src1);
                }
            }
            if(xop < 4 || (xop > 7 && xop < 0x14 && xop != 0x0e) || (xop > 0x17 && xop <= 0x1d) || (xop > 0x2c && xop <= 0x33) ||
               xop == 0x1f || xop == 0x3d) {
                switch(xop) {
                    case 0x0: /* ld, load unsigned word */
                        tcg_gen_qemu_ld32u(cpu_val, cpu_addr, dc->base.mem_idx);
                        break;
                    case 0x1: /* ldub, load unsigned byte */
                        tcg_gen_qemu_ld8u(cpu_val, cpu_addr, dc->base.mem_idx);
                        break;
                    case 0x2: /* lduh, load unsigned halfword */
                        tcg_gen_qemu_ld16u(cpu_val, cpu_addr, dc->base.mem_idx);
                        break;
                    case 0x3: /* ldd, load double word */
                        if(rd & 1) {
                            goto illegal_insn;
                        } else {
                            TCGv_i32 r_const;

                            save_state(dc, cpu_cond);
                            r_const = tcg_const_i32(7);
                            gen_helper_check_align(cpu_addr, r_const);  //  XXX remove
                            tcg_temp_free_i32(r_const);
                            tcg_gen_qemu_ld64(cpu_tmp64, cpu_addr, dc->base.mem_idx);
                            tcg_gen_trunc_i64_tl(cpu_tmp0, cpu_tmp64);
                            tcg_gen_andi_tl(cpu_tmp0, cpu_tmp0, 0xffffffffULL);
                            gen_movl_TN_reg(rd + 1, cpu_tmp0);
                            tcg_gen_shri_i64(cpu_tmp64, cpu_tmp64, 32);
                            tcg_gen_trunc_i64_tl(cpu_val, cpu_tmp64);
                            tcg_gen_andi_tl(cpu_val, cpu_val, 0xffffffffULL);
                        }
                        break;
                    case 0x9: /* ldsb, load signed byte */
                        tcg_gen_qemu_ld8s(cpu_val, cpu_addr, dc->base.mem_idx);
                        break;
                    case 0xa: /* ldsh, load signed halfword */
                        tcg_gen_qemu_ld16s(cpu_val, cpu_addr, dc->base.mem_idx);
                        break;
                    case 0xd: /* ldstub -- XXX: should be atomically */
                    {
                        tcg_gen_qemu_ld8s(cpu_val, cpu_addr, dc->base.mem_idx);
                        gen_helper_ldstub(cpu_val, cpu_addr);
                    } break;
                    case 0x0f: /* swap, swap register with memory. Also
                                  atomically */
                        CHECK_IU_FEATURE(dc, SWAP);
                        gen_movl_reg_TN(rd, cpu_val);
                        gen_helper_swap(cpu_val, cpu_val, cpu_addr);

                        break;
                    case 0x10: /* lda, load word alternate */
                        if(IS_IMM) {
                            goto illegal_insn;
                        }
                        if(!supervisor(dc)) {
                            goto priv_insn;
                        }
                        save_state(dc, cpu_cond);
                        gen_ld_asi(cpu_val, cpu_addr, insn, 4, 0);
                        break;
                    case 0x11: /* lduba, load unsigned byte alternate */
                        if(IS_IMM) {
                            goto illegal_insn;
                        }
                        if(!supervisor(dc)) {
                            goto priv_insn;
                        }
                        save_state(dc, cpu_cond);
                        gen_ld_asi(cpu_val, cpu_addr, insn, 1, 0);
                        break;
                    case 0x12: /* lduha, load unsigned halfword alternate */
                        if(IS_IMM) {
                            goto illegal_insn;
                        }
                        if(!supervisor(dc)) {
                            goto priv_insn;
                        }
                        save_state(dc, cpu_cond);
                        gen_ld_asi(cpu_val, cpu_addr, insn, 2, 0);
                        break;
                    case 0x13: /* ldda, load double word alternate */
                        if(IS_IMM) {
                            goto illegal_insn;
                        }
                        if(!supervisor(dc)) {
                            goto priv_insn;
                        }
                        if(rd & 1) {
                            goto illegal_insn;
                        }
                        save_state(dc, cpu_cond);
                        gen_ldda_asi(cpu_val, cpu_addr, insn, rd);
                        goto skip_move;
                    case 0x19: /* ldsba, load signed byte alternate */
                        if(IS_IMM) {
                            goto illegal_insn;
                        }
                        if(!supervisor(dc)) {
                            goto priv_insn;
                        }
                        save_state(dc, cpu_cond);
                        gen_ld_asi(cpu_val, cpu_addr, insn, 1, 1);
                        break;
                    case 0x1a: /* ldsha, load signed halfword alternate */
                        if(IS_IMM) {
                            goto illegal_insn;
                        }
                        if(!supervisor(dc)) {
                            goto priv_insn;
                        }
                        save_state(dc, cpu_cond);
                        gen_ld_asi(cpu_val, cpu_addr, insn, 2, 1);
                        break;
                    case 0x1d: /* ldstuba -- XXX: should be atomically */
                        if(IS_IMM) {
                            goto illegal_insn;
                        }
                        if(!supervisor(dc)) {
                            goto priv_insn;
                        }
                        save_state(dc, cpu_cond);
                        /* Generate an ldstub if ASI 1 */
                        if(GET_FIELD(insn, 19, 26) == 1) {
                            gen_helper_ldstub(cpu_val, cpu_addr);
                        } else {
                            gen_ldstub_asi(cpu_val, cpu_addr, insn);
                        }
                        break;
                    case 0x1f: /* swapa, swap reg with alt. memory. Also
                                  atomically */
                        CHECK_IU_FEATURE(dc, SWAP);
                        if(IS_IMM) {
                            goto illegal_insn;
                        }
                        if(!supervisor(dc)) {
                            goto priv_insn;
                        }
                        save_state(dc, cpu_cond);
                        gen_movl_reg_TN(rd, cpu_val);

                        /* Generate an swapa if ASI 1 */
                        if(GET_FIELD(insn, 19, 26) == 1) {
                            gen_helper_swap(cpu_val, cpu_val, cpu_addr);
                        } else {
                            gen_swap_asi(cpu_val, cpu_addr, insn);
                        }
                        break;

                    case 0x30: /* ldc */
                    case 0x31: /* ldcsr */
                    case 0x33: /* lddc */
                        goto ncp_insn;
                    default:
                        goto illegal_insn;
                }
                gen_movl_TN_reg(rd, cpu_val);
            skip_move:;
            } else if(xop >= 0x20 && xop < 0x24) {
                if(gen_trap_ifnofpu(dc, cpu_cond)) {
                    goto jmp_insn;
                }
                save_state(dc, cpu_cond);
                switch(xop) {
                    case 0x20: /* ldf, load fpreg */
                        tcg_gen_qemu_ld32u(cpu_tmp0, cpu_addr, dc->base.mem_idx);
                        tcg_gen_trunc_tl_i32(cpu_fpr[rd], cpu_tmp0);
                        break;
                    case 0x21: /* ldfsr */
                    {
                        tcg_gen_qemu_ld32u(cpu_tmp32, cpu_addr, dc->base.mem_idx);
                        gen_helper_ldfsr(cpu_tmp32);
                    } break;
                    case 0x22: /* ldqf, load quad fpreg */
                    {
                        TCGv_i32 r_const;

                        CHECK_FPU_FEATURE(dc, FLOAT128);
                        r_const = tcg_const_i32(dc->base.mem_idx);
                        gen_helper_ldqf(cpu_addr, r_const);
                        tcg_temp_free_i32(r_const);
                        gen_op_store_QT0_fpr(QFPREG(rd));
                    } break;
                    case 0x23: /* lddf, load double fpreg */
                    {
                        TCGv_i32 r_const;

                        r_const = tcg_const_i32(dc->base.mem_idx);
                        gen_helper_lddf(cpu_addr, r_const);
                        tcg_temp_free_i32(r_const);
                        gen_op_store_DT0_fpr(DFPREG(rd));
                    } break;
                    default:
                        goto illegal_insn;
                }
            } else if(xop < 8 || (xop >= 0x14 && xop < 0x18) || xop == 0xe || xop == 0x1e) {
                gen_movl_reg_TN(rd, cpu_val);
                switch(xop) {
                    case 0x4: /* st, store word */
                        save_state(dc, cpu_cond);
                        tcg_gen_qemu_st32(cpu_val, cpu_addr, dc->base.mem_idx);
                        break;
                    case 0x5: /* stb, store byte */
                        save_state(dc, cpu_cond);
                        tcg_gen_qemu_st8(cpu_val, cpu_addr, dc->base.mem_idx);
                        break;
                    case 0x6: /* sth, store halfword */
                        save_state(dc, cpu_cond);
                        tcg_gen_qemu_st16(cpu_val, cpu_addr, dc->base.mem_idx);
                        break;
                    case 0x7: /* std, store double word */
                        if(rd & 1) {
                            goto illegal_insn;
                        } else {
                            TCGv_i32 r_const;

                            save_state(dc, cpu_cond);
                            r_const = tcg_const_i32(7);
                            gen_helper_check_align(cpu_addr, r_const);  //  XXX remove
                            tcg_temp_free_i32(r_const);
                            gen_movl_reg_TN(rd + 1, cpu_tmp0);
                            tcg_gen_concat_tl_i64(cpu_tmp64, cpu_tmp0, cpu_val);
                            tcg_gen_qemu_st64(cpu_tmp64, cpu_addr, dc->base.mem_idx);
                        }
                        break;
                    case 0x14: /* sta, store word alternate */
                        if(IS_IMM) {
                            goto illegal_insn;
                        }
                        if(!supervisor(dc)) {
                            goto priv_insn;
                        }
                        save_state(dc, cpu_cond);
                        gen_st_asi(cpu_val, cpu_addr, insn, 4);
                        dc->npc = DYNAMIC_PC;
                        break;
                    case 0x15: /* stba, store byte alternate */
                        if(IS_IMM) {
                            goto illegal_insn;
                        }
                        if(!supervisor(dc)) {
                            goto priv_insn;
                        }
                        save_state(dc, cpu_cond);
                        gen_st_asi(cpu_val, cpu_addr, insn, 1);
                        dc->npc = DYNAMIC_PC;
                        break;
                    case 0x16: /* stha, store halfword alternate */
                        if(IS_IMM) {
                            goto illegal_insn;
                        }
                        if(!supervisor(dc)) {
                            goto priv_insn;
                        }
                        save_state(dc, cpu_cond);
                        gen_st_asi(cpu_val, cpu_addr, insn, 2);
                        dc->npc = DYNAMIC_PC;
                        break;
                    case 0x17: /* stda, store double word alternate */
                        if(IS_IMM) {
                            goto illegal_insn;
                        }
                        if(!supervisor(dc)) {
                            goto priv_insn;
                        }
                        if(rd & 1) {
                            goto illegal_insn;
                        } else {
                            save_state(dc, cpu_cond);
                            gen_stda_asi(cpu_val, cpu_addr, insn, rd);
                        }
                        break;
                    default:
                        goto illegal_insn;
                }
            } else if(xop > 0x23 && xop < 0x28) {
                if(gen_trap_ifnofpu(dc, cpu_cond)) {
                    goto jmp_insn;
                }
                save_state(dc, cpu_cond);
                switch(xop) {
                    case 0x24: /* stf, store fpreg */
                        tcg_gen_ext_i32_tl(cpu_tmp0, cpu_fpr[rd]);
                        tcg_gen_qemu_st32(cpu_tmp0, cpu_addr, dc->base.mem_idx);
                        break;
                    case 0x25: /* stfsr */
                        tcg_gen_ld_i32(cpu_tmp32, cpu_env, offsetof(CPUState, fsr));
                        tcg_gen_qemu_st32(cpu_tmp32, cpu_addr, dc->base.mem_idx);
                        break;
                    case 0x26:
                        /* stdfq, store floating point queue */
                        if(!supervisor(dc)) {
                            goto priv_insn;
                        }
                        if(gen_trap_ifnofpu(dc, cpu_cond)) {
                            goto jmp_insn;
                        }
                        goto nfq_insn;
                    case 0x27: /* stdf, store double fpreg */
                    {
                        TCGv_i32 r_const;

                        gen_op_load_fpr_DT0(DFPREG(rd));
                        r_const = tcg_const_i32(dc->base.mem_idx);
                        gen_helper_stdf(cpu_addr, r_const);
                        tcg_temp_free_i32(r_const);
                    } break;
                    default:
                        goto illegal_insn;
                }
            } else if(xop > 0x33 && xop < 0x3f) {
                save_state(dc, cpu_cond);
                switch(xop) {
                    case 0x34: /* stc */
                    case 0x35: /* stcsr */
                    case 0x36: /* stdcq */
                    case 0x37: /* stdc */
                        goto ncp_insn;
                    case 0x3c: /* LEON3 casa */
                        CHECK_IU_FEATURE(dc, CASA);

                        /* CASA with I=1 is supported by the ISA but not implemented yet due to
                           missing WRASI, RDASI support. */
                        if(IS_IMM) {
                            goto illegal_insn;
                        }

                        gen_movl_reg_TN(rd, cpu_val);
                        gen_cas_asi(cpu_tmp0, cpu_src1, cpu_src2, cpu_val, insn);
                        gen_movl_TN_reg(rd, cpu_tmp0);
                        break;
                    default:
                        goto illegal_insn;
                }
            } else {
                goto illegal_insn;
            }
        } break;
    }
    /* default case for non jump instructions */
    if(dc->npc == DYNAMIC_PC) {
        dc->base.pc = DYNAMIC_PC;
        gen_op_next_insn();
    } else if(dc->npc == JUMP_PC) {
        /* we can do a static jump */
        gen_branch2(dc, dc->jump_pc[0], dc->jump_pc[1], cpu_cond);
        dc->base.is_jmp = DISAS_JUMP;
    } else {
        dc->base.pc = dc->npc;
        dc->npc = dc->npc + 4;
    }
jmp_insn:
    goto egress;
illegal_insn: {
    TCGv_i32 r_const;

    save_state(dc, cpu_cond);
    r_const = tcg_const_i32(TT_ILL_INSN);
    gen_helper_raise_exception(r_const);
    tcg_temp_free_i32(r_const);
    dc->base.is_jmp = DISAS_JUMP;
}
    goto egress;
unimp_flush: {
    TCGv_i32 r_const;

    save_state(dc, cpu_cond);
    r_const = tcg_const_i32(TT_UNIMP_FLUSH);
    gen_helper_raise_exception(r_const);
    tcg_temp_free_i32(r_const);
    dc->base.is_jmp = DISAS_JUMP;
}
    goto egress;
priv_insn: {
    TCGv_i32 r_const;

    save_state(dc, cpu_cond);
    r_const = tcg_const_i32(TT_PRIV_INSN);
    gen_helper_raise_exception(r_const);
    tcg_temp_free_i32(r_const);
    dc->base.is_jmp = DISAS_JUMP;
}
    goto egress;
nfpu_insn:
    save_state(dc, cpu_cond);
    gen_op_fpexception_im(FSR_FTT_UNIMPFPOP);
    dc->base.is_jmp = DISAS_JUMP;
    goto egress;
nfq_insn:
    save_state(dc, cpu_cond);
    gen_op_fpexception_im(FSR_FTT_SEQ_ERROR);
    dc->base.is_jmp = DISAS_JUMP;
    goto egress;
ncp_insn: {
    TCGv r_const;

    save_state(dc, cpu_cond);
    r_const = tcg_const_i32(TT_NCP_INSN);
    gen_helper_raise_exception(r_const);
    tcg_temp_free(r_const);
    dc->base.is_jmp = DISAS_JUMP;
}
    goto egress;
egress:
    tcg_temp_free(cpu_tmp1);
    tcg_temp_free(cpu_tmp2);
    return 4;
}

void setup_disas_context(DisasContextBase *base, CPUState *env)
{
    DisasContext *dc = (DisasContext *)base;
    dc->npc = (target_ulong)dc->base.tb->cs_base;
    dc->cc_op = CC_OP_DYNAMIC;
    dc->base.mem_idx = cpu_mmu_index(env);
    dc->def = env->def;
    dc->fpu_enabled = tb_fpu_enabled(dc->base.tb->flags);
    dc->address_mask_32bit = tb_am_enabled(dc->base.tb->flags);

    cpu_tmp0 = tcg_temp_local_new();
    cpu_tmp32 = tcg_temp_local_new_i32();
    cpu_tmp64 = tcg_temp_local_new_i64();

    cpu_dst = tcg_temp_local_new();

    //  loads and stores
    cpu_val = tcg_temp_local_new();
    cpu_addr = tcg_temp_local_new();
}

int gen_breakpoint(DisasContextBase *base, CPUBreakpoint *bp)
{
    DisasContext *dc = (DisasContext *)base;
    if(dc->base.pc != dc->base.tb->pc) {
        save_state(dc, cpu_cond);
    }
    gen_helper_debug();
    gen_exit_tb_no_chaining(dc->base.tb);
    dc->base.is_jmp = DISAS_JUMP;
    return 1;
}

int gen_intermediate_code(CPUState *env, DisasContextBase *base)
{
    DisasContext *dc = (DisasContext *)base;

    if(dc->npc & JUMP_PC) {
        /* Since jump_pc[1] is always npc + 4, we can infer after incrementing
         * that jump_pc[1] == pc + 4.  Because of that, we can encode the branch
         * destination into a single word, and store that in npc. */
        assert(dc->jump_pc[1] == base->pc + 4);
        tcg_gen_insn_start(base->pc, dc->jump_pc[0] | JUMP_PC);
    } else {
        tcg_gen_insn_start(base->pc, dc->npc);
    }

    base->tb->size += disas_insn(env, (DisasContext *)base);

    /* if the next PC is different, we abort now */
    if((base->pc - base->tb->pc) != base->tb->size) {
        return 0;
    }
    /* if we reach a page boundary, we stop generation so that the
       PC of a TT_TFAULT exception is always in the right page */
    if((base->pc & (TARGET_PAGE_SIZE - 1)) == 0) {
        return 0;
    }
    return 1;
}

uint32_t gen_intermediate_code_epilogue(CPUState *env, DisasContextBase *base)
{
    DisasContext *dc = (DisasContext *)base;
    tcg_temp_free(cpu_addr);
    tcg_temp_free(cpu_val);
    tcg_temp_free(cpu_dst);
    tcg_temp_free_i64(cpu_tmp64);
    tcg_temp_free_i32(cpu_tmp32);
    tcg_temp_free(cpu_tmp0);
    if(!dc->base.is_jmp) {
        if(dc->base.pc != DYNAMIC_PC && (dc->npc != DYNAMIC_PC && dc->npc != JUMP_PC)) {
            /* static PC and NPC: we can use direct chaining */
            gen_goto_tb(dc, 0, dc->base.pc, dc->npc);
        } else {
            if(dc->base.pc != DYNAMIC_PC) {
                tcg_gen_movi_tl(cpu_pc, dc->base.pc);
            }
            save_npc(dc, cpu_cond);
            gen_exit_tb_no_chaining(dc->base.tb);
        }
    }
    return 0;
}

void restore_state_to_opc(CPUState *env, TranslationBlock *tb, target_ulong *data)
{
    target_ulong pc = data[0];
    target_ulong npc = data[1];

    env->pc = pc;
    if(npc == DYNAMIC_PC) {
        /* dynamic NPC: already stored */
    } else if(npc & JUMP_PC) {
        /* jump PC: use 'cond' and the jump targets of the translation
         * see the comment in gen_intermediate_code for details */
        if(env->cond) {
            env->npc = npc & ~3;
        } else {
            env->npc = pc + 4;
        }
    } else {
        env->npc = npc;
    }

    /* flush pending conditional evaluations before exposing cpu state */
    if(CC_OP != CC_OP_FLAGS) {
        helper_compute_psr();
    }
}

int process_interrupt(int interrupt_request, CPUState *env)
{
    if(tlib_is_in_debug_mode()) {
        return 0;
    }

    if(interrupt_request & CPU_INTERRUPT_HARD) {
        if(cpu_interrupts_enabled(env)) {
            env->interrupt_index = tlib_find_best_interrupt();
            if(env->interrupt_index > 0) {
                int pil = env->interrupt_index & 0xf;
                int type = env->interrupt_index & 0xf0;

                if(((type == TT_EXTINT) && cpu_pil_allowed(env, pil)) || type != TT_EXTINT) {
                    env->exception_index = env->interrupt_index;
                    do_interrupt(env);
                    return 1;
                }
            }
        }
    } else if((interrupt_request & CPU_INTERRUPT_RESET)) {
        cpu_reset(env);
    } else if((interrupt_request & CPU_INTERRUPT_RUN)) {
        /* SMP systems only, start after reset */
        cpu_reset(env);
    }
    return 0;
}

//  TODO: These empty implementations are required due to problems with weak attribute.
//  Remove this after #7035.
void cpu_exec_epilogue(CPUState *env) { }

void cpu_exec_prologue(CPUState *env) { }
