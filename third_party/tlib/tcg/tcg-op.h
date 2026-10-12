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
#pragma once

#include <inttypes.h>
#ifndef _WIN32  //  This header is not available on MinGW
#include <execinfo.h>
#endif
#include <stdio.h>
#include <stdlib.h>

#include "tcg-memop.h"
#include "tcg-mo.h"
#include "tcg.h"
#include "additional.h"
#include "../include/hash-table-store-test.h"

typedef struct CPUState CPUState;
extern CPUState *cpu;

#if TARGET_LONG_BITS == 64
#define tcg_gen_movi_tl      tcg_gen_movi_i64
#define tcg_gen_mov_tl       tcg_gen_mov_i64
#define tcg_gen_ld8u_tl      tcg_gen_ld8u_i64
#define tcg_gen_ld8s_tl      tcg_gen_ld8s_i64
#define tcg_gen_ld16u_tl     tcg_gen_ld16u_i64
#define tcg_gen_ld16s_tl     tcg_gen_ld16s_i64
#define tcg_gen_ld32u_tl     tcg_gen_ld32u_i64
#define tcg_gen_ld32s_tl     tcg_gen_ld32s_i64
#define tcg_gen_ld_tl        tcg_gen_ld_i64
#define tcg_gen_st8_tl       tcg_gen_st8_i64
#define tcg_gen_st16_tl      tcg_gen_st16_i64
#define tcg_gen_st32_tl      tcg_gen_st32_i64
#define tcg_gen_st_tl        tcg_gen_st_i64
#define tcg_gen_add_tl       tcg_gen_add_i64
#define tcg_gen_addi_tl      tcg_gen_addi_i64
#define tcg_gen_sub_tl       tcg_gen_sub_i64
#define tcg_gen_neg_tl       tcg_gen_neg_i64
#define tcg_gen_subfi_tl     tcg_gen_subfi_i64
#define tcg_gen_subi_tl      tcg_gen_subi_i64
#define tcg_gen_and_tl       tcg_gen_and_i64
#define tcg_gen_andi_tl      tcg_gen_andi_i64
#define tcg_gen_or_tl        tcg_gen_or_i64
#define tcg_gen_ori_tl       tcg_gen_ori_i64
#define tcg_gen_xor_tl       tcg_gen_xor_i64
#define tcg_gen_xori_tl      tcg_gen_xori_i64
#define tcg_gen_not_tl       tcg_gen_not_i64
#define tcg_gen_shl_tl       tcg_gen_shl_i64
#define tcg_gen_shli_tl      tcg_gen_shli_i64
#define tcg_gen_shr_tl       tcg_gen_shr_i64
#define tcg_gen_shri_tl      tcg_gen_shri_i64
#define tcg_gen_sar_tl       tcg_gen_sar_i64
#define tcg_gen_sari_tl      tcg_gen_sari_i64
#define tcg_gen_brcond_tl    tcg_gen_brcond_i64
#define tcg_gen_brcondi_tl   tcg_gen_brcondi_i64
#define tcg_gen_setcond_tl   tcg_gen_setcond_i64
#define tcg_gen_setcondi_tl  tcg_gen_setcondi_i64
#define tcg_gen_mul_tl       tcg_gen_mul_i64
#define tcg_gen_muli_tl      tcg_gen_muli_i64
#define tcg_gen_div_tl       tcg_gen_div_i64
#define tcg_gen_rem_tl       tcg_gen_rem_i64
#define tcg_gen_divu_tl      tcg_gen_divu_i64
#define tcg_gen_remu_tl      tcg_gen_remu_i64
#define tcg_gen_discard_tl   tcg_gen_discard_i64
#define tcg_gen_trunc_tl_i32 tcg_gen_trunc_i64_i32
#define tcg_gen_trunc_i64_tl tcg_gen_mov_i64
#define tcg_gen_extu_i32_tl  tcg_gen_extu_i32_i64
#define tcg_gen_ext_i32_tl   tcg_gen_ext_i32_i64
#define tcg_gen_extu_tl_i64  tcg_gen_mov_i64
#define tcg_gen_ext_tl_i64   tcg_gen_mov_i64
#define tcg_gen_ext8u_tl     tcg_gen_ext8u_i64
#define tcg_gen_ext8s_tl     tcg_gen_ext8s_i64
#define tcg_gen_ext16u_tl    tcg_gen_ext16u_i64
#define tcg_gen_ext16s_tl    tcg_gen_ext16s_i64
#define tcg_gen_ext32u_tl    tcg_gen_ext32u_i64
#define tcg_gen_ext32s_tl    tcg_gen_ext32s_i64
//  Now these support passing flags so let's adjust calls used in i386 and ppc.
#define tcg_gen_bswap16_tl(ret, arg) tcg_gen_bswap16_i64(ret, arg, 0)
#define tcg_gen_bswap32_tl(ret, arg) tcg_gen_bswap32_i64(ret, arg, 0)
#define tcg_gen_bswap64_tl           tcg_gen_bswap64_i64
#define tcg_gen_concat_tl_i64        tcg_gen_concat32_i64
#define tcg_gen_andc_tl              tcg_gen_andc_i64
#define tcg_gen_eqv_tl               tcg_gen_eqv_i64
#define tcg_gen_nand_tl              tcg_gen_nand_i64
#define tcg_gen_nor_tl               tcg_gen_nor_i64
#define tcg_gen_orc_tl               tcg_gen_orc_i64
#define tcg_gen_rotl_tl              tcg_gen_rotl_i64
#define tcg_gen_rotli_tl             tcg_gen_rotli_i64
#define tcg_gen_rotr_tl              tcg_gen_rotr_i64
#define tcg_gen_rotri_tl             tcg_gen_rotri_i64
#define tcg_gen_deposit_tl           tcg_gen_deposit_i64
#define tcg_const_tl                 tcg_const_i64
#define tcg_const_local_tl           tcg_const_local_i64
#define tcg_gen_mulu2_tl             tcg_gen_mulu2_i64
#define tcg_gen_muls2_tl             tcg_gen_muls2_i64
#define tcg_gen_movcond_tl           tcg_gen_movcond_i64
#define tcg_gen_qemu_ld_tl           tcg_gen_qemu_ld_i64
#define tcg_gen_qemu_st_tl           tcg_gen_qemu_st_i64
#else
#define tcg_gen_movi_tl              tcg_gen_movi_i32
#define tcg_gen_mov_tl               tcg_gen_mov_i32
#define tcg_gen_ld8u_tl              tcg_gen_ld8u_i32
#define tcg_gen_ld8s_tl              tcg_gen_ld8s_i32
#define tcg_gen_ld16u_tl             tcg_gen_ld16u_i32
#define tcg_gen_ld16s_tl             tcg_gen_ld16s_i32
#define tcg_gen_ld32u_tl             tcg_gen_ld_i32
#define tcg_gen_ld32s_tl             tcg_gen_ld_i32
#define tcg_gen_ld_tl                tcg_gen_ld_i32
#define tcg_gen_st8_tl               tcg_gen_st8_i32
#define tcg_gen_st16_tl              tcg_gen_st16_i32
#define tcg_gen_st32_tl              tcg_gen_st_i32
#define tcg_gen_st_tl                tcg_gen_st_i32
#define tcg_gen_add_tl               tcg_gen_add_i32
#define tcg_gen_addi_tl              tcg_gen_addi_i32
#define tcg_gen_sub_tl               tcg_gen_sub_i32
#define tcg_gen_neg_tl               tcg_gen_neg_i32
#define tcg_gen_subfi_tl             tcg_gen_subfi_i32
#define tcg_gen_subi_tl              tcg_gen_subi_i32
#define tcg_gen_and_tl               tcg_gen_and_i32
#define tcg_gen_andi_tl              tcg_gen_andi_i32
#define tcg_gen_or_tl                tcg_gen_or_i32
#define tcg_gen_ori_tl               tcg_gen_ori_i32
#define tcg_gen_xor_tl               tcg_gen_xor_i32
#define tcg_gen_xori_tl              tcg_gen_xori_i32
#define tcg_gen_not_tl               tcg_gen_not_i32
#define tcg_gen_shl_tl               tcg_gen_shl_i32
#define tcg_gen_shli_tl              tcg_gen_shli_i32
#define tcg_gen_shr_tl               tcg_gen_shr_i32
#define tcg_gen_shri_tl              tcg_gen_shri_i32
#define tcg_gen_sar_tl               tcg_gen_sar_i32
#define tcg_gen_sari_tl              tcg_gen_sari_i32
#define tcg_gen_brcond_tl            tcg_gen_brcond_i32
#define tcg_gen_brcondi_tl           tcg_gen_brcondi_i32
#define tcg_gen_setcond_tl           tcg_gen_setcond_i32
#define tcg_gen_setcondi_tl          tcg_gen_setcondi_i32
#define tcg_gen_mul_tl               tcg_gen_mul_i32
#define tcg_gen_muli_tl              tcg_gen_muli_i32
#define tcg_gen_div_tl               tcg_gen_div_i32
#define tcg_gen_rem_tl               tcg_gen_rem_i32
#define tcg_gen_divu_tl              tcg_gen_divu_i32
#define tcg_gen_remu_tl              tcg_gen_remu_i32
#define tcg_gen_discard_tl           tcg_gen_discard_i32
#define tcg_gen_trunc_tl_i32         tcg_gen_mov_i32
#define tcg_gen_trunc_i64_tl         tcg_gen_trunc_i64_i32
#define tcg_gen_extu_i32_tl          tcg_gen_mov_i32
#define tcg_gen_ext_i32_tl           tcg_gen_mov_i32
#define tcg_gen_extu_tl_i64          tcg_gen_extu_i32_i64
#define tcg_gen_ext_tl_i64           tcg_gen_ext_i32_i64
#define tcg_gen_ext8u_tl             tcg_gen_ext8u_i32
#define tcg_gen_ext8s_tl             tcg_gen_ext8s_i32
#define tcg_gen_ext16u_tl            tcg_gen_ext16u_i32
#define tcg_gen_ext16s_tl            tcg_gen_ext16s_i32
#define tcg_gen_ext32u_tl            tcg_gen_mov_i32
#define tcg_gen_ext32s_tl            tcg_gen_mov_i32
//  Now it supports passing flags so let's adjust calls used in i386 and ppc.
#define tcg_gen_bswap16_tl(ret, arg) tcg_gen_bswap16_i32(ret, arg, 0)
#define tcg_gen_bswap32_tl           tcg_gen_bswap32_i32
#define tcg_gen_concat_tl_i64        tcg_gen_concat_i32_i64
#define tcg_gen_andc_tl              tcg_gen_andc_i32
#define tcg_gen_eqv_tl               tcg_gen_eqv_i32
#define tcg_gen_nand_tl              tcg_gen_nand_i32
#define tcg_gen_nor_tl               tcg_gen_nor_i32
#define tcg_gen_orc_tl               tcg_gen_orc_i32
#define tcg_gen_rotl_tl              tcg_gen_rotl_i32
#define tcg_gen_rotli_tl             tcg_gen_rotli_i32
#define tcg_gen_rotr_tl              tcg_gen_rotr_i32
#define tcg_gen_rotri_tl             tcg_gen_rotri_i32
#define tcg_gen_deposit_tl           tcg_gen_deposit_i32
#define tcg_const_tl                 tcg_const_i32
#define tcg_const_local_tl           tcg_const_local_i32
#define tcg_gen_mulu2_tl             tcg_gen_mulu2_i32
#define tcg_gen_muls2_tl             tcg_gen_muls2_i32
#define tcg_gen_movcond_tl           tcg_gen_movcond_i32
#define tcg_gen_qemu_ld_tl           tcg_gen_qemu_ld_i32
#define tcg_gen_qemu_st_tl           tcg_gen_qemu_st_i32
#endif

int gen_new_label(void);

static inline void tcg_request_block_interrupt_check()
{
    tlib_assert(tcg->disas_context);
    tcg->disas_context->generate_block_exit_check = true;
}

static inline TCGOpcodeEntry tcg_create_opcode_entry(const TCGOpcode opcode)
{
    TCGOpcodeEntry entry = { .opcode = opcode };
#if defined(TCG_OPCODE_BACKTRACE) && DEBUG && !defined(_WIN32)
    const uint32_t buffer_entries = backtrace(entry.backtrace.return_addresses, TCG_TRACE_MAX_SIZE);
    entry.backtrace.address_count = buffer_entries;
#endif
    return entry;
}

static inline void tcg_gen_op0(const TCGOpcode opc)
{
    *gen_opc_ptr++ = tcg_create_opcode_entry(opc);
}

static inline void tcg_gen_op1_i32(TCGOpcode opc, TCGv_i32 arg1)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg1);
}

static inline void tcg_gen_op1_i64(TCGOpcode opc, TCGv_i64 arg1)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg1);
}

static inline void tcg_gen_op1i(TCGOpcode opc, TCGArg arg1)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = arg1;
}

static inline void tcg_gen_op2_i32(TCGOpcode opc, TCGv_i32 arg1, TCGv_i32 arg2)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg1);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg2);
}

static inline void tcg_gen_op2_i64(TCGOpcode opc, TCGv_i64 arg1, TCGv_i64 arg2)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg1);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg2);
}

static inline void tcg_gen_op2i_i32(TCGOpcode opc, TCGv_i32 arg1, TCGArg arg2)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg1);
    *gen_opparam_ptr++ = arg2;
}

static inline void tcg_gen_op2i_i64(TCGOpcode opc, TCGv_i64 arg1, TCGArg arg2)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg1);
    *gen_opparam_ptr++ = arg2;
}

static inline void tcg_gen_op2ii(TCGOpcode opc, TCGArg arg1, TCGArg arg2)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = arg1;
    *gen_opparam_ptr++ = arg2;
}

static inline void tcg_gen_op3_i32(TCGOpcode opc, TCGv_i32 arg1, TCGv_i32 arg2, TCGv_i32 arg3)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg1);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg2);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg3);
}

static inline void tcg_gen_op3_i64(TCGOpcode opc, TCGv_i64 arg1, TCGv_i64 arg2, TCGv_i64 arg3)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg1);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg2);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg3);
}

static inline void tcg_gen_op3i_i32(TCGOpcode opc, TCGv_i32 arg1, TCGv_i32 arg2, TCGArg arg3)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg1);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg2);
    *gen_opparam_ptr++ = arg3;
}

static inline void tcg_gen_op3i_i64(TCGOpcode opc, TCGv_i64 arg1, TCGv_i64 arg2, TCGArg arg3)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg1);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg2);
    *gen_opparam_ptr++ = arg3;
}

static inline void tcg_gen_op3iii(TCGOpcode opc, TCGArg arg1, TCGArg arg2, TCGArg arg3)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = arg1;
    *gen_opparam_ptr++ = arg2;
    *gen_opparam_ptr++ = arg3;
}

static inline void tcg_gen_ldst_op_i32(TCGOpcode opc, TCGv_i32 val, TCGv_ptr base, TCGArg offset)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I32(val);
    *gen_opparam_ptr++ = GET_TCGV_PTR(base);
    *gen_opparam_ptr++ = offset;
}

static inline void tcg_gen_ldst_op_i64(TCGOpcode opc, TCGv_i64 val, TCGv_ptr base, TCGArg offset)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I64(val);
    *gen_opparam_ptr++ = GET_TCGV_PTR(base);
    *gen_opparam_ptr++ = offset;
}

static inline void tcg_gen_qemu_ldst_op_i64_i32(TCGOpcode opc, TCGv_i64 val, TCGv_i32 addr, TCGArg mem_index)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I64(val);
    *gen_opparam_ptr++ = GET_TCGV_I32(addr);
    *gen_opparam_ptr++ = mem_index;
}

static inline void tcg_gen_qemu_ldst_op_i64_i64(TCGOpcode opc, TCGv_i64 val, TCGv_i64 addr, TCGArg mem_index)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I64(val);
    *gen_opparam_ptr++ = GET_TCGV_I64(addr);
    *gen_opparam_ptr++ = mem_index;
}

static inline void tcg_gen_op4_i32(TCGOpcode opc, TCGv_i32 arg1, TCGv_i32 arg2, TCGv_i32 arg3, TCGv_i32 arg4)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg1);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg2);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg3);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg4);
}

static inline void tcg_gen_op4_i64(TCGOpcode opc, TCGv_i64 arg1, TCGv_i64 arg2, TCGv_i64 arg3, TCGv_i64 arg4)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg1);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg2);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg3);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg4);
}

static inline void tcg_gen_op4i_i32(TCGOpcode opc, TCGv_i32 arg1, TCGv_i32 arg2, TCGv_i32 arg3, TCGArg arg4)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg1);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg2);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg3);
    *gen_opparam_ptr++ = arg4;
}

static inline void tcg_gen_atomic_intrinsic_op_i32(TCGOpcode opc, TCGv_i32 ret, TCGv_ptr address, TCGv_i32 arg)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I32(ret);
    *gen_opparam_ptr++ = GET_TCGV_PTR(address);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg);
}

static inline void tcg_gen_atomic_intrinsic_op_i64(TCGOpcode opc, TCGv_i64 ret, TCGv_ptr address, TCGv_i64 arg)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I64(ret);
    *gen_opparam_ptr++ = GET_TCGV_PTR(address);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg);
}

static inline void tcg_gen_atomic_intrinsic_op4_i32(TCGOpcode opc, TCGv_i32 ret, TCGv_i32 arg0, TCGv_ptr address, TCGv_i32 arg1)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I32(ret);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg0);
    *gen_opparam_ptr++ = GET_TCGV_PTR(address);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg1);
}

static inline void tcg_gen_atomic_intrinsic_op4_i64(TCGOpcode opc, TCGv_i64 ret, TCGv_i64 arg0, TCGv_ptr address, TCGv_i64 arg1)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I64(ret);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg0);
    *gen_opparam_ptr++ = GET_TCGV_PTR(address);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg1);
}

static inline void tcg_gen_out_i128(TCGv_i128 variable)
{
    *gen_opparam_ptr++ = GET_TCGV_I64(variable.high);
    *gen_opparam_ptr++ = GET_TCGV_I64(variable.low);
}

static inline void tcg_gen_atomic_intrinsic_op4_i128(TCGOpcode opc, TCGv_i128 ret, TCGv_i128 arg0, TCGv_ptr address,
                                                     TCGv_i128 arg1)
{
    tcg_gen_op0(opc);
    tcg_gen_out_i128(ret);
    tcg_gen_out_i128(arg0);
    *gen_opparam_ptr++ = GET_TCGV_PTR(address);
    tcg_gen_out_i128(arg1);
}

static inline void tcg_gen_op4i_i64(TCGOpcode opc, TCGv_i64 arg1, TCGv_i64 arg2, TCGv_i64 arg3, TCGArg arg4)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg1);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg2);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg3);
    *gen_opparam_ptr++ = arg4;
}

static inline void tcg_gen_op4ii_i32(TCGOpcode opc, TCGv_i32 arg1, TCGv_i32 arg2, TCGArg arg3, TCGArg arg4)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg1);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg2);
    *gen_opparam_ptr++ = arg3;
    *gen_opparam_ptr++ = arg4;
}

static inline void tcg_gen_op4ii_i64(TCGOpcode opc, TCGv_i64 arg1, TCGv_i64 arg2, TCGArg arg3, TCGArg arg4)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg1);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg2);
    *gen_opparam_ptr++ = arg3;
    *gen_opparam_ptr++ = arg4;
}

static inline void tcg_gen_op4iiii(TCGOpcode opc, TCGArg arg1, TCGArg arg2, TCGArg arg3, TCGArg arg4)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = arg1;
    *gen_opparam_ptr++ = arg2;
    *gen_opparam_ptr++ = arg3;
    *gen_opparam_ptr++ = arg4;
}

static inline void tcg_gen_op5_i32(TCGOpcode opc, TCGv_i32 arg1, TCGv_i32 arg2, TCGv_i32 arg3, TCGv_i32 arg4, TCGv_i32 arg5)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg1);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg2);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg3);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg4);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg5);
}

static inline void tcg_gen_op5_i64(TCGOpcode opc, TCGv_i64 arg1, TCGv_i64 arg2, TCGv_i64 arg3, TCGv_i64 arg4, TCGv_i64 arg5)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg1);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg2);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg3);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg4);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg5);
}

static inline void tcg_gen_op5i_i32(TCGOpcode opc, TCGv_i32 arg1, TCGv_i32 arg2, TCGv_i32 arg3, TCGv_i32 arg4, TCGArg arg5)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg1);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg2);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg3);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg4);
    *gen_opparam_ptr++ = arg5;
}

static inline void tcg_gen_op5i_i64(TCGOpcode opc, TCGv_i64 arg1, TCGv_i64 arg2, TCGv_i64 arg3, TCGv_i64 arg4, TCGArg arg5)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg1);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg2);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg3);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg4);
    *gen_opparam_ptr++ = arg5;
}

static inline void tcg_gen_op5ii_i32(TCGOpcode opc, TCGv_i32 arg1, TCGv_i32 arg2, TCGv_i32 arg3, TCGArg arg4, TCGArg arg5)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg1);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg2);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg3);
    *gen_opparam_ptr++ = arg4;
    *gen_opparam_ptr++ = arg5;
}

static inline void tcg_gen_op5ii_i64(TCGOpcode opc, TCGv_i64 arg1, TCGv_i64 arg2, TCGv_i64 arg3, TCGArg arg4, TCGArg arg5)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg1);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg2);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg3);
    *gen_opparam_ptr++ = arg4;
    *gen_opparam_ptr++ = arg5;
}

static inline void tcg_gen_op6_i32(TCGOpcode opc, TCGv_i32 arg1, TCGv_i32 arg2, TCGv_i32 arg3, TCGv_i32 arg4, TCGv_i32 arg5,
                                   TCGv_i32 arg6)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg1);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg2);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg3);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg4);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg5);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg6);
}

static inline void tcg_gen_op6_i64(TCGOpcode opc, TCGv_i64 arg1, TCGv_i64 arg2, TCGv_i64 arg3, TCGv_i64 arg4, TCGv_i64 arg5,
                                   TCGv_i64 arg6)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg1);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg2);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg3);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg4);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg5);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg6);
}

