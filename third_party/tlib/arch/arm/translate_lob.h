/*
 *  ARM translation for Low Overhead Branch (LOB) extension
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

#pragma once

#include "cpu_registers.h"

#define LOB_COUNTER_REG LR_32

#define LOB_INSN_WLS_MASK 0xFF80F001
#define LOB_INSN_WLS      0xF000C001

#define LOB_INSN_DLS_MASK 0xFF80FFFF
#define LOB_INSN_DLS      0xF000E001

#define LOB_INSN_LE_MASK 0xFFCFF001
#define LOB_INSN_LE      0xF00FC001

#define LOB_INSN_LCTP_MASK 0xFFFFFFFF
#define LOB_INSN_LCTP      0xF00FE001

#define LOB_RN_IDX 16
#define LOB_RN_LEN 4

#define LOB_IMML_IDX 11
#define LOB_IMML_LEN 1

#define LOB_IMMH_IDX 1
#define LOB_IMMH_LEN 10

#define LOB_LE_INF_IDX 21
#define LOB_LE_INF_LEN 1

#define LOB_SIZE_IDX 20
#define LOB_SIZE_LEN 2

#define LOB_LS_TP_MASK 0x00400000
#define LOB_LS_TP      0x00000000

#define LOB_LE_TP_MASK 0x00100000
#define LOB_LE_TP      0x00100000

static inline bool is_insn_wls(uint32_t insn);
static inline bool is_insn_dls(uint32_t insn);
static inline bool is_insn_le(uint32_t insn);

static inline int trans_wls(DisasContext *s, uint32_t insn);
static inline int trans_dls(DisasContext *s, uint32_t insn);
static inline int trans_le(DisasContext *s, uint32_t insn);

static inline void lob_gen_wls_ir(DisasContext *s, uint32_t rn, uint32_t imm, uint32_t size);
static inline void lob_gen_dls_ir(DisasContext *s, uint32_t rn, uint32_t size);
static inline void lob_gen_le_ir(DisasContext *s, bool is_inf_loop, uint32_t imm, bool is_tp);

static inline bool lob_is_ls_constraint_unpredictable(DisasContext *s, bool is_tp, uint32_t rn);
static inline bool lob_is_le_constraint_unpredictable(DisasContext *s);
static inline bool lob_is_ls_tp(uint32_t insn);
static inline bool lob_is_le_tp(uint32_t insn);

static inline uint32_t lob_extract_rn(uint32_t insn);
static inline uint32_t lob_extract_imm(uint32_t insn);
static inline uint32_t lob_extract_le_inf(uint32_t insn);

static inline uint32_t lob_extract_size(uint32_t insn);

/* Check if given instruction is WLS(TP) */
static inline bool is_insn_wls(uint32_t insn)
{
    bool is_tp = lob_is_ls_tp(insn);
    uint32_t rn = lob_extract_rn(insn);
    /* WLSTP with rn == 15 is a related encoding (LE)*/
    return !(is_tp && rn == 15) && (insn & LOB_INSN_WLS_MASK) == LOB_INSN_WLS;
}

/* Check if given instruction is DLS(TP) */
static inline bool is_insn_dls(uint32_t insn)
{
    bool is_tp = lob_is_ls_tp(insn);
    uint32_t rn = lob_extract_rn(insn);
    /* DLSTP with rn == 15 is a related encoding (LCTP)*/
    return !(is_tp && rn == 15) && (insn & LOB_INSN_DLS_MASK) == LOB_INSN_DLS;
}

/* Check if given instruction is LE(TP) */
static inline bool is_insn_le(uint32_t insn)
{
    return (insn & LOB_INSN_LE_MASK) == LOB_INSN_LE;
}

/* Check if given instruction is LCTP */
static inline bool is_insn_lctp(uint32_t insn)
{
    return (insn & LOB_INSN_LCTP_MASK) == LOB_INSN_LCTP;
}