static inline void tcg_gen_op6i_i32(TCGOpcode opc, TCGv_i32 arg1, TCGv_i32 arg2, TCGv_i32 arg3, TCGv_i32 arg4, TCGv_i32 arg5,
                                    TCGArg arg6)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg1);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg2);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg3);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg4);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg5);
    *gen_opparam_ptr++ = arg6;
}

static inline void tcg_gen_op6i_i64(TCGOpcode opc, TCGv_i64 arg1, TCGv_i64 arg2, TCGv_i64 arg3, TCGv_i64 arg4, TCGv_i64 arg5,
                                    TCGArg arg6)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg1);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg2);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg3);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg4);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg5);
    *gen_opparam_ptr++ = arg6;
}

static inline void tcg_gen_op6ii_i32(TCGOpcode opc, TCGv_i32 arg1, TCGv_i32 arg2, TCGv_i32 arg3, TCGv_i32 arg4, TCGArg arg5,
                                     TCGArg arg6)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg1);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg2);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg3);
    *gen_opparam_ptr++ = GET_TCGV_I32(arg4);
    *gen_opparam_ptr++ = arg5;
    *gen_opparam_ptr++ = arg6;
}

static inline void tcg_gen_op6ii_i64(TCGOpcode opc, TCGv_i64 arg1, TCGv_i64 arg2, TCGv_i64 arg3, TCGv_i64 arg4, TCGArg arg5,
                                     TCGArg arg6)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg1);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg2);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg3);
    *gen_opparam_ptr++ = GET_TCGV_I64(arg4);
    *gen_opparam_ptr++ = arg5;
    *gen_opparam_ptr++ = arg6;
}

static inline void tcg_gen_op6iiiiii(TCGOpcode opc, TCGv_i64 arg1, TCGv_i64 arg2, TCGv_i64 arg3, TCGv_i64 arg4, TCGArg arg5,
                                     TCGArg arg6)
{
    tcg_gen_op0(opc);
    *gen_opparam_ptr++ = arg1;
    *gen_opparam_ptr++ = arg2;
    *gen_opparam_ptr++ = arg3;
    *gen_opparam_ptr++ = arg4;
    *gen_opparam_ptr++ = arg5;
    *gen_opparam_ptr++ = arg6;
}

static inline void gen_set_label(int n)
{
    tcg_gen_op1i(INDEX_op_set_label, n);
}

static inline void tcg_gen_br(int label)
{
    tcg_gen_op1i(INDEX_op_br, label);
}

static inline void tcg_gen_mov_i32(TCGv_i32 ret, TCGv_i32 arg)
{
    if(!TCGV_EQUAL_I32(ret, arg)) {
        tcg_gen_op2_i32(INDEX_op_mov_i32, ret, arg);
    }
}

static inline void tcg_gen_movi_i32(TCGv_i32 ret, int32_t arg)
{
    tcg_gen_op2i_i32(INDEX_op_movi_i32, ret, arg);
}

/* A version of dh_sizemask from def-helper.h that doesn't rely on
   preprocessor magic.  */
static inline int tcg_gen_sizemask(int n, int is_64bit, int is_signed)
{
    return (is_64bit << n * 2) | (is_signed << (n * 2 + 1));
}

/* helper calls */
static inline void tcg_gen_helperN(void *func, int flags, int sizemask, TCGArg ret, int nargs, TCGArg *args)
{
    TCGv_ptr fn;
    fn = tcg_const_ptr((tcg_target_long)func);
    tcg_gen_callN(tcg->ctx, fn, flags, sizemask, ret, nargs, args);
    tcg_temp_free_ptr(fn);
}

/* Note: Both tcg_gen_helper32() and tcg_gen_helper64() are currently
   reserved for helpers in tcg-runtime.c. These helpers are all const
   and pure, hence the call to tcg_gen_callN() with TCG_CALL_CONST |
   TCG_CALL_PURE. This may need to be adjusted if these functions
   start to be used with other helpers. */
static inline void tcg_gen_helper32(void *func, int sizemask, TCGv_i32 ret, TCGv_i32 a, TCGv_i32 b)
{
    TCGv_ptr fn;
    TCGArg args[2];
    fn = tcg_const_ptr((tcg_target_long)func);
    args[0] = GET_TCGV_I32(a);
    args[1] = GET_TCGV_I32(b);
    tcg_gen_callN(tcg->ctx, fn, TCG_CALL_CONST | TCG_CALL_PURE, sizemask, GET_TCGV_I32(ret), 2, args);
    tcg_temp_free_ptr(fn);
}

static inline void tcg_gen_helper64(void *func, int sizemask, TCGv_i64 ret, TCGv_i64 a, TCGv_i64 b)
{
    TCGv_ptr fn;
    TCGArg args[2];
    fn = tcg_const_ptr((tcg_target_long)func);
    args[0] = GET_TCGV_I64(a);
    args[1] = GET_TCGV_I64(b);
    tcg_gen_callN(tcg->ctx, fn, TCG_CALL_CONST | TCG_CALL_PURE, sizemask, GET_TCGV_I64(ret), 2, args);
    tcg_temp_free_ptr(fn);
}

static inline void tcg_gen_helper32_1_arg(void *func, int sizemask, TCGv_i32 ret, TCGv_i32 a)
{
    TCGv_ptr fn;
    TCGArg args[1];
    fn = tcg_const_ptr((tcg_target_long)func);
    args[0] = GET_TCGV_I32(a);
    tcg_gen_callN(tcg->ctx, fn, TCG_CALL_CONST | TCG_CALL_PURE, sizemask, GET_TCGV_I32(ret), 1, args);
    tcg_temp_free_ptr(fn);
}

static inline void tcg_gen_helper64_1_arg(void *func, int sizemask, TCGv_i64 ret, TCGv_i64 a)
{
    TCGv_ptr fn;
    TCGArg args[1];
    fn = tcg_const_ptr((tcg_target_long)func);
    args[0] = GET_TCGV_I64(a);
    tcg_gen_callN(tcg->ctx, fn, TCG_CALL_CONST | TCG_CALL_PURE, sizemask, GET_TCGV_I64(ret), 1, args);
    tcg_temp_free_ptr(fn);
}

/* 32 bit ops */

static inline void tcg_gen_ld8u_i32(TCGv_i32 ret, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ldst_op_i32(INDEX_op_ld8u_i32, ret, arg2, offset);
}

static inline void tcg_gen_ld8s_i32(TCGv_i32 ret, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ldst_op_i32(INDEX_op_ld8s_i32, ret, arg2, offset);
}

static inline void tcg_gen_ld16u_i32(TCGv_i32 ret, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ldst_op_i32(INDEX_op_ld16u_i32, ret, arg2, offset);
}

static inline void tcg_gen_ld16s_i32(TCGv_i32 ret, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ldst_op_i32(INDEX_op_ld16s_i32, ret, arg2, offset);
}

static inline void tcg_gen_ld_i32(TCGv_i32 ret, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ldst_op_i32(INDEX_op_ld_i32, ret, arg2, offset);
}

static inline void tcg_gen_st8_i32(TCGv_i32 arg1, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ldst_op_i32(INDEX_op_st8_i32, arg1, arg2, offset);
}

static inline void tcg_gen_st16_i32(TCGv_i32 arg1, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ldst_op_i32(INDEX_op_st16_i32, arg1, arg2, offset);
}

static inline void tcg_gen_st_i32(TCGv_i32 arg1, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ldst_op_i32(INDEX_op_st_i32, arg1, arg2, offset);
}

static inline void tcg_gen_add_i32(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    tcg_gen_op3_i32(INDEX_op_add_i32, ret, arg1, arg2);
}

static inline void tcg_gen_addi_i32(TCGv_i32 ret, TCGv_i32 arg1, int32_t arg2)
{
    /* some cases can be optimized here */
    if(arg2 == 0) {
        tcg_gen_mov_i32(ret, arg1);
    } else {
        TCGv_i32 t0 = tcg_const_i32(arg2);
        tcg_gen_add_i32(ret, arg1, t0);
        tcg_temp_free_i32(t0);
    }
}

static inline void tcg_gen_sub_i32(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    tcg_gen_op3_i32(INDEX_op_sub_i32, ret, arg1, arg2);
}

static inline void tcg_gen_subfi_i32(TCGv_i32 ret, int32_t arg1, TCGv_i32 arg2)
{
    TCGv_i32 t0 = tcg_const_i32(arg1);
    tcg_gen_sub_i32(ret, t0, arg2);
    tcg_temp_free_i32(t0);
}

static inline void tcg_gen_subi_i32(TCGv_i32 ret, TCGv_i32 arg1, int32_t arg2)
{
    /* some cases can be optimized here */
    if(arg2 == 0) {
        tcg_gen_mov_i32(ret, arg1);
    } else {
        TCGv_i32 t0 = tcg_const_i32(arg2);
        tcg_gen_sub_i32(ret, arg1, t0);
        tcg_temp_free_i32(t0);
    }
}

static inline void tcg_gen_add2_i32(TCGv_i32 rl, TCGv_i32 rh, TCGv_i32 al, TCGv_i32 ah, TCGv_i32 bl, TCGv_i32 bh)
{
    tcg_gen_op6_i32(INDEX_op_add2_i32, rl, rh, al, ah, bl, bh);
}

static inline void tcg_gen_sub2_i32(TCGv_i32 rl, TCGv_i32 rh, TCGv_i32 al, TCGv_i32 ah, TCGv_i32 bl, TCGv_i32 bh)
{
    tcg_gen_op6_i32(INDEX_op_sub2_i32, rl, rh, al, ah, bl, bh);
}

static inline void tcg_gen_and_i32(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    if(TCGV_EQUAL_I32(arg1, arg2)) {
        tcg_gen_mov_i32(ret, arg1);
    } else {
        tcg_gen_op3_i32(INDEX_op_and_i32, ret, arg1, arg2);
    }
}

static inline void tcg_gen_andi_i32(TCGv_i32 ret, TCGv_i32 arg1, int32_t arg2)
{
    /* some cases can be optimized here */
    if(arg2 == 0) {
        tcg_gen_movi_i32(ret, 0);
    } else if(arg2 == 0xffffffff) {
        tcg_gen_mov_i32(ret, arg1);
    } else {
        TCGv_i32 t0 = tcg_const_i32(arg2);
        tcg_gen_and_i32(ret, arg1, t0);
        tcg_temp_free_i32(t0);
    }
}

static inline void tcg_gen_or_i32(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    if(TCGV_EQUAL_I32(arg1, arg2)) {
        tcg_gen_mov_i32(ret, arg1);
    } else {
        tcg_gen_op3_i32(INDEX_op_or_i32, ret, arg1, arg2);
    }
}

static inline void tcg_gen_ori_i32(TCGv_i32 ret, TCGv_i32 arg1, int32_t arg2)
{
    /* some cases can be optimized here */
    if(arg2 == 0xffffffff) {
        tcg_gen_movi_i32(ret, 0xffffffff);
    } else if(arg2 == 0) {
        tcg_gen_mov_i32(ret, arg1);
    } else {
        TCGv_i32 t0 = tcg_const_i32(arg2);
        tcg_gen_or_i32(ret, arg1, t0);
        tcg_temp_free_i32(t0);
    }
}

static inline void tcg_gen_xor_i32(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    if(TCGV_EQUAL_I32(arg1, arg2)) {
        tcg_gen_movi_i32(ret, 0);
    } else {
        tcg_gen_op3_i32(INDEX_op_xor_i32, ret, arg1, arg2);
    }
}

static inline void tcg_gen_xori_i32(TCGv_i32 ret, TCGv_i32 arg1, int32_t arg2)
{
    /* some cases can be optimized here */
    if(arg2 == 0) {
        tcg_gen_mov_i32(ret, arg1);
    } else {
        TCGv_i32 t0 = tcg_const_i32(arg2);
        tcg_gen_xor_i32(ret, arg1, t0);
        tcg_temp_free_i32(t0);
    }
}

static inline void tcg_gen_shl_i32(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    tcg_gen_op3_i32(INDEX_op_shl_i32, ret, arg1, arg2);
}

static inline void tcg_gen_shli_i32(TCGv_i32 ret, TCGv_i32 arg1, int32_t arg2)
{
    if(arg2 == 0) {
        tcg_gen_mov_i32(ret, arg1);
    } else {
        TCGv_i32 t0 = tcg_const_i32(arg2);
        tcg_gen_shl_i32(ret, arg1, t0);
        tcg_temp_free_i32(t0);
    }
}

static inline void tcg_gen_shr_i32(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    tcg_gen_op3_i32(INDEX_op_shr_i32, ret, arg1, arg2);
}

static inline void tcg_gen_shri_i32(TCGv_i32 ret, TCGv_i32 arg1, int32_t arg2)
{
    if(arg2 == 0) {
        tcg_gen_mov_i32(ret, arg1);
    } else {
        TCGv_i32 t0 = tcg_const_i32(arg2);
        tcg_gen_shr_i32(ret, arg1, t0);
        tcg_temp_free_i32(t0);
    }
}

static inline void tcg_gen_sar_i32(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    tcg_gen_op3_i32(INDEX_op_sar_i32, ret, arg1, arg2);
}

static inline void tcg_gen_sari_i32(TCGv_i32 ret, TCGv_i32 arg1, int32_t arg2)
{
    if(arg2 == 0) {
        tcg_gen_mov_i32(ret, arg1);
    } else {
        TCGv_i32 t0 = tcg_const_i32(arg2);
        tcg_gen_sar_i32(ret, arg1, t0);
        tcg_temp_free_i32(t0);
    }
}

static inline void tcg_gen_brcond_i32(TCGCond cond, TCGv_i32 arg1, TCGv_i32 arg2, int label_index)
{
    tcg_gen_op4ii_i32(INDEX_op_brcond_i32, arg1, arg2, cond, label_index);
}

static inline void tcg_gen_brcondi_i32(TCGCond cond, TCGv_i32 arg1, int32_t arg2, int label_index)
{
    TCGv_i32 t0 = tcg_const_i32(arg2);
    tcg_gen_brcond_i32(cond, arg1, t0, label_index);
    tcg_temp_free_i32(t0);
}

static inline void tcg_gen_setcond_i32(TCGCond cond, TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    tcg_gen_op4i_i32(INDEX_op_setcond_i32, ret, arg1, arg2, cond);
}

static inline void tcg_gen_setcondi_i32(TCGCond cond, TCGv_i32 ret, TCGv_i32 arg1, int32_t arg2)
{
    TCGv_i32 t0 = tcg_const_i32(arg2);
    tcg_gen_setcond_i32(cond, ret, arg1, t0);
    tcg_temp_free_i32(t0);
}

static inline void tcg_gen_mul_i32(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    tcg_gen_op3_i32(INDEX_op_mul_i32, ret, arg1, arg2);
}

static inline void tcg_gen_muli_i32(TCGv_i32 ret, TCGv_i32 arg1, int32_t arg2)
{
    TCGv_i32 t0 = tcg_const_i32(arg2);
    tcg_gen_mul_i32(ret, arg1, t0);
    tcg_temp_free_i32(t0);
}

static inline void tcg_gen_div_i32(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    if(TCG_TARGET_HAS_div_i32) {
        tcg_gen_op3_i32(INDEX_op_div_i32, ret, arg1, arg2);
    } else if(TCG_TARGET_HAS_div2_i32) {
        TCGv_i32 t0 = tcg_temp_new_i32();
        tcg_gen_sari_i32(t0, arg1, 31);
        tcg_gen_op5_i32(INDEX_op_div2_i32, ret, t0, arg1, t0, arg2);
        tcg_temp_free_i32(t0);
    } else {
        int sizemask = 0;
        /* Return value and both arguments are 32-bit and signed.  */
        sizemask |= tcg_gen_sizemask(0, 0, 1);
        sizemask |= tcg_gen_sizemask(1, 0, 1);
        sizemask |= tcg_gen_sizemask(2, 0, 1);
        tcg_gen_helper32(tcg_helper_div_i32, sizemask, ret, arg1, arg2);
    }
}

static inline void tcg_gen_rem_i32(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    if(TCG_TARGET_HAS_div_i32) {
        tcg_gen_op3_i32(INDEX_op_rem_i32, ret, arg1, arg2);
    } else if(TCG_TARGET_HAS_div2_i32) {
        TCGv_i32 t0 = tcg_temp_new_i32();
        tcg_gen_sari_i32(t0, arg1, 31);
        tcg_gen_op5_i32(INDEX_op_div2_i32, t0, ret, arg1, t0, arg2);
        tcg_temp_free_i32(t0);
    } else {
        int sizemask = 0;
        /* Return value and both arguments are 32-bit and signed.  */
        sizemask |= tcg_gen_sizemask(0, 0, 1);
        sizemask |= tcg_gen_sizemask(1, 0, 1);
        sizemask |= tcg_gen_sizemask(2, 0, 1);
        tcg_gen_helper32(tcg_helper_rem_i32, sizemask, ret, arg1, arg2);
    }
}

static inline void tcg_gen_divu_i32(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    if(TCG_TARGET_HAS_div_i32) {
        tcg_gen_op3_i32(INDEX_op_divu_i32, ret, arg1, arg2);
    } else if(TCG_TARGET_HAS_div2_i32) {
        TCGv_i32 t0 = tcg_temp_new_i32();
        tcg_gen_movi_i32(t0, 0);
        tcg_gen_op5_i32(INDEX_op_divu2_i32, ret, t0, arg1, t0, arg2);
        tcg_temp_free_i32(t0);
    } else {
        int sizemask = 0;
        /* Return value and both arguments are 32-bit and unsigned.  */
        sizemask |= tcg_gen_sizemask(0, 0, 0);
        sizemask |= tcg_gen_sizemask(1, 0, 0);
        sizemask |= tcg_gen_sizemask(2, 0, 0);
        tcg_gen_helper32(tcg_helper_divu_i32, sizemask, ret, arg1, arg2);
    }
}

static inline void tcg_gen_remu_i32(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    if(TCG_TARGET_HAS_div_i32) {
        tcg_gen_op3_i32(INDEX_op_remu_i32, ret, arg1, arg2);
    } else if(TCG_TARGET_HAS_div2_i32) {
        TCGv_i32 t0 = tcg_temp_new_i32();
        tcg_gen_movi_i32(t0, 0);
        tcg_gen_op5_i32(INDEX_op_divu2_i32, t0, ret, arg1, t0, arg2);
        tcg_temp_free_i32(t0);
    } else {
        int sizemask = 0;
        /* Return value and both arguments are 32-bit and unsigned.  */
        sizemask |= tcg_gen_sizemask(0, 0, 0);
        sizemask |= tcg_gen_sizemask(1, 0, 0);
        sizemask |= tcg_gen_sizemask(2, 0, 0);
        tcg_gen_helper32(tcg_helper_remu_i32, sizemask, ret, arg1, arg2);
    }
}

#if TCG_TARGET_REG_BITS == 32

static inline void tcg_gen_mov_i64(TCGv_i64 ret, TCGv_i64 arg)
{
    if(!TCGV_EQUAL_I64(ret, arg)) {
        tcg_gen_mov_i32(TCGV_LOW(ret), TCGV_LOW(arg));
        tcg_gen_mov_i32(TCGV_HIGH(ret), TCGV_HIGH(arg));
    }
}

static inline void tcg_gen_movi_i64(TCGv_i64 ret, int64_t arg)
{
    tcg_gen_movi_i32(TCGV_LOW(ret), arg);
    tcg_gen_movi_i32(TCGV_HIGH(ret), arg >> 32);
}

static inline void tcg_gen_ld8u_i64(TCGv_i64 ret, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ld8u_i32(TCGV_LOW(ret), arg2, offset);
    tcg_gen_movi_i32(TCGV_HIGH(ret), 0);
}

static inline void tcg_gen_ld8s_i64(TCGv_i64 ret, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ld8s_i32(TCGV_LOW(ret), arg2, offset);
    tcg_gen_sari_i32(TCGV_HIGH(ret), TCGV_HIGH(ret), 31);
}

static inline void tcg_gen_ld16u_i64(TCGv_i64 ret, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ld16u_i32(TCGV_LOW(ret), arg2, offset);
    tcg_gen_movi_i32(TCGV_HIGH(ret), 0);
}

static inline void tcg_gen_ld16s_i64(TCGv_i64 ret, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ld16s_i32(TCGV_LOW(ret), arg2, offset);
    tcg_gen_sari_i32(TCGV_HIGH(ret), TCGV_LOW(ret), 31);
}

static inline void tcg_gen_ld32u_i64(TCGv_i64 ret, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ld_i32(TCGV_LOW(ret), arg2, offset);
    tcg_gen_movi_i32(TCGV_HIGH(ret), 0);
}

static inline void tcg_gen_ld32s_i64(TCGv_i64 ret, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ld_i32(TCGV_LOW(ret), arg2, offset);
    tcg_gen_sari_i32(TCGV_HIGH(ret), TCGV_LOW(ret), 31);
}

static inline void tcg_gen_ld_i64(TCGv_i64 ret, TCGv_ptr arg2, tcg_target_long offset)
{
    /* since arg2 and ret have different types, they cannot be the
       same temporary */
#ifdef TCG_TARGET_WORDS_BIGENDIAN
    tcg_gen_ld_i32(TCGV_HIGH(ret), arg2, offset);
    tcg_gen_ld_i32(TCGV_LOW(ret), arg2, offset + 4);
#else
    tcg_gen_ld_i32(TCGV_LOW(ret), arg2, offset);
    tcg_gen_ld_i32(TCGV_HIGH(ret), arg2, offset + 4);
#endif
}

static inline void tcg_gen_st8_i64(TCGv_i64 arg1, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_st8_i32(TCGV_LOW(arg1), arg2, offset);
}

static inline void tcg_gen_st16_i64(TCGv_i64 arg1, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_st16_i32(TCGV_LOW(arg1), arg2, offset);
}

static inline void tcg_gen_st32_i64(TCGv_i64 arg1, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_st_i32(TCGV_LOW(arg1), arg2, offset);
}

static inline void tcg_gen_st_i64(TCGv_i64 arg1, TCGv_ptr arg2, tcg_target_long offset)
{
#ifdef TCG_TARGET_WORDS_BIGENDIAN
    tcg_gen_st_i32(TCGV_HIGH(arg1), arg2, offset);
    tcg_gen_st_i32(TCGV_LOW(arg1), arg2, offset + 4);
#else
    tcg_gen_st_i32(TCGV_LOW(arg1), arg2, offset);
    tcg_gen_st_i32(TCGV_HIGH(arg1), arg2, offset + 4);
#endif
}

static inline void tcg_gen_add_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    tcg_gen_op6_i32(INDEX_op_add2_i32, TCGV_LOW(ret), TCGV_HIGH(ret), TCGV_LOW(arg1), TCGV_HIGH(arg1), TCGV_LOW(arg2),
                    TCGV_HIGH(arg2));
}

static inline void tcg_gen_sub_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    tcg_gen_op6_i32(INDEX_op_sub2_i32, TCGV_LOW(ret), TCGV_HIGH(ret), TCGV_LOW(arg1), TCGV_HIGH(arg1), TCGV_LOW(arg2),
                    TCGV_HIGH(arg2));
}

static inline void tcg_gen_and_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    tcg_gen_and_i32(TCGV_LOW(ret), TCGV_LOW(arg1), TCGV_LOW(arg2));
    tcg_gen_and_i32(TCGV_HIGH(ret), TCGV_HIGH(arg1), TCGV_HIGH(arg2));
}

static inline void tcg_gen_andi_i64(TCGv_i64 ret, TCGv_i64 arg1, int64_t arg2)
{
    tcg_gen_andi_i32(TCGV_LOW(ret), TCGV_LOW(arg1), arg2);
    tcg_gen_andi_i32(TCGV_HIGH(ret), TCGV_HIGH(arg1), arg2 >> 32);
}

static inline void tcg_gen_or_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    tcg_gen_or_i32(TCGV_LOW(ret), TCGV_LOW(arg1), TCGV_LOW(arg2));
    tcg_gen_or_i32(TCGV_HIGH(ret), TCGV_HIGH(arg1), TCGV_HIGH(arg2));
}

static inline void tcg_gen_ori_i64(TCGv_i64 ret, TCGv_i64 arg1, int64_t arg2)
{
    tcg_gen_ori_i32(TCGV_LOW(ret), TCGV_LOW(arg1), arg2);
    tcg_gen_ori_i32(TCGV_HIGH(ret), TCGV_HIGH(arg1), arg2 >> 32);
}

static inline void tcg_gen_xor_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    tcg_gen_xor_i32(TCGV_LOW(ret), TCGV_LOW(arg1), TCGV_LOW(arg2));
    tcg_gen_xor_i32(TCGV_HIGH(ret), TCGV_HIGH(arg1), TCGV_HIGH(arg2));
}

static inline void tcg_gen_xori_i64(TCGv_i64 ret, TCGv_i64 arg1, int64_t arg2)
{
    tcg_gen_xori_i32(TCGV_LOW(ret), TCGV_LOW(arg1), arg2);
    tcg_gen_xori_i32(TCGV_HIGH(ret), TCGV_HIGH(arg1), arg2 >> 32);
}

/* XXX: use generic code when basic block handling is OK or CPU
   specific code (x86) */
static inline void tcg_gen_shl_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    int sizemask = 0;
    /* Return value and both arguments are 64-bit and signed.  */
    sizemask |= tcg_gen_sizemask(0, 1, 1);
    sizemask |= tcg_gen_sizemask(1, 1, 1);
    sizemask |= tcg_gen_sizemask(2, 1, 1);

    tcg_gen_helper64(tcg_helper_shl_i64, sizemask, ret, arg1, arg2);
}

static inline void tcg_gen_shli_i64(TCGv_i64 ret, TCGv_i64 arg1, int64_t arg2)
{
    tcg_gen_shifti_i64(ret, arg1, arg2, 0, 0);
}

static inline void tcg_gen_shr_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    int sizemask = 0;
    /* Return value and both arguments are 64-bit and signed.  */
    sizemask |= tcg_gen_sizemask(0, 1, 1);
    sizemask |= tcg_gen_sizemask(1, 1, 1);
    sizemask |= tcg_gen_sizemask(2, 1, 1);

    tcg_gen_helper64(tcg_helper_shr_i64, sizemask, ret, arg1, arg2);
}

static inline void tcg_gen_shri_i64(TCGv_i64 ret, TCGv_i64 arg1, int64_t arg2)
{
    tcg_gen_shifti_i64(ret, arg1, arg2, 1, 0);
}

static inline void tcg_gen_sar_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    int sizemask = 0;
    /* Return value and both arguments are 64-bit and signed.  */
    sizemask |= tcg_gen_sizemask(0, 1, 1);
    sizemask |= tcg_gen_sizemask(1, 1, 1);
    sizemask |= tcg_gen_sizemask(2, 1, 1);

    tcg_gen_helper64(tcg_helper_sar_i64, sizemask, ret, arg1, arg2);
}

static inline void tcg_gen_sari_i64(TCGv_i64 ret, TCGv_i64 arg1, int64_t arg2)
{
    tcg_gen_shifti_i64(ret, arg1, arg2, 1, 1);
}

static inline void tcg_gen_brcond_i64(TCGCond cond, TCGv_i64 arg1, TCGv_i64 arg2, int label_index)
{
    tcg_gen_op6ii_i32(INDEX_op_brcond2_i32, TCGV_LOW(arg1), TCGV_HIGH(arg1), TCGV_LOW(arg2), TCGV_HIGH(arg2), cond, label_index);
}

static inline void tcg_gen_setcond_i64(TCGCond cond, TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    tcg_gen_op6i_i32(INDEX_op_setcond2_i32, TCGV_LOW(ret), TCGV_LOW(arg1), TCGV_HIGH(arg1), TCGV_LOW(arg2), TCGV_HIGH(arg2),
                     cond);
    tcg_gen_movi_i32(TCGV_HIGH(ret), 0);
}

static inline void tcg_gen_mul_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    TCGv_i64 t0;
    TCGv_i32 t1;

    t0 = tcg_temp_new_i64();
    t1 = tcg_temp_new_i32();

    tcg_gen_op4_i32(INDEX_op_mulu2_i32, TCGV_LOW(t0), TCGV_HIGH(t0), TCGV_LOW(arg1), TCGV_LOW(arg2));

    tcg_gen_mul_i32(t1, TCGV_LOW(arg1), TCGV_HIGH(arg2));
    tcg_gen_add_i32(TCGV_HIGH(t0), TCGV_HIGH(t0), t1);
    tcg_gen_mul_i32(t1, TCGV_HIGH(arg1), TCGV_LOW(arg2));
    tcg_gen_add_i32(TCGV_HIGH(t0), TCGV_HIGH(t0), t1);

    tcg_gen_mov_i64(ret, t0);
    tcg_temp_free_i64(t0);
    tcg_temp_free_i32(t1);
}

static inline void tcg_gen_div_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    int sizemask = 0;
    /* Return value and both arguments are 64-bit and signed.  */
    sizemask |= tcg_gen_sizemask(0, 1, 1);
    sizemask |= tcg_gen_sizemask(1, 1, 1);
    sizemask |= tcg_gen_sizemask(2, 1, 1);

    tcg_gen_helper64(tcg_helper_div_i64, sizemask, ret, arg1, arg2);
}

static inline void tcg_gen_rem_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    int sizemask = 0;
    /* Return value and both arguments are 64-bit and signed.  */
    sizemask |= tcg_gen_sizemask(0, 1, 1);
    sizemask |= tcg_gen_sizemask(1, 1, 1);
    sizemask |= tcg_gen_sizemask(2, 1, 1);

    tcg_gen_helper64(tcg_helper_rem_i64, sizemask, ret, arg1, arg2);
}

static inline void tcg_gen_divu_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    int sizemask = 0;
    /* Return value and both arguments are 64-bit and unsigned.  */
    sizemask |= tcg_gen_sizemask(0, 1, 0);
    sizemask |= tcg_gen_sizemask(1, 1, 0);
    sizemask |= tcg_gen_sizemask(2, 1, 0);

    tcg_gen_helper64(tcg_helper_divu_i64, sizemask, ret, arg1, arg2);
}

static inline void tcg_gen_remu_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    int sizemask = 0;
    /* Return value and both arguments are 64-bit and unsigned.  */
    sizemask |= tcg_gen_sizemask(0, 1, 0);
    sizemask |= tcg_gen_sizemask(1, 1, 0);
    sizemask |= tcg_gen_sizemask(2, 1, 0);

    tcg_gen_helper64(tcg_helper_remu_i64, sizemask, ret, arg1, arg2);
}

#else

static inline void tcg_gen_mov_i64(TCGv_i64 ret, TCGv_i64 arg)
{
    if(!TCGV_EQUAL_I64(ret, arg)) {
        tcg_gen_op2_i64(INDEX_op_mov_i64, ret, arg);
    }
}

static inline void tcg_gen_movi_i64(TCGv_i64 ret, int64_t arg)
{
    tcg_gen_op2i_i64(INDEX_op_movi_i64, ret, arg);
}

static inline void tcg_gen_ld8u_i64(TCGv_i64 ret, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ldst_op_i64(INDEX_op_ld8u_i64, ret, arg2, offset);
}

static inline void tcg_gen_ld8s_i64(TCGv_i64 ret, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ldst_op_i64(INDEX_op_ld8s_i64, ret, arg2, offset);
}

static inline void tcg_gen_ld16u_i64(TCGv_i64 ret, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ldst_op_i64(INDEX_op_ld16u_i64, ret, arg2, offset);
}

static inline void tcg_gen_ld16s_i64(TCGv_i64 ret, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ldst_op_i64(INDEX_op_ld16s_i64, ret, arg2, offset);
}

static inline void tcg_gen_ld32u_i64(TCGv_i64 ret, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ldst_op_i64(INDEX_op_ld32u_i64, ret, arg2, offset);
}

static inline void tcg_gen_ld32s_i64(TCGv_i64 ret, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ldst_op_i64(INDEX_op_ld32s_i64, ret, arg2, offset);
}

static inline void tcg_gen_ld_i64(TCGv_i64 ret, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ldst_op_i64(INDEX_op_ld_i64, ret, arg2, offset);
}

static inline void tcg_gen_st8_i64(TCGv_i64 arg1, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ldst_op_i64(INDEX_op_st8_i64, arg1, arg2, offset);
}

static inline void tcg_gen_st16_i64(TCGv_i64 arg1, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ldst_op_i64(INDEX_op_st16_i64, arg1, arg2, offset);
}

static inline void tcg_gen_st32_i64(TCGv_i64 arg1, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ldst_op_i64(INDEX_op_st32_i64, arg1, arg2, offset);
}

static inline void tcg_gen_st_i64(TCGv_i64 arg1, TCGv_ptr arg2, tcg_target_long offset)
{
    tcg_gen_ldst_op_i64(INDEX_op_st_i64, arg1, arg2, offset);
}

static inline void tcg_gen_add_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    tcg_gen_op3_i64(INDEX_op_add_i64, ret, arg1, arg2);
}

static inline void tcg_gen_sub_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    tcg_gen_op3_i64(INDEX_op_sub_i64, ret, arg1, arg2);
}

static inline void tcg_gen_and_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    if(TCGV_EQUAL_I64(arg1, arg2)) {
        tcg_gen_mov_i64(ret, arg1);
    } else {
        tcg_gen_op3_i64(INDEX_op_and_i64, ret, arg1, arg2);
    }
}

static inline void tcg_gen_andi_i64(TCGv_i64 ret, TCGv_i64 arg1, int64_t arg2)
{
    TCGv_i64 t0 = tcg_const_i64(arg2);
    tcg_gen_and_i64(ret, arg1, t0);
    tcg_temp_free_i64(t0);
}

static inline void tcg_gen_or_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    if(TCGV_EQUAL_I64(arg1, arg2)) {
        tcg_gen_mov_i64(ret, arg1);
    } else {
        tcg_gen_op3_i64(INDEX_op_or_i64, ret, arg1, arg2);
    }
}

static inline void tcg_gen_ori_i64(TCGv_i64 ret, TCGv_i64 arg1, int64_t arg2)
{
    TCGv_i64 t0 = tcg_const_i64(arg2);
    tcg_gen_or_i64(ret, arg1, t0);
    tcg_temp_free_i64(t0);
}

static inline void tcg_gen_xor_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    if(TCGV_EQUAL_I64(arg1, arg2)) {
        tcg_gen_movi_i64(ret, 0);
    } else {
        tcg_gen_op3_i64(INDEX_op_xor_i64, ret, arg1, arg2);
    }
}

static inline void tcg_gen_xori_i64(TCGv_i64 ret, TCGv_i64 arg1, int64_t arg2)
{
    TCGv_i64 t0 = tcg_const_i64(arg2);
    tcg_gen_xor_i64(ret, arg1, t0);
    tcg_temp_free_i64(t0);
}

static inline void tcg_gen_shl_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    tcg_gen_op3_i64(INDEX_op_shl_i64, ret, arg1, arg2);
}

static inline void tcg_gen_shli_i64(TCGv_i64 ret, TCGv_i64 arg1, int64_t arg2)
{
    if(arg2 == 0) {
        tcg_gen_mov_i64(ret, arg1);
    } else {
        TCGv_i64 t0 = tcg_const_i64(arg2);
        tcg_gen_shl_i64(ret, arg1, t0);
        tcg_temp_free_i64(t0);
    }
}

static inline void tcg_gen_shr_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    tcg_gen_op3_i64(INDEX_op_shr_i64, ret, arg1, arg2);
}

static inline void tcg_gen_shri_i64(TCGv_i64 ret, TCGv_i64 arg1, int64_t arg2)
{
    if(arg2 == 0) {
        tcg_gen_mov_i64(ret, arg1);
    } else {
        TCGv_i64 t0 = tcg_const_i64(arg2);
        tcg_gen_shr_i64(ret, arg1, t0);
        tcg_temp_free_i64(t0);
    }
}

static inline void tcg_gen_sar_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    tcg_gen_op3_i64(INDEX_op_sar_i64, ret, arg1, arg2);
}

static inline void tcg_gen_sari_i64(TCGv_i64 ret, TCGv_i64 arg1, int64_t arg2)
{
    if(arg2 == 0) {
        tcg_gen_mov_i64(ret, arg1);
    } else {
        TCGv_i64 t0 = tcg_const_i64(arg2);
        tcg_gen_sar_i64(ret, arg1, t0);
        tcg_temp_free_i64(t0);
    }
}

static inline void tcg_gen_brcond_i64(TCGCond cond, TCGv_i64 arg1, TCGv_i64 arg2, int label_index)
{
    tcg_gen_op4ii_i64(INDEX_op_brcond_i64, arg1, arg2, cond, label_index);
}

static inline void tcg_gen_setcond_i64(TCGCond cond, TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    tcg_gen_op4i_i64(INDEX_op_setcond_i64, ret, arg1, arg2, cond);
}

static inline void tcg_gen_mul_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    tcg_gen_op3_i64(INDEX_op_mul_i64, ret, arg1, arg2);
}

static inline void tcg_gen_div_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    if(TCG_TARGET_HAS_div_i64) {
        tcg_gen_op3_i64(INDEX_op_div_i64, ret, arg1, arg2);
    } else if(TCG_TARGET_HAS_div2_i64) {
        TCGv_i64 t0 = tcg_temp_new_i64();
        tcg_gen_sari_i64(t0, arg1, 63);
        tcg_gen_op5_i64(INDEX_op_div2_i64, ret, t0, arg1, t0, arg2);
        tcg_temp_free_i64(t0);
    } else {
        int sizemask = 0;
        /* Return value and both arguments are 64-bit and signed.  */
        sizemask |= tcg_gen_sizemask(0, 1, 1);
        sizemask |= tcg_gen_sizemask(1, 1, 1);
        sizemask |= tcg_gen_sizemask(2, 1, 1);
        tcg_gen_helper64(tcg_helper_div_i64, sizemask, ret, arg1, arg2);
    }
}

static inline void tcg_gen_rem_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    if(TCG_TARGET_HAS_div_i64) {
        tcg_gen_op3_i64(INDEX_op_rem_i64, ret, arg1, arg2);
    } else if(TCG_TARGET_HAS_div2_i64) {
        TCGv_i64 t0 = tcg_temp_new_i64();
        tcg_gen_sari_i64(t0, arg1, 63);
        tcg_gen_op5_i64(INDEX_op_div2_i64, t0, ret, arg1, t0, arg2);
        tcg_temp_free_i64(t0);
    } else {
        int sizemask = 0;
        /* Return value and both arguments are 64-bit and signed.  */
        sizemask |= tcg_gen_sizemask(0, 1, 1);
        sizemask |= tcg_gen_sizemask(1, 1, 1);
        sizemask |= tcg_gen_sizemask(2, 1, 1);
        tcg_gen_helper64(tcg_helper_rem_i64, sizemask, ret, arg1, arg2);
    }
}

static inline void tcg_gen_divu_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    if(TCG_TARGET_HAS_div_i64) {
        tcg_gen_op3_i64(INDEX_op_divu_i64, ret, arg1, arg2);
    } else if(TCG_TARGET_HAS_div2_i64) {
        TCGv_i64 t0 = tcg_temp_new_i64();
        tcg_gen_movi_i64(t0, 0);
        tcg_gen_op5_i64(INDEX_op_divu2_i64, ret, t0, arg1, t0, arg2);
        tcg_temp_free_i64(t0);
    } else {
        int sizemask = 0;
        /* Return value and both arguments are 64-bit and unsigned.  */
        sizemask |= tcg_gen_sizemask(0, 1, 0);
        sizemask |= tcg_gen_sizemask(1, 1, 0);
        sizemask |= tcg_gen_sizemask(2, 1, 0);
        tcg_gen_helper64(tcg_helper_divu_i64, sizemask, ret, arg1, arg2);
    }
}

static inline void tcg_gen_remu_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    if(TCG_TARGET_HAS_div_i64) {
        tcg_gen_op3_i64(INDEX_op_remu_i64, ret, arg1, arg2);
    } else if(TCG_TARGET_HAS_div2_i64) {
        TCGv_i64 t0 = tcg_temp_new_i64();
        tcg_gen_movi_i64(t0, 0);
        tcg_gen_op5_i64(INDEX_op_divu2_i64, t0, ret, arg1, t0, arg2);
        tcg_temp_free_i64(t0);
    } else {
        int sizemask = 0;
        /* Return value and both arguments are 64-bit and unsigned.  */
        sizemask |= tcg_gen_sizemask(0, 1, 0);
        sizemask |= tcg_gen_sizemask(1, 1, 0);
        sizemask |= tcg_gen_sizemask(2, 1, 0);
        tcg_gen_helper64(tcg_helper_remu_i64, sizemask, ret, arg1, arg2);
    }
}
#endif /* TCG_TARGET_REG_BITS == 32 */

static inline void tcg_gen_addi_i64(TCGv_i64 ret, TCGv_i64 arg1, int64_t arg2)
{
    /* some cases can be optimized here */
    if(arg2 == 0) {
        tcg_gen_mov_i64(ret, arg1);
    } else {
        TCGv_i64 t0 = tcg_const_i64(arg2);
        tcg_gen_add_i64(ret, arg1, t0);
        tcg_temp_free_i64(t0);
    }
}

static inline void tcg_gen_subfi_i64(TCGv_i64 ret, int64_t arg1, TCGv_i64 arg2)
{
    TCGv_i64 t0 = tcg_const_i64(arg1);
    tcg_gen_sub_i64(ret, t0, arg2);
    tcg_temp_free_i64(t0);
}

static inline void tcg_gen_subi_i64(TCGv_i64 ret, TCGv_i64 arg1, int64_t arg2)
{
    /* some cases can be optimized here */
    if(arg2 == 0) {
        tcg_gen_mov_i64(ret, arg1);
    } else {
        TCGv_i64 t0 = tcg_const_i64(arg2);
        tcg_gen_sub_i64(ret, arg1, t0);
        tcg_temp_free_i64(t0);
    }
}
static inline void tcg_gen_brcondi_i64(TCGCond cond, TCGv_i64 arg1, int64_t arg2, int label_index)
{
    TCGv_i64 t0 = tcg_const_i64(arg2);
    tcg_gen_brcond_i64(cond, arg1, t0, label_index);
    tcg_temp_free_i64(t0);
}

static inline void tcg_gen_setcondi_i64(TCGCond cond, TCGv_i64 ret, TCGv_i64 arg1, int64_t arg2)
{
    TCGv_i64 t0 = tcg_const_i64(arg2);
    tcg_gen_setcond_i64(cond, ret, arg1, t0);
    tcg_temp_free_i64(t0);
}

static inline void tcg_gen_muli_i64(TCGv_i64 ret, TCGv_i64 arg1, int64_t arg2)
{
    TCGv_i64 t0 = tcg_const_i64(arg2);
    tcg_gen_mul_i64(ret, arg1, t0);
    tcg_temp_free_i64(t0);
}

/***************************************/
/* optional operations */

static inline void tcg_gen_ext8s_i32(TCGv_i32 ret, TCGv_i32 arg)
{
    if(TCG_TARGET_HAS_ext8s_i32) {
        tcg_gen_op2_i32(INDEX_op_ext8s_i32, ret, arg);
    } else {
        tcg_gen_shli_i32(ret, arg, 24);
        tcg_gen_sari_i32(ret, ret, 24);
    }
}

static inline void tcg_gen_ext16s_i32(TCGv_i32 ret, TCGv_i32 arg)
{
    if(TCG_TARGET_HAS_ext16s_i32) {
        tcg_gen_op2_i32(INDEX_op_ext16s_i32, ret, arg);
    } else {
        tcg_gen_shli_i32(ret, arg, 16);
        tcg_gen_sari_i32(ret, ret, 16);
    }
}

static inline void tcg_gen_ext8u_i32(TCGv_i32 ret, TCGv_i32 arg)
{
    if(TCG_TARGET_HAS_ext8u_i32) {
        tcg_gen_op2_i32(INDEX_op_ext8u_i32, ret, arg);
    } else {
        tcg_gen_andi_i32(ret, arg, 0xffu);
    }
}

static inline void tcg_gen_ext16u_i32(TCGv_i32 ret, TCGv_i32 arg)
{
    if(TCG_TARGET_HAS_ext16u_i32) {
        tcg_gen_op2_i32(INDEX_op_ext16u_i32, ret, arg);
    } else {
        tcg_gen_andi_i32(ret, arg, 0xffffu);
    }
}