/* Check if given instruction is BF, BFX, BFL, BFLX or BFCSEL */
static inline bool is_insn_bf_bfx_bfl_bflx_bfcsel(uint32_t insn)
{
    bool bfl = (insn & 0xF800F001) == 0xF000C001;
    bool bfcsel = (insn & 0xF840F001) == 0xF000E001;
    bool bf = (insn & 0xF860F001) == 0xF040E001;
    bool bfx = (insn & 0xF870FFFF) == 0xF060E001;
    bool bflx = (insn & 0xF870FFFF) == 0xF070E001;

    uint32_t boff = extract32(insn, 23, 4);

    return (boff != 0) && (bfl || bfcsel || bf || bfx || bflx);
}

/* Translate WLS(TP) instruction */
static inline int trans_wls(DisasContext *s, uint32_t insn)
{
    bool is_tp = lob_is_ls_tp(insn);
    uint32_t rn = lob_extract_rn(insn);
    uint32_t imm = lob_extract_imm(insn);
    uint32_t size = LTPSIZE_PREDICATION_DISABLED;

    if(lob_is_ls_constraint_unpredictable(s, is_tp, rn)) {
        return TRANS_STATUS_ILLEGAL_INSN;
    }

    if(is_tp) {
        if(!arm_feature(env, ARM_FEATURE_MVE)) {
            return TRANS_STATUS_ILLEGAL_INSN;
        }

        if(!s->vfp_enabled) { /* WLSTP instruction requires access to FP/MVE coprocessor */
            gen_exception_insn(s, 4, EXCP_NOCP);
            return TRANS_STATUS_SUCCESS;
        }
        size = lob_extract_size(insn);
        /* As opposed to DLSTP instruction we can't do the lazy state preservation here as we skip it in case we immediately exit
         * the loop */
    }

    lob_gen_wls_ir(s, rn, imm, size);

    return TRANS_STATUS_SUCCESS;
}

/* Translate DLS(TP) instruction */
static inline int trans_dls(DisasContext *s, uint32_t insn)
{
    bool is_tp = lob_is_ls_tp(insn);
    uint32_t rn = lob_extract_rn(insn);
    uint32_t size = LTPSIZE_PREDICATION_DISABLED;

    if(lob_is_ls_constraint_unpredictable(s, is_tp, rn)) {
        return TRANS_STATUS_ILLEGAL_INSN;
    }

    if(is_tp) {
        if(!arm_feature(env, ARM_FEATURE_MVE)) {
            return TRANS_STATUS_ILLEGAL_INSN;
        }
        if(!s->vfp_enabled) { /* DLSTP instruction requires access to FP/MVE coprocessor */
            gen_exception_insn(s, 4, EXCP_NOCP);
            return TRANS_STATUS_SUCCESS;
        }

        size = lob_extract_size(insn);
        gen_helper_fp_lsp(cpu_env);
    }

    lob_gen_dls_ir(s, rn, size);

    return TRANS_STATUS_SUCCESS;
}