/* Note: we assume the two high bytes are set to zero */
static inline void tcg_gen_bswap16_i32(TCGv_i32 ret, TCGv_i32 arg, int flags)
{
    /* Only one extension flag may be present. */
    tcg_debug_assert(!(flags & TCG_BSWAP_OS) || !(flags & TCG_BSWAP_OZ));

    if(TCG_TARGET_HAS_bswap16_i32) {
        tcg_gen_op3i_i32(INDEX_op_bswap16_i32, ret, arg, flags);
    } else {
        TCGv_i32 t0 = tcg_temp_new_i32();
        TCGv_i32 t1 = tcg_temp_new_i32();

        tcg_gen_shri_i32(t0, arg, 8);
        if(!(flags & TCG_BSWAP_IZ)) {
            tcg_gen_ext8u_i32(t0, t0);
        }

        if(flags & TCG_BSWAP_OS) {
            tcg_gen_shli_i32(t1, arg, 24);
            tcg_gen_sari_i32(t1, t1, 16);
        } else if(flags & TCG_BSWAP_OZ) {
            tcg_gen_ext8u_i32(t1, arg);
            tcg_gen_shli_i32(t1, t1, 8);
        } else {
            tcg_gen_shli_i32(t1, arg, 8);
            //  Make sure that top bytes are zero
            tcg_gen_andi_i32(t1, t1, 0xffff);
        }

        tcg_gen_or_i32(ret, t0, t1);
        tcg_temp_free_i32(t0);
        tcg_temp_free_i32(t1);
    }
}

static inline void tcg_gen_bswap32_i32(TCGv_i32 ret, TCGv_i32 arg)
{
    if(TCG_TARGET_HAS_bswap32_i32) {
        tcg_gen_op3i_i32(INDEX_op_bswap32_i32, ret, arg, 0);
    } else {
        TCGv_i32 t0, t1;
        t0 = tcg_temp_new_i32();
        t1 = tcg_temp_new_i32();

        tcg_gen_shli_i32(t0, arg, 24);

        tcg_gen_andi_i32(t1, arg, 0x0000ff00);
        tcg_gen_shli_i32(t1, t1, 8);
        tcg_gen_or_i32(t0, t0, t1);

        tcg_gen_shri_i32(t1, arg, 8);
        tcg_gen_andi_i32(t1, t1, 0x0000ff00);
        tcg_gen_or_i32(t0, t0, t1);

        tcg_gen_shri_i32(t1, arg, 24);
        tcg_gen_or_i32(ret, t0, t1);
        tcg_temp_free_i32(t0);
        tcg_temp_free_i32(t1);
    }
}

static inline void tcg_gen_ext8s_i64(TCGv_i64 ret, TCGv_i64 arg)
{
#if TCG_TARGET_REG_BITS == 32
    tcg_gen_ext8s_i32(TCGV_LOW(ret), TCGV_LOW(arg));
    tcg_gen_sari_i32(TCGV_HIGH(ret), TCGV_LOW(ret), 31);
#else
    if(TCG_TARGET_HAS_ext8s_i64) {
        tcg_gen_op2_i64(INDEX_op_ext8s_i64, ret, arg);
    } else {
        tcg_gen_shli_i64(ret, arg, 56);
        tcg_gen_sari_i64(ret, ret, 56);
    }
#endif
}

static inline void tcg_gen_ext16s_i64(TCGv_i64 ret, TCGv_i64 arg)
{
#if TCG_TARGET_REG_BITS == 32
    tcg_gen_ext16s_i32(TCGV_LOW(ret), TCGV_LOW(arg));
    tcg_gen_sari_i32(TCGV_HIGH(ret), TCGV_LOW(ret), 31);
#else
    if(TCG_TARGET_HAS_ext16s_i64) {
        tcg_gen_op2_i64(INDEX_op_ext16s_i64, ret, arg);
    } else {
        tcg_gen_shli_i64(ret, arg, 48);
        tcg_gen_sari_i64(ret, ret, 48);
    }
#endif
}

static inline void tcg_gen_ext32s_i64(TCGv_i64 ret, TCGv_i64 arg)
{
#if TCG_TARGET_REG_BITS == 32
    tcg_gen_mov_i32(TCGV_LOW(ret), TCGV_LOW(arg));
    tcg_gen_sari_i32(TCGV_HIGH(ret), TCGV_LOW(ret), 31);
#else
    if(TCG_TARGET_HAS_ext32s_i64) {
        tcg_gen_op2_i64(INDEX_op_ext32s_i64, ret, arg);
    } else {
        tcg_gen_shli_i64(ret, arg, 32);
        tcg_gen_sari_i64(ret, ret, 32);
    }
#endif
}

static inline void tcg_gen_ext8u_i64(TCGv_i64 ret, TCGv_i64 arg)
{
#if TCG_TARGET_REG_BITS == 32
    tcg_gen_ext8u_i32(TCGV_LOW(ret), TCGV_LOW(arg));
    tcg_gen_movi_i32(TCGV_HIGH(ret), 0);
#else
    if(TCG_TARGET_HAS_ext8u_i64) {
        tcg_gen_op2_i64(INDEX_op_ext8u_i64, ret, arg);
    } else {
        tcg_gen_andi_i64(ret, arg, 0xffu);
    }
#endif
}

static inline void tcg_gen_ext16u_i64(TCGv_i64 ret, TCGv_i64 arg)
{
#if TCG_TARGET_REG_BITS == 32
    tcg_gen_ext16u_i32(TCGV_LOW(ret), TCGV_LOW(arg));
    tcg_gen_movi_i32(TCGV_HIGH(ret), 0);
#else
    if(TCG_TARGET_HAS_ext16u_i64) {
        tcg_gen_op2_i64(INDEX_op_ext16u_i64, ret, arg);
    } else {
        tcg_gen_andi_i64(ret, arg, 0xffffu);
    }
#endif
}

static inline void tcg_gen_ext32u_i64(TCGv_i64 ret, TCGv_i64 arg)
{
#if TCG_TARGET_REG_BITS == 32
    tcg_gen_mov_i32(TCGV_LOW(ret), TCGV_LOW(arg));
    tcg_gen_movi_i32(TCGV_HIGH(ret), 0);
#else
    if(TCG_TARGET_HAS_ext32u_i64) {
        tcg_gen_op2_i64(INDEX_op_ext32u_i64, ret, arg);
    } else {
        tcg_gen_andi_i64(ret, arg, 0xffffffffu);
    }
#endif
}

static inline void tcg_gen_trunc_i64_i32(TCGv_i32 ret, TCGv_i64 arg)
{
#if TCG_TARGET_REG_BITS == 32
    tcg_gen_mov_i32(ret, TCGV_LOW(arg));
#else
    /* Note: we assume the target supports move between 32 and 64 bit
       registers.  This will probably break MIPS64 targets.  */
    tcg_gen_mov_i32(ret, MAKE_TCGV_I32(GET_TCGV_I64(arg)));
#endif
}

static inline void tcg_gen_extu_i32_i64(TCGv_i64 ret, TCGv_i32 arg)
{
#if TCG_TARGET_REG_BITS == 32
    tcg_gen_mov_i32(TCGV_LOW(ret), arg);
    tcg_gen_movi_i32(TCGV_HIGH(ret), 0);
#else
    /* Note: we assume the target supports move between 32 and 64 bit
       registers */
    tcg_gen_ext32u_i64(ret, MAKE_TCGV_I64(GET_TCGV_I32(arg)));
#endif
}

static inline void tcg_gen_ext_i32_i64(TCGv_i64 ret, TCGv_i32 arg)
{
#if TCG_TARGET_REG_BITS == 32
    tcg_gen_mov_i32(TCGV_LOW(ret), arg);
    tcg_gen_sari_i32(TCGV_HIGH(ret), TCGV_LOW(ret), 31);
#else
    /* Note: we assume the target supports move between 32 and 64 bit
       registers */
    tcg_gen_ext32s_i64(ret, MAKE_TCGV_I64(GET_TCGV_I32(arg)));
#endif
}

/* Note: we assume the six high bytes are set to zero */
static inline void tcg_gen_bswap16_i64(TCGv_i64 ret, TCGv_i64 arg, int flags)
{
#if TCG_TARGET_REG_BITS == 32
    tcg_debug_assert(!(flags & TCG_BSWAP_OS) || !(flags & TCG_BSWAP_OZ));
    tcg_gen_bswap16_i32(TCGV_LOW(ret), TCGV_LOW(arg), flags);
    if(flags & TCG_BSWAP_OS) {
        tcg_gen_sari_i32(TCGV_HIGH(ret), TCGV_LOW(ret), 31);
    } else {
        tcg_gen_movi_i32(TCGV_HIGH(ret), 0);
    }
#else
    tcg_debug_assert(!(flags & TCG_BSWAP_OS) || !(flags & TCG_BSWAP_OZ));
    if(TCG_TARGET_HAS_bswap16_i64) {
        tcg_gen_op3i_i64(INDEX_op_bswap16_i64, ret, arg, flags);
    } else {
        TCGv_i64 t0 = tcg_temp_new_i64();
        TCGv_i64 t1 = tcg_temp_new_i64();

        tcg_gen_shri_i64(t0, arg, 8);
        if(!(flags & TCG_BSWAP_IZ)) {
            tcg_gen_ext8u_i64(t0, t0);
        }

        if(flags & TCG_BSWAP_OS) {
            tcg_gen_shli_i64(t1, arg, 56);
            tcg_gen_sari_i64(t1, t1, 48);
        } else if(flags & TCG_BSWAP_OZ) {
            tcg_gen_ext8u_i64(t1, arg);
            tcg_gen_shli_i64(t1, t1, 8);
        } else {
            tcg_gen_shli_i64(t1, arg, 8);
            //  Make sure that top bytes are zero
            tcg_gen_andi_i64(t1, t1, 0xffff);
        }

        tcg_gen_or_i64(ret, t0, t1);
        tcg_temp_free_i64(t0);
        tcg_temp_free_i64(t1);
    }
#endif
}

/* Note: we assume the four high bytes are set to zero */
static inline void tcg_gen_bswap32_i64(TCGv_i64 ret, TCGv_i64 arg, int flags)
{
#if TCG_TARGET_REG_BITS == 32
    tcg_debug_assert(!(flags & TCG_BSWAP_OS) || !(flags & TCG_BSWAP_OZ));
    tcg_gen_bswap32_i32(TCGV_LOW(ret), TCGV_LOW(arg));
    if(flags & TCG_BSWAP_OS) {
        tcg_gen_sari_i32(TCGV_HIGH(ret), TCGV_LOW(ret), 31);
    } else {
        tcg_gen_movi_i32(TCGV_HIGH(ret), 0);
    }
#else
    tcg_debug_assert(!(flags & TCG_BSWAP_OS) || !(flags & TCG_BSWAP_OZ));
    if(TCG_TARGET_HAS_bswap32_i64) {
        tcg_gen_op3i_i64(INDEX_op_bswap32_i64, ret, arg, flags);
    } else {
        TCGv_i64 t0 = tcg_temp_new_i64();
        TCGv_i64 t1 = tcg_temp_new_i64();
        TCGv_i64 t2 = tcg_const_i64(0x00ff00ff);

        /* arg = xxxxabcd */
        tcg_gen_shri_i64(t0, arg, 8); /*  t0 = .xxxxabc */
        tcg_gen_and_i64(t1, arg, t2); /*  t1 = .....b.d */
        tcg_gen_and_i64(t0, t0, t2);  /*  t0 = .....a.c */
        tcg_gen_shli_i64(t1, t1, 8);  /*  t1 = ....b.d. */
        tcg_gen_or_i64(ret, t0, t1);  /* ret = ....badc */

        tcg_gen_shli_i64(t1, ret, 48); /*  t1 = dc...... */
        tcg_gen_shri_i64(t0, ret, 16); /*  t0 = ......ba */
        if(flags & TCG_BSWAP_OS) {
            tcg_gen_sari_i64(t1, t1, 32); /*  t1 = ssssdc.. */
        } else {
            tcg_gen_shri_i64(t1, t1, 32); /*  t1 = ....dc.. */
        }
        tcg_gen_or_i64(ret, t0, t1); /* ret = ssssdcba */

        tcg_temp_free_i64(t0);
        tcg_temp_free_i64(t1);
    }
#endif
}

static inline void tcg_gen_bswap64_i64(TCGv_i64 ret, TCGv_i64 arg)
{
#if TCG_TARGET_REG_BITS == 32
    TCGv_i32 t0, t1;
    t0 = tcg_temp_new_i32();
    t1 = tcg_temp_new_i32();

    tcg_gen_bswap32_i32(t0, TCGV_LOW(arg));
    tcg_gen_bswap32_i32(t1, TCGV_HIGH(arg));
    tcg_gen_mov_i32(TCGV_LOW(ret), t1);
    tcg_gen_mov_i32(TCGV_HIGH(ret), t0);
    tcg_temp_free_i32(t0);
    tcg_temp_free_i32(t1);
#else
    if(TCG_TARGET_HAS_bswap64_i64) {
        tcg_gen_op3i_i64(INDEX_op_bswap64_i64, ret, arg, 0);
    } else {
        TCGv_i64 t0 = tcg_temp_new_i64();
        TCGv_i64 t1 = tcg_temp_new_i64();

        tcg_gen_shli_i64(t0, arg, 56);

        tcg_gen_andi_i64(t1, arg, 0x0000ff00);
        tcg_gen_shli_i64(t1, t1, 40);
        tcg_gen_or_i64(t0, t0, t1);

        tcg_gen_andi_i64(t1, arg, 0x00ff0000);
        tcg_gen_shli_i64(t1, t1, 24);
        tcg_gen_or_i64(t0, t0, t1);

        tcg_gen_andi_i64(t1, arg, 0xff000000);
        tcg_gen_shli_i64(t1, t1, 8);
        tcg_gen_or_i64(t0, t0, t1);

        tcg_gen_shri_i64(t1, arg, 8);
        tcg_gen_andi_i64(t1, t1, 0xff000000);
        tcg_gen_or_i64(t0, t0, t1);

        tcg_gen_shri_i64(t1, arg, 24);
        tcg_gen_andi_i64(t1, t1, 0x00ff0000);
        tcg_gen_or_i64(t0, t0, t1);

        tcg_gen_shri_i64(t1, arg, 40);
        tcg_gen_andi_i64(t1, t1, 0x0000ff00);
        tcg_gen_or_i64(t0, t0, t1);

        tcg_gen_shri_i64(t1, arg, 56);
        tcg_gen_or_i64(ret, t0, t1);
        tcg_temp_free_i64(t0);
        tcg_temp_free_i64(t1);
    }
#endif
}

static inline void tcg_gen_neg_i32(TCGv_i32 ret, TCGv_i32 arg)
{
    if(TCG_TARGET_HAS_neg_i32) {
        tcg_gen_op2_i32(INDEX_op_neg_i32, ret, arg);
    } else {
        TCGv_i32 t0 = tcg_const_i32(0);
        tcg_gen_sub_i32(ret, t0, arg);
        tcg_temp_free_i32(t0);
    }
}

static inline void tcg_gen_neg_i64(TCGv_i64 ret, TCGv_i64 arg)
{
    if(TCG_TARGET_HAS_neg_i64) {
        tcg_gen_op2_i64(INDEX_op_neg_i64, ret, arg);
    } else {
        TCGv_i64 t0 = tcg_const_i64(0);
        tcg_gen_sub_i64(ret, t0, arg);
        tcg_temp_free_i64(t0);
    }
}

static inline void tcg_gen_not_i32(TCGv_i32 ret, TCGv_i32 arg)
{
    if(TCG_TARGET_HAS_not_i32) {
        tcg_gen_op2_i32(INDEX_op_not_i32, ret, arg);
    } else {
        tcg_gen_xori_i32(ret, arg, -1);
    }
}

static inline void tcg_gen_not_i64(TCGv_i64 ret, TCGv_i64 arg)
{
#if TCG_TARGET_REG_BITS == 64
    if(TCG_TARGET_HAS_not_i64) {
        tcg_gen_op2_i64(INDEX_op_not_i64, ret, arg);
    } else {
        tcg_gen_xori_i64(ret, arg, -1);
    }
#else
    tcg_gen_not_i32(TCGV_LOW(ret), TCGV_LOW(arg));
    tcg_gen_not_i32(TCGV_HIGH(ret), TCGV_HIGH(arg));
#endif
}

static inline void tcg_gen_discard_i32(TCGv_i32 arg)
{
    tcg_gen_op1_i32(INDEX_op_discard, arg);
}

static inline void tcg_gen_discard_i64(TCGv_i64 arg)
{
#if TCG_TARGET_REG_BITS == 32
    tcg_gen_discard_i32(TCGV_LOW(arg));
    tcg_gen_discard_i32(TCGV_HIGH(arg));
#else
    tcg_gen_op1_i64(INDEX_op_discard, arg);
#endif
}

static inline void tcg_gen_concat_i32_i64(TCGv_i64 dest, TCGv_i32 low, TCGv_i32 high)
{
#if TCG_TARGET_REG_BITS == 32
    tcg_gen_mov_i32(TCGV_LOW(dest), low);
    tcg_gen_mov_i32(TCGV_HIGH(dest), high);
#else
    TCGv_i64 tmp = tcg_temp_new_i64();
    /* This extension is only needed for type correctness.
       We may be able to do better given target specific information.  */
    tcg_gen_extu_i32_i64(tmp, high);
    tcg_gen_shli_i64(tmp, tmp, 32);
    tcg_gen_extu_i32_i64(dest, low);
    tcg_gen_or_i64(dest, dest, tmp);
    tcg_temp_free_i64(tmp);
#endif
}

static inline void tcg_gen_concat32_i64(TCGv_i64 dest, TCGv_i64 low, TCGv_i64 high)
{
#if TCG_TARGET_REG_BITS == 32
    tcg_gen_concat_i32_i64(dest, TCGV_LOW(low), TCGV_LOW(high));
#else
    TCGv_i64 tmp = tcg_temp_new_i64();
    tcg_gen_ext32u_i64(dest, low);
    tcg_gen_shli_i64(tmp, high, 32);
    tcg_gen_or_i64(dest, dest, tmp);
    tcg_temp_free_i64(tmp);
#endif
}

static inline void tcg_gen_extr_i64_i32(TCGv_i32 lo, TCGv_i32 hi, TCGv_i64 arg)
{
#if TCG_TARGET_REG_BITS == 32
    tcg_gen_mov_i32(lo, TCGV_LOW(arg));
    tcg_gen_mov_i32(hi, TCGV_HIGH(arg));
#else
    TCGv_i64 t0 = tcg_temp_new_i64();
    tcg_gen_trunc_i64_i32(lo, arg);
    tcg_gen_shri_i64(t0, arg, 32);
    tcg_gen_trunc_i64_i32(hi, t0);
    tcg_temp_free_i64(t0);
#endif
}

static inline void tcg_gen_andc_i32(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    if(TCG_TARGET_HAS_andc_i32) {
        tcg_gen_op3_i32(INDEX_op_andc_i32, ret, arg1, arg2);
    } else {
        TCGv_i32 t0 = tcg_temp_new_i32();
        tcg_gen_not_i32(t0, arg2);
        tcg_gen_and_i32(ret, arg1, t0);
        tcg_temp_free_i32(t0);
    }
}

static inline void tcg_gen_andc_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
#if TCG_TARGET_REG_BITS == 64
    if(TCG_TARGET_HAS_andc_i64) {
        tcg_gen_op3_i64(INDEX_op_andc_i64, ret, arg1, arg2);
    } else {
        TCGv_i64 t0 = tcg_temp_new_i64();
        tcg_gen_not_i64(t0, arg2);
        tcg_gen_and_i64(ret, arg1, t0);
        tcg_temp_free_i64(t0);
    }
#else
    tcg_gen_andc_i32(TCGV_LOW(ret), TCGV_LOW(arg1), TCGV_LOW(arg2));
    tcg_gen_andc_i32(TCGV_HIGH(ret), TCGV_HIGH(arg1), TCGV_HIGH(arg2));
#endif
}

static inline void tcg_gen_eqv_i32(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    if(TCG_TARGET_HAS_eqv_i32) {
        tcg_gen_op3_i32(INDEX_op_eqv_i32, ret, arg1, arg2);
    } else {
        tcg_gen_xor_i32(ret, arg1, arg2);
        tcg_gen_not_i32(ret, ret);
    }
}

static inline void tcg_gen_eqv_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
#if TCG_TARGET_REG_BITS == 64
    if(TCG_TARGET_HAS_eqv_i64) {
        tcg_gen_op3_i64(INDEX_op_eqv_i64, ret, arg1, arg2);
    } else {
        tcg_gen_xor_i64(ret, arg1, arg2);
        tcg_gen_not_i64(ret, ret);
    }
#else
    tcg_gen_eqv_i32(TCGV_LOW(ret), TCGV_LOW(arg1), TCGV_LOW(arg2));
    tcg_gen_eqv_i32(TCGV_HIGH(ret), TCGV_HIGH(arg1), TCGV_HIGH(arg2));
#endif
}

static inline void tcg_gen_nand_i32(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    if(TCG_TARGET_HAS_nand_i32) {
        tcg_gen_op3_i32(INDEX_op_nand_i32, ret, arg1, arg2);
    } else {
        tcg_gen_and_i32(ret, arg1, arg2);
        tcg_gen_not_i32(ret, ret);
    }
}

static inline void tcg_gen_nand_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
#if TCG_TARGET_REG_BITS == 64
    if(TCG_TARGET_HAS_nand_i64) {
        tcg_gen_op3_i64(INDEX_op_nand_i64, ret, arg1, arg2);
    } else {
        tcg_gen_and_i64(ret, arg1, arg2);
        tcg_gen_not_i64(ret, ret);
    }
#else
    tcg_gen_nand_i32(TCGV_LOW(ret), TCGV_LOW(arg1), TCGV_LOW(arg2));
    tcg_gen_nand_i32(TCGV_HIGH(ret), TCGV_HIGH(arg1), TCGV_HIGH(arg2));
#endif
}

static inline void tcg_gen_nor_i32(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    if(TCG_TARGET_HAS_nor_i32) {
        tcg_gen_op3_i32(INDEX_op_nor_i32, ret, arg1, arg2);
    } else {
        tcg_gen_or_i32(ret, arg1, arg2);
        tcg_gen_not_i32(ret, ret);
    }
}

static inline void tcg_gen_nor_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
#if TCG_TARGET_REG_BITS == 64
    if(TCG_TARGET_HAS_nor_i64) {
        tcg_gen_op3_i64(INDEX_op_nor_i64, ret, arg1, arg2);
    } else {
        tcg_gen_or_i64(ret, arg1, arg2);
        tcg_gen_not_i64(ret, ret);
    }
#else
    tcg_gen_nor_i32(TCGV_LOW(ret), TCGV_LOW(arg1), TCGV_LOW(arg2));
    tcg_gen_nor_i32(TCGV_HIGH(ret), TCGV_HIGH(arg1), TCGV_HIGH(arg2));
#endif
}

static inline void tcg_gen_orc_i32(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    if(TCG_TARGET_HAS_orc_i32) {
        tcg_gen_op3_i32(INDEX_op_orc_i32, ret, arg1, arg2);
    } else {
        TCGv_i32 t0 = tcg_temp_new_i32();
        tcg_gen_not_i32(t0, arg2);
        tcg_gen_or_i32(ret, arg1, t0);
        tcg_temp_free_i32(t0);
    }
}

static inline void tcg_gen_orc_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
#if TCG_TARGET_REG_BITS == 64
    if(TCG_TARGET_HAS_orc_i64) {
        tcg_gen_op3_i64(INDEX_op_orc_i64, ret, arg1, arg2);
    } else {
        TCGv_i64 t0 = tcg_temp_new_i64();
        tcg_gen_not_i64(t0, arg2);
        tcg_gen_or_i64(ret, arg1, t0);
        tcg_temp_free_i64(t0);
    }
#else
    tcg_gen_orc_i32(TCGV_LOW(ret), TCGV_LOW(arg1), TCGV_LOW(arg2));
    tcg_gen_orc_i32(TCGV_HIGH(ret), TCGV_HIGH(arg1), TCGV_HIGH(arg2));
#endif
}

static inline void tcg_gen_rotl_i32(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    if(TCG_TARGET_HAS_rot_i32) {
        tcg_gen_op3_i32(INDEX_op_rotl_i32, ret, arg1, arg2);
    } else {
        TCGv_i32 t0, t1;

        t0 = tcg_temp_new_i32();
        t1 = tcg_temp_new_i32();
        tcg_gen_shl_i32(t0, arg1, arg2);
        tcg_gen_subfi_i32(t1, 32, arg2);
        tcg_gen_shr_i32(t1, arg1, t1);
        tcg_gen_or_i32(ret, t0, t1);
        tcg_temp_free_i32(t0);
        tcg_temp_free_i32(t1);
    }
}

static inline void tcg_gen_rotl_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    if(TCG_TARGET_HAS_rot_i64) {
        tcg_gen_op3_i64(INDEX_op_rotl_i64, ret, arg1, arg2);
    } else {
        TCGv_i64 t0, t1;
        t0 = tcg_temp_new_i64();
        t1 = tcg_temp_new_i64();
        tcg_gen_shl_i64(t0, arg1, arg2);
        tcg_gen_subfi_i64(t1, 64, arg2);
        tcg_gen_shr_i64(t1, arg1, t1);
        tcg_gen_or_i64(ret, t0, t1);
        tcg_temp_free_i64(t0);
        tcg_temp_free_i64(t1);
    }
}

static inline void tcg_gen_rotli_i32(TCGv_i32 ret, TCGv_i32 arg1, int32_t arg2)
{
    /* some cases can be optimized here */
    if(arg2 == 0) {
        tcg_gen_mov_i32(ret, arg1);
    } else if(TCG_TARGET_HAS_rot_i32) {
        TCGv_i32 t0 = tcg_const_i32(arg2);
        tcg_gen_rotl_i32(ret, arg1, t0);
        tcg_temp_free_i32(t0);
    } else {
        TCGv_i32 t0, t1;
        t0 = tcg_temp_new_i32();
        t1 = tcg_temp_new_i32();
        tcg_gen_shli_i32(t0, arg1, arg2);
        tcg_gen_shri_i32(t1, arg1, 32 - arg2);
        tcg_gen_or_i32(ret, t0, t1);
        tcg_temp_free_i32(t0);
        tcg_temp_free_i32(t1);
    }
}

static inline void tcg_gen_rotli_i64(TCGv_i64 ret, TCGv_i64 arg1, int64_t arg2)
{
    /* some cases can be optimized here */
    if(arg2 == 0) {
        tcg_gen_mov_i64(ret, arg1);
    } else if(TCG_TARGET_HAS_rot_i64) {
        TCGv_i64 t0 = tcg_const_i64(arg2);
        tcg_gen_rotl_i64(ret, arg1, t0);
        tcg_temp_free_i64(t0);
    } else {
        TCGv_i64 t0, t1;
        t0 = tcg_temp_new_i64();
        t1 = tcg_temp_new_i64();
        tcg_gen_shli_i64(t0, arg1, arg2);
        tcg_gen_shri_i64(t1, arg1, 64 - arg2);
        tcg_gen_or_i64(ret, t0, t1);
        tcg_temp_free_i64(t0);
        tcg_temp_free_i64(t1);
    }
}

static inline void tcg_gen_rotr_i32(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
    if(TCG_TARGET_HAS_rot_i32) {
        tcg_gen_op3_i32(INDEX_op_rotr_i32, ret, arg1, arg2);
    } else {
        TCGv_i32 t0, t1;

        t0 = tcg_temp_new_i32();
        t1 = tcg_temp_new_i32();
        tcg_gen_shr_i32(t0, arg1, arg2);
        tcg_gen_subfi_i32(t1, 32, arg2);
        tcg_gen_shl_i32(t1, arg1, t1);
        tcg_gen_or_i32(ret, t0, t1);
        tcg_temp_free_i32(t0);
        tcg_temp_free_i32(t1);
    }
}

static inline void tcg_gen_rotr_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
    if(TCG_TARGET_HAS_rot_i64) {
        tcg_gen_op3_i64(INDEX_op_rotr_i64, ret, arg1, arg2);
    } else {
        TCGv_i64 t0, t1;
        t0 = tcg_temp_new_i64();
        t1 = tcg_temp_new_i64();
        tcg_gen_shr_i64(t0, arg1, arg2);
        tcg_gen_subfi_i64(t1, 64, arg2);
        tcg_gen_shl_i64(t1, arg1, t1);
        tcg_gen_or_i64(ret, t0, t1);
        tcg_temp_free_i64(t0);
        tcg_temp_free_i64(t1);
    }
}

static inline void tcg_gen_rotri_i32(TCGv_i32 ret, TCGv_i32 arg1, int32_t arg2)
{
    /* some cases can be optimized here */
    if(arg2 == 0) {
        tcg_gen_mov_i32(ret, arg1);
    } else {
        tcg_gen_rotli_i32(ret, arg1, 32 - arg2);
    }
}

static inline void tcg_gen_rotri_i64(TCGv_i64 ret, TCGv_i64 arg1, int64_t arg2)
{
    /* some cases can be optimized here */
    if(arg2 == 0) {
        tcg_gen_mov_i64(ret, arg1);
    } else {
        tcg_gen_rotli_i64(ret, arg1, 64 - arg2);
    }
}

static inline void tcg_gen_deposit_i32(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2, unsigned int ofs, unsigned int len)
{
    uint32_t mask;
    TCGv_i32 t1;

    if(ofs == 0 && len == 32) {
        tcg_gen_mov_i32(ret, arg2);
        return;
    }
    if(TCG_TARGET_HAS_deposit_i32 && TCG_TARGET_deposit_i32_valid(ofs, len)) {
        tcg_gen_op5ii_i32(INDEX_op_deposit_i32, ret, arg1, arg2, ofs, len);
        return;
    }

    mask = (1u << len) - 1;
    t1 = tcg_temp_new_i32();

    if(ofs + len < 32) {
        tcg_gen_andi_i32(t1, arg2, mask);
        tcg_gen_shli_i32(t1, t1, ofs);
    } else {
        tcg_gen_shli_i32(t1, arg2, ofs);
    }
    tcg_gen_andi_i32(ret, arg1, ~(mask << ofs));
    tcg_gen_or_i32(ret, ret, t1);

    tcg_temp_free_i32(t1);
}

static inline void tcg_gen_extract_i32(TCGv_i32 ret, TCGv_i32 arg, unsigned int ofs, unsigned int len)
{
    assert(ofs < 32);
    assert(len > 0);
    assert(len <= 32);
    assert(ofs + len <= 32);

    /* Canonicalize certain special cases, even if extract is supported.  */
    if(ofs + len == 32) {
        tcg_gen_shri_i32(ret, arg, 32 - len);
        return;
    }
    if(ofs == 0) {
        tcg_gen_andi_i32(ret, arg, (1u << len) - 1);
        return;
    }

    if(TCG_TARGET_HAS_extract_i32 && TCG_TARGET_extract_i32_valid(ofs, len)) {
        tcg_gen_op4ii_i32(INDEX_op_extract_i32, ret, arg, ofs, len);
        return;
    }

    /* Assume that zero-extension, if available, is cheaper than a shift.  */
    switch(ofs + len) {
        case 16:
            if(TCG_TARGET_HAS_ext16u_i32) {
                tcg_gen_ext16u_i32(ret, arg);
                tcg_gen_shri_i32(ret, ret, ofs);
                return;
            }
            break;
        case 8:
            if(TCG_TARGET_HAS_ext8u_i32) {
                tcg_gen_ext8u_i32(ret, arg);
                tcg_gen_shri_i32(ret, ret, ofs);
                return;
            }
            break;
    }

    /* ??? Ideally we'd know what values are available for immediate AND.
       Assume that 8 bits are available, plus the special case of 16,
       so that we get ext8u, ext16u.  */
    switch(len) {
        case 1 ... 8:
        case 16:
            tcg_gen_shri_i32(ret, arg, ofs);
            tcg_gen_andi_i32(ret, ret, (1u << len) - 1);
            break;
        default:
            tcg_gen_shli_i32(ret, arg, 32 - len - ofs);
            tcg_gen_shri_i32(ret, ret, 32 - len);
            break;
    }
}

/*
 * Extract 32-bits from a 64-bit input, ah:al, starting from ofs.
 * Unlike tcg_gen_extract_i32 above, len is fixed at 32.
 */
static inline void tcg_gen_extract2_i32(TCGv_i32 ret, TCGv_i32 al, TCGv_i32 ah, unsigned int ofs)
{
    tcg_debug_assert(ofs <= 32);
    if(ofs == 0) {
        tcg_gen_mov_i32(ret, al);
    } else if(ofs == 32) {
        tcg_gen_mov_i32(ret, ah);
    } else if(al == ah) {
        tcg_gen_rotri_i32(ret, al, ofs);
//  TODO: Implement extract2_i32 to increase simulation performance.
#if defined(TCG_TARGET_HAS_extract2_i32)
    } else if(TCG_TARGET_HAS_extract2_i32) {
        tcg_gen_op4i_i32(INDEX_op_extract2_i32, ret, al, ah, ofs);
#endif
    } else {
        TCGv_i32 t0 = tcg_temp_new_i32();
        tcg_gen_shri_i32(t0, al, ofs);
        tcg_gen_deposit_i32(ret, t0, ah, 32 - ofs, ofs);
        tcg_temp_free_i32(t0);
    }
}

static inline void tcg_gen_movcond_i32(TCGCond cond, TCGv_i32 ret, TCGv_i32 c1, TCGv_i32 c2, TCGv_i32 v1, TCGv_i32 v2)
{
    if(cond == TCG_COND_ALWAYS) {
        tcg_gen_mov_i32(ret, v1);
    } else if(cond == TCG_COND_NEVER) {
        tcg_gen_mov_i32(ret, v2);
    } else if(TCG_TARGET_HAS_movcond_i32) {
        tcg_gen_op6i_i32(INDEX_op_movcond_i32, ret, c1, c2, v1, v2, cond);
    } else {
        TCGv_i32 t0 = tcg_temp_new_i32();
        TCGv_i32 t1 = tcg_temp_new_i32();
        tcg_gen_setcond_i32(cond, t0, c1, c2);
        tcg_gen_neg_i32(t0, t0);
        tcg_gen_and_i32(t1, v1, t0);
        tcg_gen_andc_i32(ret, v2, t0);
        tcg_gen_or_i32(ret, ret, t1);
        tcg_temp_free_i32(t0);
        tcg_temp_free_i32(t1);
    }
}

static inline void tcg_gen_movcond_i64(TCGCond cond, TCGv_i64 ret, TCGv_i64 c1, TCGv_i64 c2, TCGv_i64 v1, TCGv_i64 v2)
{
    if(cond == TCG_COND_ALWAYS) {
        tcg_gen_mov_i64(ret, v1);
        return;
    } else if(cond == TCG_COND_NEVER) {
        tcg_gen_mov_i64(ret, v2);
        return;
    }
#if TCG_TARGET_REG_BITS == 32
    TCGv_i32 t0 = tcg_temp_new_i32();
    TCGv_i32 t1 = tcg_temp_new_i32();
    tcg_gen_op6i_i32(INDEX_op_setcond2_i32, t0, TCGV_LOW(c1), TCGV_HIGH(c1), TCGV_LOW(c2), TCGV_HIGH(c2), cond);

    if(TCG_TARGET_HAS_movcond_i32) {
        tcg_gen_movi_i32(t1, 0);
        tcg_gen_movcond_i32(TCG_COND_NE, TCGV_LOW(ret), t0, t1, TCGV_LOW(v1), TCGV_LOW(v2));
        tcg_gen_movcond_i32(TCG_COND_NE, TCGV_HIGH(ret), t0, t1, TCGV_HIGH(v1), TCGV_HIGH(v2));
    } else {
        tcg_gen_neg_i32(t0, t0);

        tcg_gen_and_i32(t1, TCGV_LOW(v1), t0);
        tcg_gen_andc_i32(TCGV_LOW(ret), TCGV_LOW(v2), t0);
        tcg_gen_or_i32(TCGV_LOW(ret), TCGV_LOW(ret), t1);

        tcg_gen_and_i32(t1, TCGV_HIGH(v1), t0);
        tcg_gen_andc_i32(TCGV_HIGH(ret), TCGV_HIGH(v2), t0);
        tcg_gen_or_i32(TCGV_HIGH(ret), TCGV_HIGH(ret), t1);
    }
    tcg_temp_free_i32(t0);
    tcg_temp_free_i32(t1);
    return;
#endif
    if(TCG_TARGET_HAS_movcond_i64) {
        tcg_gen_op6i_i64(INDEX_op_movcond_i64, ret, c1, c2, v1, v2, cond);
        return;
    } else {
        TCGv_i64 t0 = tcg_temp_new_i64();
        TCGv_i64 t1 = tcg_temp_new_i64();
        tcg_gen_setcond_i64(cond, t0, c1, c2);
        tcg_gen_neg_i64(t0, t0);
        tcg_gen_and_i64(t1, v1, t0);
        tcg_gen_andc_i64(ret, v2, t0);
        tcg_gen_or_i64(ret, ret, t1);
        tcg_temp_free_i64(t0);
        tcg_temp_free_i64(t1);
    }
}

static inline void tcg_gen_deposit_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2, unsigned int ofs, unsigned int len)
{
    uint64_t mask;
    TCGv_i64 t1;

    if(ofs == 0 && len == 64) {
        tcg_gen_mov_i64(ret, arg2);
        return;
    }
    if(TCG_TARGET_HAS_deposit_i64 && TCG_TARGET_deposit_i64_valid(ofs, len)) {
        tcg_gen_op5ii_i64(INDEX_op_deposit_i64, ret, arg1, arg2, ofs, len);
        return;
    }

#if TCG_TARGET_REG_BITS == 32
    if(ofs >= 32) {
        tcg_gen_mov_i32(TCGV_LOW(ret), TCGV_LOW(arg1));
        tcg_gen_deposit_i32(TCGV_HIGH(ret), TCGV_HIGH(arg1), TCGV_LOW(arg2), ofs - 32, len);
        return;
    }
    if(ofs + len <= 32) {
        tcg_gen_deposit_i32(TCGV_LOW(ret), TCGV_LOW(arg1), TCGV_LOW(arg2), ofs, len);
        tcg_gen_mov_i32(TCGV_HIGH(ret), TCGV_HIGH(arg1));
        return;
    }
#endif

    mask = (1ull << len) - 1;
    t1 = tcg_temp_new_i64();

    if(ofs + len < 64) {
        tcg_gen_andi_i64(t1, arg2, mask);
        tcg_gen_shli_i64(t1, t1, ofs);
    } else {
        tcg_gen_shli_i64(t1, arg2, ofs);
    }
    tcg_gen_andi_i64(ret, arg1, ~(mask << ofs));
    tcg_gen_or_i64(ret, ret, t1);

    tcg_temp_free_i64(t1);
}

static inline void tcg_gen_mulu2_i32(TCGv_i32 rl, TCGv_i32 rh, TCGv_i32 arg1, TCGv_i32 arg2)
{
    if(TCG_TARGET_HAS_mulu2_i32) {
        tcg_gen_op4_i32(INDEX_op_mulu2_i32, rl, rh, arg1, arg2);
        /* Allow the optimizer room to replace mulu2 with two moves.  */
        tcg_gen_op0(INDEX_op_nop);
    } else {
        TCGv_i64 t0 = tcg_temp_new_i64();
        TCGv_i64 t1 = tcg_temp_new_i64();
        tcg_gen_extu_i32_i64(t0, arg1);
        tcg_gen_extu_i32_i64(t1, arg2);
        tcg_gen_mul_i64(t0, t0, t1);
        tcg_gen_extr_i64_i32(rl, rh, t0);
        tcg_temp_free_i64(t0);
        tcg_temp_free_i64(t1);
    }
}

static inline void tcg_gen_muls2_i32(TCGv_i32 rl, TCGv_i32 rh, TCGv_i32 arg1, TCGv_i32 arg2)
{
    if(TCG_TARGET_HAS_muls2_i32) {
        tcg_gen_op4_i32(INDEX_op_muls2_i32, rl, rh, arg1, arg2);
        /* Allow the optimizer room to replace muls2 with two moves.  */
        tcg_gen_op0(INDEX_op_nop);
    } else if(TCG_TARGET_REG_BITS == 32 && TCG_TARGET_HAS_mulu2_i32) {
        TCGv_i32 t0 = tcg_temp_new_i32();
        TCGv_i32 t1 = tcg_temp_new_i32();
        TCGv_i32 t2 = tcg_temp_new_i32();
        TCGv_i32 t3 = tcg_temp_new_i32();
        tcg_gen_op4_i32(INDEX_op_mulu2_i32, t0, t1, arg1, arg2);
        /* Allow the optimizer room to replace mulu2 with two moves.  */
        tcg_gen_op0(INDEX_op_nop);
        /* Adjust for negative inputs.  */
        tcg_gen_sari_i32(t2, arg1, 31);
        tcg_gen_sari_i32(t3, arg2, 31);
        tcg_gen_and_i32(t2, t2, arg2);
        tcg_gen_and_i32(t3, t3, arg1);
        tcg_gen_sub_i32(rh, t1, t2);
        tcg_gen_sub_i32(rh, rh, t3);
        tcg_gen_mov_i32(rl, t0);
        tcg_temp_free_i32(t0);
        tcg_temp_free_i32(t1);
        tcg_temp_free_i32(t2);
        tcg_temp_free_i32(t3);
    } else {
        TCGv_i64 t0 = tcg_temp_new_i64();
        TCGv_i64 t1 = tcg_temp_new_i64();
        tcg_gen_ext_i32_i64(t0, arg1);
        tcg_gen_ext_i32_i64(t1, arg2);
        tcg_gen_mul_i64(t0, t0, t1);
        tcg_gen_extr_i64_i32(rl, rh, t0);
        tcg_temp_free_i64(t0);
        tcg_temp_free_i64(t1);
    }
}

static inline void tcg_gen_sextract_i32(TCGv_i32 ret, TCGv_i32 arg, unsigned int ofs, unsigned int len)
{
    tcg_debug_assert(ofs < 32);
    tcg_debug_assert(len > 0);
    tcg_debug_assert(len <= 32);
    tcg_debug_assert(ofs + len <= 32);

    /* Canonicalize certain special cases, even if extract is supported.  */
    if(ofs + len == 32) {
        tcg_gen_sari_i32(ret, arg, 32 - len);
        return;
    }
    if(ofs == 0) {
        switch(len) {
            case 16:
                tcg_gen_ext16s_i32(ret, arg);
                return;
            case 8:
                tcg_gen_ext8s_i32(ret, arg);
                return;
        }
    }
//  TODO: Implement sextract_i32 to increase simulation performance.
#ifdef TCG_TARGET_HAS_sextract_i32
    if(TCG_TARGET_HAS_sextract_i32 && TCG_TARGET_extract_i32_valid(ofs, len)) {
        tcg_gen_op4ii_i32(INDEX_op_sextract_i32, ret, arg, ofs, len);
        return;
    }
#endif

    /* Assume that sign-extension, if available, is cheaper than a shift.  */
    switch(ofs + len) {
        case 16:
            if(TCG_TARGET_HAS_ext16s_i32) {
                tcg_gen_ext16s_i32(ret, arg);
                tcg_gen_sari_i32(ret, ret, ofs);
                return;
            }
            break;
        case 8:
            if(TCG_TARGET_HAS_ext8s_i32) {
                tcg_gen_ext8s_i32(ret, arg);
                tcg_gen_sari_i32(ret, ret, ofs);
                return;
            }
            break;
    }
    switch(len) {
        case 16:
            if(TCG_TARGET_HAS_ext16s_i32) {
                tcg_gen_shri_i32(ret, arg, ofs);
                tcg_gen_ext16s_i32(ret, ret);
                return;
            }
            break;
        case 8:
            if(TCG_TARGET_HAS_ext8s_i32) {
                tcg_gen_shri_i32(ret, arg, ofs);
                tcg_gen_ext8s_i32(ret, ret);
                return;
            }
            break;
    }

    tcg_gen_shli_i32(ret, arg, 32 - len - ofs);
    tcg_gen_sari_i32(ret, ret, 32 - len);
}