/* Translate LE(TP) instruction */
static inline int trans_le(DisasContext *s, uint32_t insn)
{
    bool is_tp = lob_is_le_tp(insn);
    bool is_inf_loop = lob_extract_le_inf(insn); /* Should always be 0 for LETP */
    uint32_t imm = lob_extract_imm(insn);

    if(lob_is_le_constraint_unpredictable(s)) {
        return TRANS_STATUS_ILLEGAL_INSN;
    }

    if(is_tp) {
        if(!arm_feature(env, ARM_FEATURE_MVE)) {
            return TRANS_STATUS_ILLEGAL_INSN;
        }

        if(!s->vfp_enabled) { /* LETP instruction requires access to FP/MVE coprocessor */
            s->eci_handled = true;
            gen_exception_insn(s, 4, EXCP_NOCP);
            return TRANS_STATUS_SUCCESS;
        }

        gen_helper_fp_lsp(cpu_env);
    }

    s->eci_handled = true;

    /*
     * With MVE, LTPSIZE might not be 4, and we must emit an INVSTATE
     * UsageFault exception for the LE insn in that case. Note that we
     * are not directly checking FPSCR.LTPSIZE but instead check the
     * pseudocode LTPSIZE() function, which returns 4 if the FPU is
     * not currently active (ie ActiveFPState() returns false).
     *
     * Usually we don't need to care about this distinction between
     * LTPSIZE and FPSCR.LTPSIZE, because the code in helper_fp_lsp()
     * will clear the conditions that make the FPU not active.
     * But LE is an unusual case of a non-FP insn that looks at LTPSIZE.
     */

    if(!is_tp && arm_feature(env, ARM_FEATURE_MVE) && s->vfp_enabled) {
        /* Need to do a runtime check for LTPSIZE != 4 */
        TCGv_i32 ltpsize = load_cpu_field(v7m.ltpsize);
        TCGv_i32 fpu_context_active = tcg_temp_local_new_i32();

        int not_failed_label = gen_new_label();
        /* ltpsize == 4, everything is alright */
        tcg_gen_brcondi_i32(TCG_COND_EQ, ltpsize, 4, not_failed_label);

        /*
         * In case that ltpsize != 4 we should check if fpu context is active.
         * If it is not we ignore the ltpsize field and assume it is 4 so exception is not raised.
         * We decide to first check if ltpsize is 4 to not have to call the helper on every proper execution of LE.
         */
        gen_helper_is_fpu_context_active(fpu_context_active, cpu_env);
        tcg_gen_brcondi_i32(TCG_COND_EQ, fpu_context_active, 0, not_failed_label);

        gen_exception_insn(s, 4, EXCP_INVSTATE);

        gen_set_label(not_failed_label);
        tcg_temp_free_i32(ltpsize);
        tcg_temp_free_i32(fpu_context_active);
    }

    lob_gen_le_ir(s, is_inf_loop, imm, is_tp);

    return TRANS_STATUS_SUCCESS;
}

static int trans_lctp(DisasContext *s)
{
    /*
     * M-profile Loop Clear with Tail Predication. Since our implementation
     * doesn't cache branch information, all we need to do is reset
     * FPSCR.LTPSIZE to 4.
     */
    gen_helper_fp_lsp(cpu_env);
    store_cpu_field(tcg_const_i32(LTPSIZE_PREDICATION_DISABLED), v7m.ltpsize);
    return TRANS_STATUS_SUCCESS;
}

/* Generate intermediate representation for WLS instruction */
static inline void lob_gen_wls_ir(DisasContext *s, uint32_t rn, uint32_t imm, uint32_t size)
{
    //  If counter == 0 skip loop
    int next_insn_label = gen_new_label();
    tcg_gen_brcondi_i32(TCG_COND_EQ, cpu_R[rn], 0, next_insn_label);
    //  Setup loop counter
    TCGv_i32 loop_counter = load_reg(s, rn);
    store_reg(s, LOB_COUNTER_REG, loop_counter);

    if(size != LTPSIZE_PREDICATION_DISABLED) {
        gen_helper_fp_lsp(cpu_env);
        store_cpu_field(tcg_const_i32(size), v7m.ltpsize);
    }

    //  Jump to loop content (next TB)
    gen_goto_tb(s, 1, s->base.pc);
    //  Loop ending instructions
    gen_set_label(next_insn_label);
    gen_jmp(s, s->base.pc + imm, STACK_FRAME_NO_CHANGE);
}

/* Generate intermediate representation for DLS instruction */
static inline void lob_gen_dls_ir(DisasContext *s, uint32_t rn, uint32_t size)
{
    //  Setup loop counter
    TCGv_i32 loop_counter = load_reg(s, rn);
    store_reg(s, LOB_COUNTER_REG, loop_counter);
    if(size != LTPSIZE_PREDICATION_DISABLED) {
        store_cpu_field(tcg_const_i32(size), v7m.ltpsize);
    }
}