static inline void tcg_gen_mulu2_i64(TCGv_i64 rl, TCGv_i64 rh, TCGv_i64 arg1, TCGv_i64 arg2)
{
    if(TCG_TARGET_HAS_mulu2_i64) {
        tcg_gen_op4_i64(INDEX_op_mulu2_i64, rl, rh, arg1, arg2);
        /* Allow the optimizer room to replace mulu2 with two moves.  */
        tcg_gen_op0(INDEX_op_nop);
    } else if(TCG_TARGET_HAS_mulu2_i64) {
        TCGv_i64 t0 = tcg_temp_new_i64();
        TCGv_i64 t1 = tcg_temp_new_i64();
        TCGv_i64 t2 = tcg_temp_new_i64();
        TCGv_i64 t3 = tcg_temp_new_i64();
        tcg_gen_op4_i64(INDEX_op_mulu2_i64, t0, t1, arg1, arg2);
        /* Allow the optimizer room to replace mulu2 with two moves.  */
        tcg_gen_op0(INDEX_op_nop);
        /* Adjust for negative inputs.  */
        tcg_gen_sari_i64(t2, arg1, 63);
        tcg_gen_sari_i64(t3, arg2, 63);
        tcg_gen_and_i64(t2, t2, arg2);
        tcg_gen_and_i64(t3, t3, arg1);
        tcg_gen_sub_i64(rh, t1, t2);
        tcg_gen_sub_i64(rh, rh, t3);
        tcg_gen_mov_i64(rl, t0);
        tcg_temp_free_i64(t0);
        tcg_temp_free_i64(t1);
        tcg_temp_free_i64(t2);
        tcg_temp_free_i64(t3);
    } else {
        TCGv_i64 t0 = tcg_temp_new_i64();
        int sizemask = 0;
        /* Return value and both arguments are 64-bit and unsigned.  */
        sizemask |= tcg_gen_sizemask(0, 1, 0);
        sizemask |= tcg_gen_sizemask(1, 1, 0);
        sizemask |= tcg_gen_sizemask(2, 1, 0);
        tcg_gen_mul_i64(t0, arg1, arg2);
        tcg_gen_helper64(tcg_helper_muluh_i64, sizemask, rh, arg1, arg2);
        tcg_gen_mov_i64(rl, t0);
        tcg_temp_free_i64(t0);
    }
}

static inline void tcg_gen_muls2_i64(TCGv_i64 rl, TCGv_i64 rh, TCGv_i64 arg1, TCGv_i64 arg2)
{
    if(TCG_TARGET_HAS_muls2_i64) {
        tcg_gen_op4_i64(INDEX_op_muls2_i64, rl, rh, arg1, arg2);
        /* Allow the optimizer room to replace muls2 with two moves.  */
        tcg_gen_op0(INDEX_op_nop);
    } else {
        TCGv_i64 t0 = tcg_temp_new_i64();
        int sizemask = 0;
        /* Return value and both arguments are 64-bit and signed.  */
        sizemask |= tcg_gen_sizemask(0, 1, 1);
        sizemask |= tcg_gen_sizemask(1, 1, 1);
        sizemask |= tcg_gen_sizemask(2, 1, 1);
        tcg_gen_mul_i64(t0, arg1, arg2);
        tcg_gen_helper64(tcg_helper_mulsh_i64, sizemask, rh, arg1, arg2);
        tcg_gen_mov_i64(rl, t0);
        tcg_temp_free_i64(t0);
    }
}

static inline void tcg_gen_abs_i64(TCGv_i64 ret, TCGv_i64 a)
{
    TCGv_i64 t = tcg_temp_new_i64();

    tcg_gen_sari_i64(t, a, 63);
    tcg_gen_xor_i64(ret, a, t);
    tcg_gen_sub_i64(ret, ret, t);
    tcg_temp_free_i64(t);
}

static inline void tcg_gen_add2_i64(TCGv_i64 rl, TCGv_i64 rh, TCGv_i64 al, TCGv_i64 ah, TCGv_i64 bl, TCGv_i64 bh)
{
//  TODO: Implement add2_i64 to increase simulation performance.
#if defined(TCG_TARGET_HAS_add2_i64)
    if(TCG_TARGET_HAS_add2_i64) {
        tcg_gen_op6_i64(INDEX_op_add2_i64, rl, rh, al, ah, bl, bh);
        return;
    }
#endif
    TCGv_i64 t0 = tcg_temp_new_i64();
    TCGv_i64 t1 = tcg_temp_new_i64();
    tcg_gen_add_i64(t0, al, bl);
    tcg_gen_setcond_i64(TCG_COND_LTU, t1, t0, al);
    tcg_gen_add_i64(rh, ah, bh);
    tcg_gen_add_i64(rh, rh, t1);
    tcg_gen_mov_i64(rl, t0);
    tcg_temp_free_i64(t0);
    tcg_temp_free_i64(t1);
}

static inline void tcg_gen_smin_i64(TCGv_i64 ret, TCGv_i64 a, TCGv_i64 b)
{
    tcg_gen_movcond_i64(TCG_COND_LT, ret, a, b, a, b);
}

static inline void tcg_gen_umin_i64(TCGv_i64 ret, TCGv_i64 a, TCGv_i64 b)
{
    tcg_gen_movcond_i64(TCG_COND_LTU, ret, a, b, a, b);
}

static inline void tcg_gen_smax_i64(TCGv_i64 ret, TCGv_i64 a, TCGv_i64 b)
{
    tcg_gen_movcond_i64(TCG_COND_LT, ret, a, b, b, a);
}

static inline void tcg_gen_umax_i64(TCGv_i64 ret, TCGv_i64 a, TCGv_i64 b)
{
    tcg_gen_movcond_i64(TCG_COND_LTU, ret, a, b, b, a);
}

#ifndef TARGET_LONG_BITS
#error must include emulated target headers
#endif

#if TARGET_LONG_BITS == 32
#define TCGv                          TCGv_i32
#define tcg_temp_new()                tcg_temp_new_i32()
#define tcg_global_reg_new            tcg_global_reg_new_i32
#define tcg_global_mem_new            tcg_global_mem_new_i32
#define tcg_temp_local_new()          tcg_temp_local_new_i32()
#define tcg_temp_free                 tcg_temp_free_i32
#define tcg_gen_qemu_ld_op            tcg_gen_op3i_i32
#define tcg_gen_qemu_st_op            tcg_gen_st_op_i32
#define tcg_gen_qemu_st_op_unsafe     tcg_gen_op3i_i32
#define tcg_gen_qemu_ld_op_i64        tcg_gen_qemu_ldst_op_i64_i32
#define tcg_gen_qemu_st_op_i64        tcg_gen_qemu_st_op_i64_i32
#define tcg_gen_qemu_st_op_i64_unsafe tcg_gen_qemu_ldst_op_i64_i32
#define TCGV_UNUSED(x)                TCGV_UNUSED_I32(x)
#define TCGV_EQUAL(a, b)              TCGV_EQUAL_I32(a, b)
#else
#define TCGv                          TCGv_i64
#define tcg_temp_new()                tcg_temp_new_i64()
#define tcg_global_reg_new            tcg_global_reg_new_i64
#define tcg_global_mem_new            tcg_global_mem_new_i64
#define tcg_temp_local_new()          tcg_temp_local_new_i64()
#define tcg_temp_free                 tcg_temp_free_i64
#define tcg_gen_qemu_ld_op            tcg_gen_op3i_i64
#define tcg_gen_qemu_st_op            tcg_gen_st_op_i64
#define tcg_gen_qemu_st_op_unsafe     tcg_gen_op3i_i64
#define tcg_gen_qemu_ld_op_i64        tcg_gen_qemu_ldst_op_i64_i64
#define tcg_gen_qemu_st_op_i64        tcg_gen_qemu_st_op_i64_i64
#define tcg_gen_qemu_st_op_i64_unsafe tcg_gen_qemu_ldst_op_i64_i64
#define TCGV_UNUSED(x)                TCGV_UNUSED_I64(x)
#define TCGV_EQUAL(a, b)              TCGV_EQUAL_I64(a, b)
#endif

#if TARGET_INSN_START_WORDS == 1
#if TARGET_LONG_BITS <= TCG_TARGET_REG_BITS
static inline void tcg_gen_insn_start(target_ulong pc)
{
    tcg_gen_op1i(INDEX_op_insn_start, pc);
}
#else
static inline void tcg_gen_insn_start(target_ulong pc)
{
    tcg_gen_op2ii(INDEX_op_insn_start, (uint32_t)pc, (uint32_t)(pc >> 32));
}
#endif
#elif TARGET_INSN_START_WORDS == 2
#if TARGET_LONG_BITS <= TCG_TARGET_REG_BITS
static inline void tcg_gen_insn_start(target_ulong pc, target_ulong a1)
{
    tcg_gen_op2ii(INDEX_op_insn_start, pc, a1);
}
#else
static inline void tcg_gen_insn_start(target_ulong pc, target_ulong a1)
{
    tcg_gen_op4iiii(INDEX_op_insn_start, (uint32_t)pc, (uint32_t)(pc >> 32), (uint32_t)a1, (uint32_t)(a1 >> 32));
}
#endif
#elif TARGET_INSN_START_WORDS == 3
#if TARGET_LONG_BITS <= TCG_TARGET_REG_BITS
static inline void tcg_gen_insn_start(target_ulong pc, target_ulong a1, target_ulong a2)
{
    tcg_gen_op3iii(INDEX_op_insn_start, pc, a1, a2);
}
#else
static inline void tcg_gen_insn_start(target_ulong pc, target_ulong a1, target_ulong a2)
{
    tcg_gen_op6iiiiii(INDEX_op_insn_start, (uint32_t)pc, (uint32_t)(pc >> 32), (uint32_t)a1, (uint32_t)(a1 >> 32), (uint32_t)a2,
                      (uint32_t)(a2 >> 32));
}
#endif
#else
#error "Unhandled number of operands to insn_start"
#endif

//  This function most likely shouldn't be called directly
//  Use `gen_exit_tb` or `gen_exit_tb_no_chaining` instead
static inline void tcg_gen_exit_tb(tcg_target_long val)
{
    tcg_gen_op1i(INDEX_op_exit_tb, val);
}

static inline void tcg_gen_goto_tb(int idx)
{
    tcg_gen_op1i(INDEX_op_goto_tb, idx);
}

#define TB_HELPER_FROM_TCG_OP
#include "../include/tb-helper.h"
#undef TB_HELPER_FROM_TCG_OP

/* handle guest stores */

static inline void tcg_gen_qemu_st_op_i64_i32(TCGOpcode opc, TCGv_i64 val, TCGv_i32 addr, TCGArg mem_index)
{
    gen_store_table_lock(cpu, addr);
    tcg_request_block_interrupt_check();
    tcg_gen_qemu_ldst_op_i64_i32(opc, val, addr, mem_index);
    gen_store_table_set(cpu, addr);
    gen_store_table_unlock(cpu, addr);
}

static inline void tcg_gen_st_op_i32(TCGOpcode opc, TCGv_i32 arg, TCGv_i32 addr, TCGArg mem_index)
{
    gen_store_table_lock(cpu, addr);
    tcg_request_block_interrupt_check();
    tcg_gen_op3i_i32(opc, arg, addr, mem_index);
    gen_store_table_set(cpu, addr);
    gen_store_table_unlock(cpu, addr);
}

static inline void tcg_gen_qemu_st_op_i64_i64(TCGOpcode opc, TCGv_i64 val, TCGv_i64 addr, TCGArg mem_index)
{
    gen_store_table_lock(cpu, addr);
    tcg_request_block_interrupt_check();
    tcg_gen_qemu_ldst_op_i64_i64(opc, val, addr, mem_index);
    gen_store_table_set(cpu, addr);
    gen_store_table_unlock(cpu, addr);
}

static inline void tcg_gen_st_op_i64(TCGOpcode opc, TCGv_i64 arg, TCGv_i64 addr, TCGArg mem_index)
{
    gen_store_table_lock(cpu, addr);
    tcg_request_block_interrupt_check();
    tcg_gen_op3i_i64(opc, arg, addr, mem_index);
    gen_store_table_set(cpu, addr);
    gen_store_table_unlock(cpu, addr);
}

#if TCG_TARGET_REG_BITS == 32
static inline void tcg_gen_qemu_ld8u(TCGv ret, TCGv addr, int mem_index)
{
#if TARGET_LONG_BITS == 32
    tcg_gen_op3i_i32(INDEX_op_qemu_ld8u, ret, addr, mem_index);
#else
    tcg_gen_op4i_i32(INDEX_op_qemu_ld8u, TCGV_LOW(ret), TCGV_LOW(addr), TCGV_HIGH(addr), mem_index);
    tcg_gen_movi_i32(TCGV_HIGH(ret), 0);
#endif
}

static inline void tcg_gen_qemu_ld8s(TCGv ret, TCGv addr, int mem_index)
{
#if TARGET_LONG_BITS == 32
    tcg_gen_op3i_i32(INDEX_op_qemu_ld8s, ret, addr, mem_index);
#else
    tcg_gen_op4i_i32(INDEX_op_qemu_ld8s, TCGV_LOW(ret), TCGV_LOW(addr), TCGV_HIGH(addr), mem_index);
    tcg_gen_sari_i32(TCGV_HIGH(ret), TCGV_LOW(ret), 31);
#endif
}

static inline void tcg_gen_qemu_ld16u(TCGv ret, TCGv addr, int mem_index)
{
#if TARGET_LONG_BITS == 32
    tcg_gen_op3i_i32(INDEX_op_qemu_ld16u, ret, addr, mem_index);
#else
    tcg_gen_op4i_i32(INDEX_op_qemu_ld16u, TCGV_LOW(ret), TCGV_LOW(addr), TCGV_HIGH(addr), mem_index);
    tcg_gen_movi_i32(TCGV_HIGH(ret), 0);
#endif
}

static inline void tcg_gen_qemu_ld16s(TCGv ret, TCGv addr, int mem_index)
{
#if TARGET_LONG_BITS == 32
    tcg_gen_op3i_i32(INDEX_op_qemu_ld16s, ret, addr, mem_index);
#else
    tcg_gen_op4i_i32(INDEX_op_qemu_ld16s, TCGV_LOW(ret), TCGV_LOW(addr), TCGV_HIGH(addr), mem_index);
    tcg_gen_sari_i32(TCGV_HIGH(ret), TCGV_LOW(ret), 31);
#endif
}

static inline void tcg_gen_qemu_ld32u(TCGv ret, TCGv addr, int mem_index)
{
#if TARGET_LONG_BITS == 32
    tcg_gen_op3i_i32(INDEX_op_qemu_ld32, ret, addr, mem_index);
#else
    tcg_gen_op4i_i32(INDEX_op_qemu_ld32, TCGV_LOW(ret), TCGV_LOW(addr), TCGV_HIGH(addr), mem_index);
    tcg_gen_movi_i32(TCGV_HIGH(ret), 0);
#endif
}

static inline void tcg_gen_qemu_ld32s(TCGv ret, TCGv addr, int mem_index)
{
#if TARGET_LONG_BITS == 32
    tcg_gen_op3i_i32(INDEX_op_qemu_ld32, ret, addr, mem_index);
#else
    tcg_gen_op4i_i32(INDEX_op_qemu_ld32, TCGV_LOW(ret), TCGV_LOW(addr), TCGV_HIGH(addr), mem_index);
    tcg_gen_sari_i32(TCGV_HIGH(ret), TCGV_LOW(ret), 31);
#endif
}

static inline void tcg_gen_qemu_ld64(TCGv_i64 ret, TCGv addr, int mem_index)
{
#if TARGET_LONG_BITS == 32
    tcg_gen_op4i_i32(INDEX_op_qemu_ld64, TCGV_LOW(ret), TCGV_HIGH(ret), addr, mem_index);
#else
    tcg_gen_op5i_i32(INDEX_op_qemu_ld64, TCGV_LOW(ret), TCGV_HIGH(ret), TCGV_LOW(addr), TCGV_HIGH(addr), mem_index);
#endif
}

static inline void tcg_gen_qemu_st8(TCGv arg, TCGv addr, int mem_index)
{
    gen_store_table_lock(cpu, addr);
    tcg_request_block_interrupt_check();
#if TARGET_LONG_BITS == 32
    tcg_gen_op3i_i32(INDEX_op_qemu_st8, arg, addr, mem_index);
#else
    tcg_gen_op4i_i32(INDEX_op_qemu_st8, TCGV_LOW(arg), TCGV_LOW(addr), TCGV_HIGH(addr), mem_index);
#endif
    gen_store_table_set(cpu, addr);
    gen_store_table_unlock(cpu, addr);
}

static inline void tcg_gen_qemu_st16(TCGv arg, TCGv addr, int mem_index)
{
    gen_store_table_lock(cpu, addr);
    tcg_request_block_interrupt_check();
#if TARGET_LONG_BITS == 32
    tcg_gen_op3i_i32(INDEX_op_qemu_st16, arg, addr, mem_index);
#else
    tcg_gen_op4i_i32(INDEX_op_qemu_st16, TCGV_LOW(arg), TCGV_LOW(addr), TCGV_HIGH(addr), mem_index);
#endif
    gen_store_table_set(cpu, addr);
    gen_store_table_unlock(cpu, addr);
}

static inline void tcg_gen_qemu_st32(TCGv arg, TCGv addr, int mem_index)
{
    gen_store_table_lock(cpu, addr);
    tcg_request_block_interrupt_check();
#if TARGET_LONG_BITS == 32
    tcg_gen_op3i_i32(INDEX_op_qemu_st32, arg, addr, mem_index);
#else
    tcg_gen_op4i_i32(INDEX_op_qemu_st32, TCGV_LOW(arg), TCGV_LOW(addr), TCGV_HIGH(addr), mem_index);
#endif
    gen_store_table_set(cpu, addr);
    gen_store_table_unlock(cpu, addr);
}

static inline void tcg_gen_qemu_st64(TCGv_i64 arg, TCGv addr, int mem_index)
{
    gen_store_table_lock(cpu, addr);
    tcg_request_block_interrupt_check();
#if TARGET_LONG_BITS == 32
    tcg_gen_op4i_i32(INDEX_op_qemu_st64, TCGV_LOW(arg), TCGV_HIGH(arg), addr, mem_index);
#else
    tcg_gen_op5i_i32(INDEX_op_qemu_st64, TCGV_LOW(arg), TCGV_HIGH(arg), TCGV_LOW(addr), TCGV_HIGH(addr), mem_index);
#endif
    gen_store_table_set(cpu, addr);
    gen_store_table_unlock(cpu, addr);
}

#define tcg_gen_ld_ptr(R, A, O) tcg_gen_ld_i32(TCGV_PTR_TO_NAT(R), (A), (O))
#define tcg_gen_discard_ptr(A)  tcg_gen_discard_i32(TCGV_PTR_TO_NAT(A))

#else /* TCG_TARGET_REG_BITS == 32 */

static inline void tcg_gen_qemu_ld8u(TCGv ret, TCGv addr, int mem_index)
{
    tcg_gen_qemu_ld_op(INDEX_op_qemu_ld8u, ret, addr, mem_index);
}

static inline void tcg_gen_qemu_ld8s(TCGv ret, TCGv addr, int mem_index)
{
    tcg_gen_qemu_ld_op(INDEX_op_qemu_ld8s, ret, addr, mem_index);
}

static inline void tcg_gen_qemu_ld16u(TCGv ret, TCGv addr, int mem_index)
{
    tcg_gen_qemu_ld_op(INDEX_op_qemu_ld16u, ret, addr, mem_index);
}

static inline void tcg_gen_qemu_ld16s(TCGv ret, TCGv addr, int mem_index)
{
    tcg_gen_qemu_ld_op(INDEX_op_qemu_ld16s, ret, addr, mem_index);
}

static inline void tcg_gen_qemu_ld32u(TCGv ret, TCGv addr, int mem_index)
{
#if TARGET_LONG_BITS == 32
    tcg_gen_qemu_ld_op(INDEX_op_qemu_ld32, ret, addr, mem_index);
#else
    tcg_gen_qemu_ld_op(INDEX_op_qemu_ld32u, ret, addr, mem_index);
#endif
}

static inline void tcg_gen_qemu_ld32s(TCGv ret, TCGv addr, int mem_index)
{
#if TARGET_LONG_BITS == 32
    tcg_gen_qemu_ld_op(INDEX_op_qemu_ld32, ret, addr, mem_index);
#else
    tcg_gen_qemu_ld_op(INDEX_op_qemu_ld32s, ret, addr, mem_index);
#endif
}

static inline void tcg_gen_qemu_ld64(TCGv_i64 ret, TCGv addr, int mem_index)
{
    tcg_gen_qemu_ld_op_i64(INDEX_op_qemu_ld64, ret, addr, mem_index);
}

static inline void tcg_gen_qemu_st8(TCGv arg, TCGv addr, int mem_index)
{
    tcg_request_block_interrupt_check();
    tcg_gen_qemu_st_op(INDEX_op_qemu_st8, arg, addr, mem_index);
}

static inline void tcg_gen_qemu_st16(TCGv arg, TCGv addr, int mem_index)
{
    tcg_request_block_interrupt_check();
    tcg_gen_qemu_st_op(INDEX_op_qemu_st16, arg, addr, mem_index);
}

static inline void tcg_gen_qemu_st32(TCGv arg, TCGv addr, int mem_index)
{
    tcg_request_block_interrupt_check();
    tcg_gen_qemu_st_op(INDEX_op_qemu_st32, arg, addr, mem_index);
}

/*
 * Performs an "unsafe" store.
 * This means that it will NOT acquire the hash table lock and
 * NOT invalidate any memory reservation.
 */
static inline void tcg_gen_qemu_st32_unsafe(TCGv arg, TCGv addr, int mem_index)
{
    tcg_request_block_interrupt_check();
    tcg_gen_qemu_st_op_unsafe(INDEX_op_qemu_st32, arg, addr, mem_index);
}

static inline void tcg_gen_qemu_st64(TCGv_i64 arg, TCGv addr, int mem_index)
{
    tcg_request_block_interrupt_check();
    tcg_gen_qemu_st_op_i64(INDEX_op_qemu_st64, arg, addr, mem_index);
}

/*
 * Performs an "unsafe" store.
 * This means that it will NOT acquire the hash table lock and
 * NOT invalidate any memory reservation.
 */
static inline void tcg_gen_qemu_st64_unsafe(TCGv_i64 arg, TCGv addr, int mem_index)
{
    tcg_request_block_interrupt_check();
    tcg_gen_qemu_st_op_i64_unsafe(INDEX_op_qemu_st64, arg, addr, mem_index);
}

#define tcg_gen_ld_ptr(R, A, O) tcg_gen_ld_i64(TCGV_PTR_TO_NAT(R), (A), (O))
#define tcg_gen_discard_ptr(A)  tcg_gen_discard_i64(TCGV_PTR_TO_NAT(A))

#endif /* TCG_TARGET_REG_BITS != 32 */

//  From tcg/tcg.c @ f713d6ad7b9f52129695d5e3e63541abcd0375c0
static const TCGOpcode old_ld_opc[8] = {
    [MO_UB] = INDEX_op_qemu_ld8u, [MO_SB] = INDEX_op_qemu_ld8s, [MO_UW] = INDEX_op_qemu_ld16u, [MO_SW] = INDEX_op_qemu_ld16s,
#if TCG_TARGET_REG_BITS == 32
    [MO_UL] = INDEX_op_qemu_ld32, [MO_SL] = INDEX_op_qemu_ld32,
#else
    [MO_UL] = INDEX_op_qemu_ld32u, [MO_SL] = INDEX_op_qemu_ld32s,
#endif
    [MO_Q] = INDEX_op_qemu_ld64,
};

static const TCGOpcode old_st_opc[4] = {
    [MO_UB] = INDEX_op_qemu_st8,
    [MO_UW] = INDEX_op_qemu_st16,
    [MO_UL] = INDEX_op_qemu_st32,
    [MO_Q] = INDEX_op_qemu_st64,
};

static inline unsigned get_alignment_bits(TCGMemOp memop)
{
    unsigned a = memop & MO_AMASK;

    if(a == MO_UNALN) {
        /* No alignment required.  */
        a = 0;
    } else if(a == MO_ALIGN) {
        /* A natural alignment requirement.  */
        a = memop & MO_SIZE;
    } else {
        /* A specific alignment requirement.  */
        a = a >> MO_ASHIFT;
    }
#if defined(CONFIG_SOFTMMU)
    /* The requested alignment cannot overlap the TLB flags.  */
    tcg_debug_assert((TLB_FLAGS_MASK & ((1 << a) - 1)) == 0);
#endif
    return a;
}

static inline void tcg_gen_mb(TCGBar mb_type)
{
    if(unlikely(tcg_context_are_multiple_cpus_registered())) {
        tcg_gen_op1i(INDEX_op_mb, mb_type);
    }
}

static inline void tcg_gen_req_mo(TCGBar type)
{
//  No define means guest architecture gives no memory ordering guarantees.
//  Otherwise, if guest guarantees are stronger than the target ones, e.g.
//  i386 simulated on Arm host, memory barrier will be generated for each
//  memory access in multicore setups.
#ifdef TCG_GUEST_DEFAULT_MO
    type &= TCG_GUEST_DEFAULT_MO;
    type &= ~TCG_TARGET_DEFAULT_MO;
    if(type) {
        tcg_gen_mb(type | TCG_BAR_SC);
    }
#endif
}

static inline TCGMemOp tcg_canonicalize_memop(TCGMemOp op, bool is64, bool st)
{
    unsigned a_bits = get_alignment_bits(op);

    if(a_bits == (op & MO_SIZE)) {
        op = (op & ~MO_AMASK) | MO_ALIGN;
    }
    switch(op & MO_SIZE) {
        case MO_8:
            op &= ~MO_BSWAP;
            break;
        case MO_16:
            break;
        case MO_32:
            if(!is64) {
                op &= ~MO_SIGN;
            }
            break;
        case MO_64:
            if(!is64) {
                tcg_abort();
            }
            break;
    }
    if(st) {
        op &= ~MO_SIGN;
    }
    return op;
}

static inline void tcg_gen_qemu_ld_i32(TCGv_i32 val, TCGv addr, TCGArg idx, TCGMemOp memop)
{
    TCGMemOp orig_memop;
    tcg_gen_req_mo(TCG_MO_LD_LD | TCG_MO_ST_LD);
    memop = tcg_canonicalize_memop(memop, 0, 0);
    orig_memop = memop;
    if(!TCG_TARGET_HAS_MEMORY_BSWAP && (memop & MO_BSWAP)) {
        memop &= ~MO_BSWAP;
        /* The bswap primitive benefits from zero-extended input.  */
        if((memop & MO_SSIZE) == MO_SW) {
            memop &= ~MO_SIGN;
        }
    }

    tcg_gen_qemu_ld_op(old_ld_opc[memop & MO_SSIZE], val, addr, idx);

    if((orig_memop ^ memop) & MO_BSWAP) {
        switch(orig_memop & MO_SIZE) {
            case MO_16:
                tcg_gen_bswap16_i32(val, val, (orig_memop & MO_SIGN ? TCG_BSWAP_IZ | TCG_BSWAP_OS : TCG_BSWAP_IZ | TCG_BSWAP_OZ));
                break;
            case MO_32:
                tcg_gen_bswap32_i32(val, val);
                break;
            default:
                tcg_abort();
                __builtin_unreachable();
        }
    }
}

typedef void (*tcg_st_op_func32)(TCGOpcode opc, TCGv_i32 val, TCGv_i32 addr, TCGArg idx);

static inline void tcg_gen_qemu_st_i32_common(TCGv_i32 val, TCGv_i32 addr, TCGArg idx, TCGMemOp memop,
                                              tcg_st_op_func32 emit_st_op)
{
    TCGv_i32 swap = -1;
    tcg_request_block_interrupt_check();
    tcg_gen_req_mo(TCG_MO_LD_ST | TCG_MO_ST_ST);
    memop = tcg_canonicalize_memop(memop, 0, 1);
    if(!TCG_TARGET_HAS_MEMORY_BSWAP && (memop & MO_BSWAP)) {
        swap = tcg_temp_new_i32();
        switch(memop & MO_SIZE) {
            case MO_16:
                tcg_gen_bswap16_i32(swap, val, 0);
                break;
            case MO_32:
                tcg_gen_bswap32_i32(swap, val);
                break;
            default:
                tcg_abort();
                __builtin_unreachable();
        }
        val = swap;
        memop &= ~MO_BSWAP;
    }
    emit_st_op(old_st_opc[memop & MO_SIZE], val, addr, idx);

    if(swap != -1) {
        tcg_temp_free_i32(swap);
    }
}

static inline void tcg_gen_qemu_st_i32(TCGv_i32 val, TCGv_i32 addr, TCGArg idx, TCGMemOp memop)
{
    tcg_gen_qemu_st_i32_common(val, addr, idx, memop, tcg_gen_qemu_st_op);
}

/*
 * Performs an "unsafe" store.
 * This means that it will NOT acquire the hash table lock and
 * NOT invalidate any memory reservation.
 */
static inline void tcg_gen_qemu_st_i32_unsafe(TCGv_i32 val, TCGv_i32 addr, TCGArg idx, TCGMemOp memop)
{
    tcg_gen_qemu_st_i32_common(val, addr, idx, memop, tcg_gen_qemu_st_op_unsafe);
}

static inline void tcg_gen_qemu_ld_i64(TCGv_i64 val, TCGv addr, TCGArg idx, TCGMemOp memop)
{
    TCGMemOp orig_memop;
#if TCG_TARGET_REG_BITS == 32
    if((memop & MO_SIZE) < MO_64) {
        tcg_gen_qemu_ld_i32(TCGV_LOW(val), addr, idx, memop);
        if(memop & MO_SIGN) {
            tcg_gen_sari_i32(TCGV_HIGH(val), TCGV_LOW(val), 31);
        } else {
            tcg_gen_movi_i32(TCGV_HIGH(val), 0);
        }
        return;
    }
#endif
    tcg_gen_req_mo(TCG_MO_LD_LD | TCG_MO_ST_LD);
    memop = tcg_canonicalize_memop(memop, 1, 0);
    orig_memop = memop;
    if(!TCG_TARGET_HAS_MEMORY_BSWAP && (memop & MO_BSWAP)) {
        memop &= ~MO_BSWAP;
        /* The bswap primitive benefits from zero-extended input.  */
        if((memop & MO_SIGN) && (memop & MO_SIZE) < MO_64) {
            memop &= ~MO_SIGN;
        }
    }

    tcg_gen_qemu_ld_op_i64(old_ld_opc[memop & MO_SSIZE], val, addr, idx);

    if((orig_memop ^ memop) & MO_BSWAP) {
        int flags = (orig_memop & MO_SIGN ? TCG_BSWAP_IZ | TCG_BSWAP_OS : TCG_BSWAP_IZ | TCG_BSWAP_OZ);
        switch(orig_memop & MO_SIZE) {
            case MO_16:
                tcg_gen_bswap16_i64(val, val, flags);
                break;
            case MO_32:
                tcg_gen_bswap32_i64(val, val, flags);
                break;
            case MO_64:
                tcg_gen_bswap64_i64(val, val);
                break;
            default:
                tcg_abort();
                __builtin_unreachable();
        }
    }
}

/*
 * Non-atomically loads a 128-bit value from guest memory.
 * Subject to tearing.
 * Assumes that half of the value is located at `guestAddressLow`
 * and the other at `guestAddressLow + sizeof(uint64_t)`.
 * Endianness is taken into account.
 */
static inline void tcg_gen_qemu_ld_i128(TCGv_i128 result, TCGv guestAddressLow, TCGArg memIndex, TCGMemOp memop)
{
    //  Compute the address of the upper 64 bits.
    TCGv guestAddressHigh = tcg_temp_local_new();
    tcg_gen_addi_tl(guestAddressHigh, guestAddressLow, sizeof(uint64_t));

#ifdef TARGET_WORDS_BIGENDIAN
    tcg_gen_qemu_ld_i64(result.low, guestAddressHigh, memIndex, memop);
    tcg_gen_qemu_ld_i64(result.high, guestAddressLow, memIndex, memop);
#else
    tcg_gen_qemu_ld_i64(result.low, guestAddressLow, memIndex, memop);
    tcg_gen_qemu_ld_i64(result.high, guestAddressHigh, memIndex, memop);
#endif

    tcg_temp_free(guestAddressHigh);
}

typedef void (*tcg_st_op_func64)(TCGOpcode opc, TCGv_i64 val, TCGv addr, TCGArg idx);

static inline void tcg_gen_qemu_st_i64_common(TCGv_i64 val, TCGv addr, TCGArg idx, TCGMemOp memop, tcg_st_op_func64 emit_st_op)
{
    TCGv_i64 swap = -1;

    tcg_request_block_interrupt_check();
#if TCG_TARGET_REG_BITS == 32
    if((memop & MO_SIZE) < MO_64) {
        tcg_gen_qemu_st_i32(TCGV_LOW(val), addr, idx, memop);
        return;
    }
#endif

    tcg_gen_req_mo(TCG_MO_LD_ST | TCG_MO_ST_ST);
    memop = tcg_canonicalize_memop(memop, 1, 1);

    if(!TCG_TARGET_HAS_MEMORY_BSWAP && (memop & MO_BSWAP)) {
        swap = tcg_temp_new_i64();
        switch(memop & MO_SIZE) {
            case MO_16:
                tcg_gen_bswap16_i64(swap, val, 0);
                break;
            case MO_32:
                tcg_gen_bswap32_i64(swap, val, 0);
                break;
            case MO_64:
                tcg_gen_bswap64_i64(swap, val);
                break;
            default:
                tcg_abort();
                __builtin_unreachable();
        }
        val = swap;
        memop &= ~MO_BSWAP;
    }

    emit_st_op(old_st_opc[memop & MO_SIZE], val, addr, idx);

    if(swap != -1) {
        tcg_temp_free_i64(swap);
    }
}

static inline void tcg_gen_qemu_st_i64(TCGv_i64 val, TCGv addr, TCGArg idx, TCGMemOp memop)
{
    tcg_gen_qemu_st_i64_common(val, addr, idx, memop, tcg_gen_qemu_st_op);
}

/*
 * Performs an "unsafe" store.
 * This means that it will NOT acquire the hash table lock and
 * NOT invalidate any memory reservation.
 */
static inline void tcg_gen_qemu_st_i64_unsafe(TCGv_i64 val, TCGv addr, TCGArg idx, TCGMemOp memop)
{
    tcg_gen_qemu_st_i64_common(val, addr, idx, memop, tcg_gen_qemu_st_op_unsafe);
}

/*
 * Non-atomically stores a 128-bit value to guest memory.
 * Subject to tearing.
 * Places half of the value located at `guestAddressLow`
 * and the other at `guestAddressLow + sizeof(uint64_t)`.
 * Endianness is taken into account.
 *
 * Performs an "unsafe" store.
 * This means that it will NOT acquire the hash table lock and
 * NOT invalidate any memory reservation.
 */
static inline void tcg_gen_qemu_st_i128_unsafe(TCGv_i128 value, TCGv guestAddressLow, TCGArg memIndex, TCGMemOp memop)
{
    //  Compute the address of the upper 64 bits.
    TCGv guestAddressHigh = tcg_temp_local_new();
    tcg_gen_addi_tl(guestAddressHigh, guestAddressLow, sizeof(uint64_t));

#ifdef TARGET_WORDS_BIGENDIAN
    tcg_gen_qemu_st_i64_unsafe(value.low, guestAddressHigh, memIndex, memop);
    tcg_gen_qemu_st_i64_unsafe(value.high, guestAddressLow, memIndex, memop);
#else
    tcg_gen_qemu_st_i64_unsafe(value.low, guestAddressLow, memIndex, memop);
    tcg_gen_qemu_st_i64_unsafe(value.high, guestAddressHigh, memIndex, memop);
#endif

    tcg_temp_free(guestAddressHigh);
}

static inline void tcg_gen_abs_i32(TCGv_i32 ret, TCGv_i32 a)
{
    TCGv_i32 t = tcg_temp_new_i32();

    tcg_gen_sari_i32(t, a, 31);
    tcg_gen_xor_i32(ret, a, t);
    tcg_gen_sub_i32(ret, ret, t);
    tcg_temp_free_i32(t);
}

static inline void tcg_gen_extrl_i64_i32(TCGv_i32 ret, TCGv_i64 arg)
{
#if TCG_TARGET_REG_BITS == 32
    tcg_gen_mov_i32(ret, TCGV_LOW(arg));
#else
    tcg_gen_mov_i32(ret, (TCGv_i32)arg);
#endif
}

static inline void tcg_gen_extrh_i64_i32(TCGv_i32 ret, TCGv_i64 arg)
{
#if TCG_TARGET_REG_BITS == 32
    tcg_gen_mov_i32(ret, TCGV_HIGH(arg));
#else
    TCGv_i64 t = tcg_temp_new_i64();
    tcg_gen_shri_i64(t, arg, 32);
    tcg_gen_mov_i32(ret, (TCGv_i32)t);
    tcg_temp_free_i64(t);
#endif
}

static inline void tcg_gen_smax_i32(TCGv_i32 ret, TCGv_i32 a, TCGv_i32 b)
{
    tcg_gen_movcond_i32(TCG_COND_LT, ret, a, b, b, a);
}

static inline void tcg_gen_smin_i32(TCGv_i32 ret, TCGv_i32 a, TCGv_i32 b)
{
    tcg_gen_movcond_i32(TCG_COND_LT, ret, a, b, a, b);
}

static inline void tcg_gen_umin_i32(TCGv_i32 ret, TCGv_i32 a, TCGv_i32 b)
{
    tcg_gen_movcond_i32(TCG_COND_LTU, ret, a, b, a, b);
}

static inline void tcg_gen_umax_i32(TCGv_i32 ret, TCGv_i32 a, TCGv_i32 b)
{
    tcg_gen_movcond_i32(TCG_COND_LTU, ret, a, b, b, a);
}

static inline void tcg_gen_clrsb_i32(TCGv_i32 ret, TCGv_i32 arg)
{
//  TODO: Implement clz_i32 to increase simulation performance.
#ifdef TCG_TARGET_HAS_clz_i32
    if(TCG_TARGET_HAS_clz_i32) {
        TCGv_i32 t = tcg_temp_new_i32();
        tcg_gen_sari_i32(t, arg, 31);
        tcg_gen_xor_i32(t, t, arg);
        tcg_gen_clzi_i32(t, t, 32);
        tcg_gen_subi_i32(ret, t, 1);
        tcg_temp_free_i32(t);
        return;
    }
#endif
    int sizemask = 0;
    /* Return value and argument are 32-bit and unsigned.  */
    sizemask |= tcg_gen_sizemask(0, 0, 0);
    sizemask |= tcg_gen_sizemask(1, 0, 0);
    tcg_gen_helper32_1_arg(tcg_helper_clrsb_i32, sizemask, ret, arg);
}

static inline void tcg_gen_clz_i32(TCGv_i32 ret, TCGv_i32 arg1, TCGv_i32 arg2)
{
//  TODO: Implement clz_i32 to increase simulation performance.
#ifdef TCG_TARGET_HAS_clz_i32
    if(TCG_TARGET_HAS_clz_i32) {
        tcg_gen_op3_i32(INDEX_op_clz_i32, ret, arg1, arg2);
        return;
    }
#endif
    int sizemask = 0;
    /* Return value and both arguments are 32-bit and unsigned.  */
    sizemask |= tcg_gen_sizemask(0, 0, 0);
    sizemask |= tcg_gen_sizemask(1, 0, 0);
    sizemask |= tcg_gen_sizemask(2, 0, 0);
    tcg_gen_helper32(tcg_helper_clz_i32, sizemask, ret, arg1, arg2);
}

static inline void tcg_gen_clzi_i32(TCGv_i32 ret, TCGv_i32 arg1, uint32_t arg2)
{
    TCGv_i32 t = tcg_const_i32(arg2);
    tcg_gen_clz_i32(ret, arg1, t);
    tcg_temp_free_i32(t);
}

static inline void tcg_gen_ext_i32(TCGv_i32 ret, TCGv_i32 val, TCGMemOp opc)
{
    switch(opc & MO_SSIZE) {
        case MO_SB:
            tcg_gen_ext8s_i32(ret, val);
            break;
        case MO_UB:
            tcg_gen_ext8u_i32(ret, val);
            break;
        case MO_SW:
            tcg_gen_ext16s_i32(ret, val);
            break;
        case MO_UW:
            tcg_gen_ext16u_i32(ret, val);
            break;
        default:
            tcg_gen_mov_i32(ret, val);
            break;
    }
}

static inline void tcg_gen_ext_i64(TCGv_i64 ret, TCGv_i64 val, TCGMemOp opc)
{
    switch(opc & MO_SSIZE) {
        case MO_SB:
            tcg_gen_ext8s_i64(ret, val);
            break;
        case MO_UB:
            tcg_gen_ext8u_i64(ret, val);
            break;
        case MO_SW:
            tcg_gen_ext16s_i64(ret, val);
            break;
        case MO_UW:
            tcg_gen_ext16u_i64(ret, val);
            break;
        case MO_SL:
            tcg_gen_ext32s_i64(ret, val);
            break;
        case MO_UL:
            tcg_gen_ext32u_i64(ret, val);
            break;
        default:
            tcg_gen_mov_i64(ret, val);
            break;
    }
}

static inline void tcg_gen_deposit_z_i32(TCGv_i32 ret, TCGv_i32 arg, unsigned int ofs, unsigned int len)
{
    tcg_debug_assert(ofs < 32);
    tcg_debug_assert(len > 0);
    tcg_debug_assert(len <= 32);
    tcg_debug_assert(ofs + len <= 32);

    if(ofs + len == 32) {
        tcg_gen_shli_i32(ret, arg, ofs);
    } else if(ofs == 0) {
        tcg_gen_andi_i32(ret, arg, (1u << len) - 1);
    } else if(TCG_TARGET_HAS_deposit_i32 && TCG_TARGET_deposit_i32_valid(ofs, len)) {
        TCGv_i32 zero = tcg_const_i32(0);
        tcg_gen_op5ii_i32(INDEX_op_deposit_i32, ret, zero, arg, ofs, len);
        tcg_temp_free_i32(zero);
    } else {
        /* To help two-operand hosts we prefer to zero-extend first,
           which allows ARG to stay live.  */
        switch(len) {
            case 16:
                if(TCG_TARGET_HAS_ext16u_i32) {
                    tcg_gen_ext16u_i32(ret, arg);
                    tcg_gen_shli_i32(ret, ret, ofs);
                    return;
                }
                break;
            case 8:
                if(TCG_TARGET_HAS_ext8u_i32) {
                    tcg_gen_ext8u_i32(ret, arg);
                    tcg_gen_shli_i32(ret, ret, ofs);
                    return;
                }
                break;
        }
        /* Otherwise prefer zero-extension over AND for code size.  */
        switch(ofs + len) {
            case 16:
                if(TCG_TARGET_HAS_ext16u_i32) {
                    tcg_gen_shli_i32(ret, arg, ofs);
                    tcg_gen_ext16u_i32(ret, ret);
                    return;
                }
                break;
            case 8:
                if(TCG_TARGET_HAS_ext8u_i32) {
                    tcg_gen_shli_i32(ret, arg, ofs);
                    tcg_gen_ext8u_i32(ret, ret);
                    return;
                }
                break;
        }
        tcg_gen_andi_i32(ret, arg, (1u << len) - 1);
        tcg_gen_shli_i32(ret, ret, ofs);
    }
}