/* Generate intermediate representation for LE instruction */
static inline void lob_gen_le_ir(DisasContext *s, bool is_inf_loop, uint32_t imm, bool is_tp)
{
    uint32_t loop_start_addr = s->base.pc - imm;
    if(is_inf_loop) {
        //  Jump back to the loop start
        gen_jmp(s, loop_start_addr, STACK_FRAME_NO_CHANGE);
    } else {
        int loop_end_label = gen_new_label();
        TCGv_i32 loop_counter_reg = cpu_R[LOB_COUNTER_REG];

        if(!is_tp) {
            //  Jump outside loop if counter == 0
            tcg_gen_brcondi_i32(TCG_COND_LEU, loop_counter_reg, 1, loop_end_label);
            //  Decrement loop counter
            tcg_gen_addi_i32(loop_counter_reg, loop_counter_reg, -1);
        } else {
            TCGv_i32 ltpsize = load_cpu_field(v7m.ltpsize);

            //  Decrement loop counter by 1 << (4 - LTPSIZE)
            TCGv_i32 decr = tcg_temp_local_new_i32();
            tcg_gen_movi_i32(decr, 4);
            tcg_gen_sub_i32(decr, decr, ltpsize);

            TCGv_i32 one = tcg_const_i32(1);
            tcg_gen_shl_i32(decr, one, decr);
            tcg_temp_free_i32(one);

            tcg_temp_free_i32(ltpsize);

            tcg_gen_brcond_i32(TCG_COND_LEU, cpu_R[LOB_COUNTER_REG], decr, loop_end_label);

            tcg_gen_sub_i32(cpu_R[LOB_COUNTER_REG], cpu_R[LOB_COUNTER_REG], decr);

            tcg_temp_free_i32(decr);
        }
        //  Jump back to the loop start
        gen_jmp(s, loop_start_addr, STACK_FRAME_NO_CHANGE);
        //  Loop ending instructions
        gen_set_label(loop_end_label);

        if(is_tp) {
            //  Exits from tail-pred loops must reset LTPSIZE to default
            store_cpu_field(tcg_const_i32(LTPSIZE_PREDICATION_DISABLED), v7m.ltpsize);
        }
        gen_goto_tb(s, 1, s->base.pc);
    }
}

/* Check if loop start instruction is CONSTRAINED_UNPREDICTABLE */
static inline bool lob_is_ls_constraint_unpredictable(DisasContext *s, bool is_tp, uint32_t rn)
{
    return (rn == 13) || (!is_tp && rn == 15) || (s->condexec_mask);
}

/* Check if loop end instruction is CONSTRAINED_UNPREDICTABLE */
static inline bool lob_is_le_constraint_unpredictable(DisasContext *s)
{
    return s->condexec_mask;
}

/* Check if loop start instruction is tail prediction variant */
static inline bool lob_is_ls_tp(uint32_t insn)
{
    return (insn & LOB_LS_TP_MASK) == LOB_LS_TP;
}

/* Check if loop end instruction is tail prediction variant */
static inline bool lob_is_le_tp(uint32_t insn)
{
    return (insn & LOB_LE_TP_MASK) == LOB_LE_TP;
}

/* Extract rn from LOB instruction */
static inline uint32_t lob_extract_rn(uint32_t insn)
{
    return extract32(insn, LOB_RN_IDX, LOB_RN_LEN);
}

/* Extract imm from LOB instruction */
static inline uint32_t lob_extract_imm(uint32_t insn)
{
    uint32_t imml = extract32(insn, LOB_IMML_IDX, LOB_IMML_LEN);
    uint32_t immh = extract32(insn, LOB_IMMH_IDX, LOB_IMMH_LEN);
    return (immh << 2) + (imml << 1);
}

/* Extract infinite loop bit from loop end instruction */
static inline uint32_t lob_extract_le_inf(uint32_t insn)
{
    return extract32(insn, LOB_LE_INF_IDX, LOB_LE_INF_LEN);
}

static inline uint32_t lob_extract_size(uint32_t insn)
{
    return extract32(insn, LOB_SIZE_IDX, LOB_SIZE_LEN);
}