static inline void tcg_gen_deposit_z_i64(TCGv_i64 ret, TCGv_i64 arg, unsigned int ofs, unsigned int len)
{
    tcg_debug_assert(ofs < 64);
    tcg_debug_assert(len > 0);
    tcg_debug_assert(len <= 64);
    tcg_debug_assert(ofs + len <= 64);

    if(ofs + len == 64) {
        tcg_gen_shli_i64(ret, arg, ofs);
    } else if(ofs == 0) {
        tcg_gen_andi_i64(ret, arg, (1ull << len) - 1);
    } else if(TCG_TARGET_HAS_deposit_i64 && TCG_TARGET_deposit_i64_valid(ofs, len)) {
        TCGv_i64 zero = tcg_const_i64(0);
        tcg_gen_op5ii_i64(INDEX_op_deposit_i64, ret, zero, arg, ofs, len);
        tcg_temp_free_i32(zero);
    } else {
#if TCG_TARGET_REG_BITS == 32
        if(ofs >= 32) {
            tcg_gen_deposit_z_i32(TCGV_HIGH(ret), TCGV_LOW(arg), ofs - 32, len);
            tcg_gen_movi_i32(TCGV_LOW(ret), 0);
            return;
        }
        if(ofs + len <= 32) {
            tcg_gen_deposit_z_i32(TCGV_LOW(ret), TCGV_LOW(arg), ofs, len);
            tcg_gen_movi_i32(TCGV_HIGH(ret), 0);
            return;
        }
#endif
        /* To help two-operand hosts we prefer to zero-extend first,
           which allows ARG to stay live.  */
        switch(len) {
            case 32:
                if(TCG_TARGET_HAS_ext32u_i64) {
                    tcg_gen_ext32u_i64(ret, arg);
                    tcg_gen_shli_i64(ret, ret, ofs);
                    return;
                }
                break;
            case 16:
                if(TCG_TARGET_HAS_ext16u_i64) {
                    tcg_gen_ext16u_i64(ret, arg);
                    tcg_gen_shli_i64(ret, ret, ofs);
                    return;
                }
                break;
            case 8:
                if(TCG_TARGET_HAS_ext8u_i64) {
                    tcg_gen_ext8u_i64(ret, arg);
                    tcg_gen_shli_i64(ret, ret, ofs);
                    return;
                }
                break;
        }
        /* Otherwise prefer zero-extension over AND for code size.  */
        switch(ofs + len) {
            case 32:
                if(TCG_TARGET_HAS_ext32u_i64) {
                    tcg_gen_shli_i64(ret, arg, ofs);
                    tcg_gen_ext32u_i64(ret, ret);
                    return;
                }
                break;
            case 16:
                if(TCG_TARGET_HAS_ext16u_i64) {
                    tcg_gen_shli_i64(ret, arg, ofs);
                    tcg_gen_ext16u_i64(ret, ret);
                    return;
                }
                break;
            case 8:
                if(TCG_TARGET_HAS_ext8u_i64) {
                    tcg_gen_shli_i64(ret, arg, ofs);
                    tcg_gen_ext8u_i64(ret, ret);
                    return;
                }
                break;
        }
        tcg_gen_andi_i64(ret, arg, (1ull << len) - 1);
        tcg_gen_shli_i64(ret, ret, ofs);
    }
}

static inline void tcg_gen_extr32_i64(TCGv_i64 lo, TCGv_i64 hi, TCGv_i64 arg)
{
    tcg_gen_ext32u_i64(lo, arg);
    tcg_gen_shri_i64(hi, arg, 32);
}

static inline void tcg_gen_extract_i64(TCGv_i64 ret, TCGv_i64 arg, unsigned int ofs, unsigned int len)
{
    tcg_debug_assert(ofs < 64);
    tcg_debug_assert(len > 0);
    tcg_debug_assert(len <= 64);
    tcg_debug_assert(ofs + len <= 64);

    /* Canonicalize certain special cases, even if extract is supported.  */
    if(ofs + len == 64) {
        tcg_gen_shri_i64(ret, arg, 64 - len);
        return;
    }
    if(ofs == 0) {
        tcg_gen_andi_i64(ret, arg, (1ull << len) - 1);
        return;
    }

#if TCG_TARGET_REG_BITS == 32
    /* Look for a 32-bit extract within one of the two words.  */
    if(ofs >= 32) {
        tcg_gen_extract_i32(TCGV_LOW(ret), TCGV_HIGH(arg), ofs - 32, len);
        tcg_gen_movi_i32(TCGV_HIGH(ret), 0);
        return;
    }
    if(ofs + len <= 32) {
        tcg_gen_extract_i32(TCGV_LOW(ret), TCGV_LOW(arg), ofs, len);
        tcg_gen_movi_i32(TCGV_HIGH(ret), 0);
        return;
    }
    /* The field is split across two words.  One double-word
        shift is better than two double-word shifts.  */
    goto do_shift_and;
#endif

//  TODO: Implement extract_i64 to increase simulation performance.
#if defined(TCG_TARGET_HAS_extract_i64)
    if(TCG_TARGET_HAS_extract_i64 && TCG_TARGET_extract_i64_valid(ofs, len)) {
        tcg_gen_op4ii_i64(INDEX_op_extract_i64, ret, arg, ofs, len);
        return;
    }
#endif

    /* Assume that zero-extension, if available, is cheaper than a shift.  */
    switch(ofs + len) {
        case 32:
            if(TCG_TARGET_HAS_ext32u_i64) {
                tcg_gen_ext32u_i64(ret, arg);
                tcg_gen_shri_i64(ret, ret, ofs);
                return;
            }
            break;
        case 16:
            if(TCG_TARGET_HAS_ext16u_i64) {
                tcg_gen_ext16u_i64(ret, arg);
                tcg_gen_shri_i64(ret, ret, ofs);
                return;
            }
            break;
        case 8:
            if(TCG_TARGET_HAS_ext8u_i64) {
                tcg_gen_ext8u_i64(ret, arg);
                tcg_gen_shri_i64(ret, ret, ofs);
                return;
            }
            break;
    }

    /* ??? Ideally we'd know what values are available for immediate AND.
       Assume that 8 bits are available, plus the special cases of 16 and 32,
       so that we get ext8u, ext16u, and ext32u.  */
    switch(len) {
        case 1 ... 8:
        case 16:
        case 32:
#if TCG_TARGET_REG_BITS == 32
        do_shift_and:
#endif
            tcg_gen_shri_i64(ret, arg, ofs);
            tcg_gen_andi_i64(ret, ret, (1ull << len) - 1);
            break;
        default:
            tcg_gen_shli_i64(ret, arg, 64 - len - ofs);
            tcg_gen_shri_i64(ret, ret, 64 - len);
            break;
    }
}

static inline void tcg_gen_sextract_i64(TCGv_i64 ret, TCGv_i64 arg, unsigned int ofs, unsigned int len)
{
    tcg_debug_assert(ofs < 64);
    tcg_debug_assert(len > 0);
    tcg_debug_assert(len <= 64);
    tcg_debug_assert(ofs + len <= 64);

    /* Canonicalize certain special cases, even if sextract is supported.  */
    if(ofs + len == 64) {
        tcg_gen_sari_i64(ret, arg, 64 - len);
        return;
    }
    if(ofs == 0) {
        switch(len) {
            case 32:
                tcg_gen_ext32s_i64(ret, arg);
                return;
            case 16:
                tcg_gen_ext16s_i64(ret, arg);
                return;
            case 8:
                tcg_gen_ext8s_i64(ret, arg);
                return;
        }
    }

#if TCG_TARGET_REG_BITS == 32
    /* Look for a 32-bit extract within one of the two words.  */
    if(ofs >= 32) {
        tcg_gen_sextract_i32(TCGV_LOW(ret), TCGV_HIGH(arg), ofs - 32, len);
    } else if(ofs + len <= 32) {
        tcg_gen_sextract_i32(TCGV_LOW(ret), TCGV_LOW(arg), ofs, len);
    } else if(ofs == 0) {
        tcg_gen_mov_i32(TCGV_LOW(ret), TCGV_LOW(arg));
        tcg_gen_sextract_i32(TCGV_HIGH(ret), TCGV_HIGH(arg), 0, len - 32);
        return;
    } else if(len > 32) {
        TCGv_i32 t = tcg_temp_new_i32();
        /* Extract the bits for the high word normally.  */
        tcg_gen_sextract_i32(t, TCGV_HIGH(arg), ofs + 32, len - 32);
        /* Shift the field down for the low part.  */
        tcg_gen_shri_i64(ret, arg, ofs);
        /* Overwrite the shift into the high part.  */
        tcg_gen_mov_i32(TCGV_HIGH(ret), t);
        tcg_temp_free_i32(t);
        return;
    } else {
        /* Shift the field down for the low part, such that the
            field sits at the MSB.  */
        tcg_gen_shri_i64(ret, arg, ofs + len - 32);
        /* Shift the field down from the MSB, sign extending.  */
        tcg_gen_sari_i32(TCGV_LOW(ret), TCGV_LOW(ret), 32 - len);
    }
    /* Sign-extend the field from 32 bits.  */
    tcg_gen_sari_i32(TCGV_HIGH(ret), TCGV_LOW(ret), 31);
    return;
#endif

//  TODO: Implement sextract_i64 to increase simulation performance.
#if defined(TCG_TARGET_HAS_sextract_i64)
    if(TCG_TARGET_HAS_sextract_i64 && TCG_TARGET_extract_i64_valid(ofs, len)) {
        tcg_gen_op4ii_i64(INDEX_op_sextract_i64, ret, arg, ofs, len);
        return;
    }
#endif

    /* Assume that sign-extension, if available, is cheaper than a shift.  */
    switch(ofs + len) {
        case 32:
            if(TCG_TARGET_HAS_ext32s_i64) {
                tcg_gen_ext32s_i64(ret, arg);
                tcg_gen_sari_i64(ret, ret, ofs);
                return;
            }
            break;
        case 16:
            if(TCG_TARGET_HAS_ext16s_i64) {
                tcg_gen_ext16s_i64(ret, arg);
                tcg_gen_sari_i64(ret, ret, ofs);
                return;
            }
            break;
        case 8:
            if(TCG_TARGET_HAS_ext8s_i64) {
                tcg_gen_ext8s_i64(ret, arg);
                tcg_gen_sari_i64(ret, ret, ofs);
                return;
            }
            break;
    }
    switch(len) {
        case 32:
            if(TCG_TARGET_HAS_ext32s_i64) {
                tcg_gen_shri_i64(ret, arg, ofs);
                tcg_gen_ext32s_i64(ret, ret);
                return;
            }
            break;
        case 16:
            if(TCG_TARGET_HAS_ext16s_i64) {
                tcg_gen_shri_i64(ret, arg, ofs);
                tcg_gen_ext16s_i64(ret, ret);
                return;
            }
            break;
        case 8:
            if(TCG_TARGET_HAS_ext8s_i64) {
                tcg_gen_shri_i64(ret, arg, ofs);
                tcg_gen_ext8s_i64(ret, ret);
                return;
            }
            break;
    }
    tcg_gen_shli_i64(ret, arg, 64 - len - ofs);
    tcg_gen_sari_i64(ret, ret, 64 - len);
}

/*
 * Extract 64 bits from a 128-bit input, ah:al, starting from ofs.
 * Unlike tcg_gen_extract_i64 above, len is fixed at 64.
 */
static inline void tcg_gen_extract2_i64(TCGv_i64 ret, TCGv_i64 al, TCGv_i64 ah, unsigned int ofs)
{
    tcg_debug_assert(ofs <= 64);
    if(ofs == 0) {
        tcg_gen_mov_i64(ret, al);
    } else if(ofs == 64) {
        tcg_gen_mov_i64(ret, ah);
    } else if(al == ah) {
        tcg_gen_rotri_i64(ret, al, ofs);
//  TODO: Implement extract2_i64 to increase simulation performance.
#if defined(TCG_TARGET_HAS_extract2_i64)
    } else if(TCG_TARGET_HAS_extract2_i64) {
        tcg_gen_op4i_i64(INDEX_op_extract2_i64, ret, al, ah, ofs);
#endif
    } else {
        TCGv_i64 t0 = tcg_temp_new_i64();
        tcg_gen_shri_i64(t0, al, ofs);
        tcg_gen_deposit_i64(ret, t0, ah, 64 - ofs, ofs);
        tcg_temp_free_i64(t0);
    }
}

static inline void tcg_gen_clrsb_i64(TCGv_i64 ret, TCGv_i64 arg)
{
//  TODO: Implement clz_i64 and clz_i32 to increase simulation performance.
#if defined(TCG_TARGET_HAS_clz_i64) && defined(TCG_TARGET_HAS_clz_i32)
    if(TCG_TARGET_HAS_clz_i64 || TCG_TARGET_HAS_clz_i32) {
        TCGv_i64 t = tcg_temp_new_i64();
        tcg_gen_sari_i64(t, arg, 63);
        tcg_gen_xor_i64(t, t, arg);
        tcg_gen_clzi_i64(t, t, 64);
        tcg_gen_subi_i64(ret, t, 1);
        tcg_temp_free_i64(t);
        return;
    }
#endif
    int sizemask = 0;
    /* Return value and argument are 64-bit and unsigned.  */
    sizemask |= tcg_gen_sizemask(0, 1, 0);
    sizemask |= tcg_gen_sizemask(1, 1, 0);
    tcg_gen_helper64_1_arg(tcg_helper_clrsb_i64, sizemask, ret, arg);
}

static inline void tcg_gen_clz_i64(TCGv_i64 ret, TCGv_i64 arg1, TCGv_i64 arg2)
{
//  TODO: Implement clz_i64 to increase simulation performance.
#ifdef TCG_TARGET_HAS_clz_i64
    if(TCG_TARGET_HAS_clz_i64) {
        tcg_gen_op3_i64(INDEX_op_clz_i64, ret, arg1, arg2);
        return;
    }
#endif
    int sizemask = 0;
    /* Return value and argument are 64-bit and unsigned.  */
    sizemask |= tcg_gen_sizemask(0, 1, 0);
    sizemask |= tcg_gen_sizemask(1, 1, 0);
    sizemask |= tcg_gen_sizemask(2, 1, 0);
    tcg_gen_helper64(tcg_helper_clz_i64, sizemask, ret, arg1, arg2);
}

static inline void tcg_gen_clzi_i64(TCGv_i64 ret, TCGv_i64 arg1, uint64_t arg2)
{
//  TODO: Implement clz_i32 to increase simulation performance.
#if TCG_TARGET_REG_BITS == 32 && defined(TCG_TARGET_HAS_clz_i32)
    if(TCG_TARGET_HAS_clz_i32 && arg2 <= 0xffffffffu) {
        TCGv_i32 t = tcg_temp_new_i32();
        tcg_gen_clzi_i32(t, TCGV_LOW(arg1), arg2 - 32);
        tcg_gen_addi_i32(t, t, 32);
        tcg_gen_clz_i32(TCGV_LOW(ret), TCGV_HIGH(arg1), t);
        tcg_gen_movi_i32(TCGV_HIGH(ret), 0);
        tcg_temp_free_i32(t);
        return;
    }
#endif
    TCGv_i64 t0 = tcg_const_i64(arg2);
    tcg_gen_clz_i64(ret, arg1, t0);
    tcg_temp_free_i64(t0);
}

#if __llvm__
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wunused-function"
#else
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wunused-function"
#endif

/*
 * Atomically adds `toAdd` to the value located at `address` in host memory.
 * The original value located at `address`, before the addition, is returned in `ret`.
 */
static void tcg_gen_atomic_fetch_add_intrinsic_i32(TCGv_i32 ret, TCGv_ptr hostAddress, TCGv_i32 toAdd)
{
    tcg_gen_atomic_intrinsic_op_i32(INDEX_op_atomic_fetch_add_intrinsic_i32, ret, hostAddress, toAdd);
    tcg_gen_ext32s_i64(ret, ret);
}

/*
 * Atomically adds `toAdd` to the value located at `address` in host memory.
 * The original value located at `address`, before the addition, is returned in `ret`.
 */
static void tcg_gen_atomic_fetch_add_intrinsic_i64(TCGv_i64 ret, TCGv_ptr hostAddress, TCGv_i64 toAdd)
{
    tcg_gen_atomic_intrinsic_op_i64(INDEX_op_atomic_fetch_add_intrinsic_i64, ret, hostAddress, toAdd);
}

/*
 * Atomically compares the expected value in `expected` with the value
 * located at `hostAddress` in memory. If they are equal, `newValue` is written
 * to `hostAddress`. The actual value at `hostAddress` is written to `actual`.
 */
static void tcg_gen_atomic_compare_and_swap_intrinsic_i32(TCGv_i32 actual, TCGv_i32 expected, TCGv_ptr hostAddress,
                                                          TCGv_i32 newValue)
{
    tcg_gen_atomic_intrinsic_op4_i32(INDEX_op_atomic_compare_and_swap_intrinsic_i32, actual, expected, hostAddress, newValue);
}

/*
 * Atomically compares the expected value in `expected` with the value
 * located at `hostAddress` in memory. If they are equal, `newValue` is written
 * to `hostAddress`. The actual value at `hostAddress` is written to `actual`.
 */
static void tcg_gen_atomic_compare_and_swap_intrinsic_i64(TCGv_i64 actual, TCGv_i64 expected, TCGv_ptr hostAddress,
                                                          TCGv_i64 newValue)
{
    tcg_gen_atomic_intrinsic_op4_i64(INDEX_op_atomic_compare_and_swap_intrinsic_i64, actual, expected, hostAddress, newValue);
}

/*
 * Atomically compares the expected value in `expectedHigh:expectedLow` with the value
 * located at `hostAddress` in memory. If they are equal, `newValueHigh:newValueLow` is written
 * to `hostAddress`. The actual value at `hostAddress` is written to `actualHigh:actualLow`.
 */
static void tcg_gen_atomic_compare_and_swap_intrinsic_i128(TCGv_i128 actual, TCGv_i128 expected, TCGv_ptr hostAddress,
                                                           TCGv_i128 newValue)
{
    tcg_gen_atomic_intrinsic_op4_i128(INDEX_op_atomic_compare_and_swap_intrinsic_i128, actual, expected, hostAddress, newValue);
}

#ifdef __llvm__
#pragma clang diagnostic pop
#else
#pragma GCC diagnostic pop
#endif

void tcg_gen_mov_vec(TCGv_vec, TCGv_vec);
void tcg_gen_dup_i32(unsigned vece, TCGv_i32, TCGv_i32);
void tcg_gen_dup_i32_vec(unsigned vece, TCGv_vec, TCGv_i32);
void tcg_gen_dup_i64_vec(unsigned vece, TCGv_vec, TCGv_i64);
void tcg_gen_dup_mem_vec(unsigned vece, TCGv_vec, TCGv_ptr, tcg_target_long);
void tcg_gen_dupi_vec(unsigned vece, TCGv_vec, uint64_t);
void tcg_gen_add_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec b);
void tcg_gen_sub_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec b);
void tcg_gen_mul_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec b);
void tcg_gen_and_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec b);
void tcg_gen_or_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec b);
void tcg_gen_xor_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec b);
void tcg_gen_andc_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec b);
void tcg_gen_orc_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec b);
void tcg_gen_nand_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec b);
void tcg_gen_nor_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec b);
void tcg_gen_eqv_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec b);
void tcg_gen_not_vec(unsigned vece, TCGv_vec r, TCGv_vec a);
void tcg_gen_neg_vec(unsigned vece, TCGv_vec r, TCGv_vec a);
void tcg_gen_abs_vec(unsigned vece, TCGv_vec r, TCGv_vec a);
void tcg_gen_ssadd_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec b);
void tcg_gen_usadd_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec b);
void tcg_gen_sssub_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec b);
void tcg_gen_ussub_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec b);
void tcg_gen_smin_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec b);
void tcg_gen_umin_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec b);
void tcg_gen_smax_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec b);
void tcg_gen_umax_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec b);

void tcg_gen_shli_vec(unsigned vece, TCGv_vec r, TCGv_vec a, int64_t i);
void tcg_gen_shri_vec(unsigned vece, TCGv_vec r, TCGv_vec a, int64_t i);
void tcg_gen_sari_vec(unsigned vece, TCGv_vec r, TCGv_vec a, int64_t i);
void tcg_gen_rotli_vec(unsigned vece, TCGv_vec r, TCGv_vec a, int64_t i);
void tcg_gen_rotri_vec(unsigned vece, TCGv_vec r, TCGv_vec a, int64_t i);

void tcg_gen_shls_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_i32 s);
void tcg_gen_shrs_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_i32 s);
void tcg_gen_sars_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_i32 s);
void tcg_gen_rotls_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_i32 s);

void tcg_gen_shlv_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec s);
void tcg_gen_shrv_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec s);
void tcg_gen_sarv_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec s);
void tcg_gen_rotlv_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec s);
void tcg_gen_rotrv_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec s);

void tcg_gen_cmp_vec(TCGCond cond, unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec b);

void tcg_gen_bitsel_vec(unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec b, TCGv_vec c);
void tcg_gen_cmpsel_vec(TCGCond cond, unsigned vece, TCGv_vec r, TCGv_vec a, TCGv_vec b, TCGv_vec c, TCGv_vec d);

void tcg_gen_ld_vec(TCGv_vec r, TCGv_ptr base, TCGArg offset);
void tcg_gen_st_vec(TCGv_vec r, TCGv_ptr base, TCGArg offset);
void tcg_gen_stl_vec(TCGv_vec r, TCGv_ptr base, TCGArg offset, TCGType t);

#if TCG_TARGET_REG_BITS == 32
#define tcg_gen_add_ptr(R, A, B)  tcg_gen_add_i32(TCGV_PTR_TO_NAT(R), TCGV_PTR_TO_NAT(A), TCGV_PTR_TO_NAT(B))
#define tcg_gen_addi_ptr(R, A, B) tcg_gen_addi_i32(TCGV_PTR_TO_NAT(R), TCGV_PTR_TO_NAT(A), (B))
#define tcg_gen_ext_i32_ptr(R, A) tcg_gen_mov_i32(TCGV_PTR_TO_NAT(R), (A))
#else /* TCG_TARGET_REG_BITS == 32 */
#define tcg_gen_add_ptr(R, A, B)  tcg_gen_add_i64(TCGV_PTR_TO_NAT(R), TCGV_PTR_TO_NAT(A), TCGV_PTR_TO_NAT(B))
#define tcg_gen_addi_ptr(R, A, B) tcg_gen_addi_i64(TCGV_PTR_TO_NAT(R), TCGV_PTR_TO_NAT(A), (B))
#define tcg_gen_ext_i32_ptr(R, A) tcg_gen_ext_i32_i64(TCGV_PTR_TO_NAT(R), (A))
#endif /* TCG_TARGET_REG_BITS != 32 */
