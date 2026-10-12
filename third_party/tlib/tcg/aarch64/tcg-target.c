/*
 * Tiny Code Generator for tlib
 *
 * Copyright (c) 2024 Antmicro
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

#include "tcg-target.h"
#include "tlib/include/infrastructure.h"
#include "tlib/tcg/additional.h"
#include "tlib/tcg/tcg.h"

#if defined(__linux__)
#include <sys/auxv.h>
#include <asm/hwcap.h>
#endif

//  Order registers get picked in
static const int tcg_target_reg_alloc_order[] = {
    TCG_REG_R8,  TCG_REG_R9,  TCG_REG_R10, TCG_REG_R11, TCG_REG_R12, TCG_REG_R13, TCG_REG_R14, TCG_REG_R15, TCG_REG_R16,
    TCG_REG_R17, TCG_REG_R19, TCG_REG_R20, TCG_REG_R0,  TCG_REG_R1,  TCG_REG_R2,  TCG_REG_R3,  TCG_REG_R4,  TCG_REG_R5,
    TCG_REG_R6,  TCG_REG_R7,  TCG_REG_R21, TCG_REG_R22, TCG_REG_R23, TCG_REG_R24, TCG_REG_R25, TCG_REG_R26, TCG_REG_R27,
};

//  Registers that can be used for input functions arguments
static const int tcg_target_call_iarg_regs[8] = {
    TCG_REG_R0, TCG_REG_R1, TCG_REG_R2, TCG_REG_R3, TCG_REG_R4, TCG_REG_R5, TCG_REG_R6, TCG_REG_R7,
};

static inline int tcg_target_get_call_iarg_regs_count(int flags)
{
    return 8;
}

//  Registers that can be used for output functions arguments
static const int tcg_target_call_oarg_regs[8] = {
    TCG_REG_R0, TCG_REG_R1, TCG_REG_R2, TCG_REG_R3, TCG_REG_R4, TCG_REG_R5, TCG_REG_R6, TCG_REG_R7,
};

//  Taken from the arm32 target
enum arm_cond_code_e {
    COND_EQ = 0x0,
    COND_NE = 0x1,
    COND_CS = 0x2, /* Unsigned greater or equal */
    COND_CC = 0x3, /* Unsigned less than */
    COND_MI = 0x4, /* Negative */
    COND_PL = 0x5, /* Zero or greater */
    COND_VS = 0x6, /* Overflow */
    COND_VC = 0x7, /* No overflow */
    COND_HI = 0x8, /* Unsigned greater than */
    COND_LS = 0x9, /* Unsigned less or equal */
    COND_GE = 0xa,
    COND_LT = 0xb,
    COND_GT = 0xc,
    COND_LE = 0xd,
    COND_AL = 0xe,
};

static const uint8_t tcg_cond_to_arm_cond[] = {
    [TCG_COND_EQ] = COND_EQ,
    [TCG_COND_NE] = COND_NE,
    [TCG_COND_LT] = COND_LT,
    [TCG_COND_GE] = COND_GE,
    [TCG_COND_LE] = COND_LE,
    [TCG_COND_GT] = COND_GT,
    /* unsigned */
    [TCG_COND_LTU] = COND_CC,
    [TCG_COND_GEU] = COND_CS,
    [TCG_COND_LEU] = COND_LS,
    [TCG_COND_GTU] = COND_HI,
};

//  Tells tcg what registers can be used for a argument with a certain flag
//  Flag are set in arm_op_defs later in this file, and are used if some
//  registers can not be used with a certain instruction
static int target_parse_constraint(TCGArgConstraint *ct, const char **pct_str)
{
    //  This is just taken from the arm target, might want to refactor to something cleaner
    const char *ct_str;
    ct_str = *pct_str;
    switch(ct_str[0]) {
        case 'r':
            //  Any general purpose register
            ct->ct |= TCG_CT_REG;
            tcg_regset_set(ct->u.regs, (1L << TCG_TARGET_GP_REGS) - 1);
            break;
        case 's':
            //  Registers to be used with qemu_st instructions, needs three extra scratch registers,
            //  so prevent allocating R0, R1, and R2 since they will be overwritten
            ct->ct |= TCG_CT_REG;
            tcg_regset_set(ct->u.regs, (1L << TCG_TARGET_GP_REGS) - 1);
            tcg_regset_reset_reg(ct->u.regs, TCG_REG_R0);
            tcg_regset_reset_reg(ct->u.regs, TCG_REG_R1);
            tcg_regset_reset_reg(ct->u.regs, TCG_REG_R2);
            break;
        case 'l':
            //  Registers to be used with qemu_ld instructions, needs three extra scratch registers,
            //  so prevent allocating R0, R1, and R2 since they will be overwritten
            ct->ct |= TCG_CT_REG;
            tcg_regset_set(ct->u.regs, (1L << TCG_TARGET_GP_REGS) - 1);
            tcg_regset_reset_reg(ct->u.regs, TCG_REG_R0);
            tcg_regset_reset_reg(ct->u.regs, TCG_REG_R1);
            tcg_regset_reset_reg(ct->u.regs, TCG_REG_R2);
            break;
        case 'x':
            //  Registers to be used with atomic operations might need up to two extra scratch register
            //  for the cases where they are implemented with LD/STX Loops
            ct->ct |= TCG_CT_REG;
            tcg_regset_set(ct->u.regs, (1L << TCG_TARGET_GP_REGS) - 1);
            tcg_regset_reset_reg(ct->u.regs, TCG_REG_R0);
            tcg_regset_reset_reg(ct->u.regs, TCG_REG_R1);
            break;
        case 'c':
            //  Registers to be used with atomic compare and swap operations might need up to 4 scratch registers
            //  for the cases where they are implemented with LD/STX Loops
            ct->ct |= TCG_CT_REG;
            tcg_regset_set(ct->u.regs, (1L << TCG_TARGET_GP_REGS) - 1);
            tcg_regset_reset_reg(ct->u.regs, TCG_REG_R0);
            tcg_regset_reset_reg(ct->u.regs, TCG_REG_R1);
            tcg_regset_reset_reg(ct->u.regs, TCG_REG_R2);
            tcg_regset_reset_reg(ct->u.regs, TCG_REG_R3);
            break;
        default:
            tcg_abortf("Constraint %c not implemented", ct_str[0]);
            return -1;
    }
    ct_str++;
    *pct_str = ct_str;

    return 0;
}

//  Tells tcg if a constant `val` can be used with the set arg constraints
//  Can be used to check if tcg should put a value as an immediate or
//  place it in a register before host code is generated,
//  allowing it to generate better code
static inline int tcg_target_const_match(tcg_target_long val, const TCGArgConstraint *arg_ct)
{
    //  TODO: Add constraints and checks for instructions to cut down on generated movi instructions
    int ct;
    ct = arg_ct->ct;
    if(ct & TCG_CT_CONST) {
        //  Register sized constant
        return 1;
    } else {
        return 0;
    }
}

//  Defines to match reloc names and ids from elf standard
#define R_AARCH64_JUMP26   282
#define R_AARCH64_CONDBR19 280

//  Patch the conditional branch instruction, address is 19-bits long
static void reloc_condbr_19(void *code_ptr, tcg_target_long target, int cond)
{
    //  code_ptr should have bits [23, 5] set to target - current PC, 4 to zero and [3, 0] to cond
    //  Its a bit of a hack to set cond this way, but tcg does not seem to offer a better solution
    uint32_t offset = target - ((tcg_target_long)code_ptr);
    //  Offset for cond branch is encoded as offset * 4
    offset = offset >> 2;
    //  Mask out 19 bits from the offset
    *(uint32_t *)code_ptr = (*(uint32_t *)code_ptr & ~0x1FFFFFF) | ((offset & 0x7FFFF) << 5) | cond;
    //  set bit 4 to zero
    *(uint32_t *)code_ptr &= (~(1 << 4));
}

//  Path the unconditional branch instruction, address is 26-bits long
static void reloc_jump26(void *code_ptr, tcg_target_long target)
{
    //  code_ptr should have bits [25, 0] set to target - current PC
    uint32_t offset = target - ((tcg_target_long)code_ptr);
    //  Offset for cond branch is encoded as offset * 4
    offset = offset >> 2;
    *(uint32_t *)code_ptr = (*(uint32_t *)code_ptr & ~0x3FFFFFF) | (offset & 0x3FFFFFF);
}

static void patch_reloc(uint8_t *code_ptr, int type, tcg_target_long value, tcg_target_long addend)
{
    switch(type) {
        case R_AARCH64_JUMP26:
            reloc_jump26(code_ptr, value);
            break;
        case R_AARCH64_CONDBR19:
            reloc_condbr_19(code_ptr, value, addend);
            break;
        default:
            tcg_abortf("patch reloc for type %i not implemented", type);
            break;
    }
}

static inline void tcg_out_br(TCGContext *s, int addr_reg)
{
    tcg_out32(s, 0xd61f0000 | (addr_reg << 5));
}

static inline void tcg_out_b(TCGContext *s, int offset)
{
    offset = offset >> 2;
    tcg_out32(s, 0x14000000 | ((offset & 0x3FFFFFF) << 0));
}

static inline void tcg_out_bl(TCGContext *s, int offset)
{
    offset = offset >> 2;
    tcg_out32(s, 0x94000000 | ((offset & 0x3FFFFFF) << 0));
}

static inline void tcg_out_blr(TCGContext *s, int reg)
{
    tcg_out32(s, 0xd63f0000 | (reg << 5));
}

static inline void tcg_out_ret(TCGContext *s, int reg)
{
    tcg_out32(s, 0xd65f0000 | (reg << 5));
}

static inline void tcg_out_b_cond(TCGContext *s, int cond, tcg_target_long offset)
{
    //  Offset is encoded as offset/4
    offset = offset >> 2;
    //  Offset needs to be masked to 19-bits
    tcg_out32(s, 0x54000000 | ((0x7FFFF & offset) << 5) | (tcg_cond_to_arm_cond[cond] << 0));
}

static inline void tcg_out_compare_branch_non_zero(TCGContext *s, int bits, int reg_value, tcg_target_long offset)
{
    offset = offset >> 2;
    switch(bits) {
        case 32:
            tcg_out32(s, 0x35000000 | ((offset & 0x7FFFF) << 5) | (reg_value << 0));
            break;
        case 64:
            tcg_out32(s, 0xB5000000 | ((offset & 0x7FFFF) << 5) | (reg_value << 0));
            break;
        default:
            tlib_abortf("tcg_out_compare_branch_non_zero called for unsupported width: %d", bits);
            break;
    }
}

static inline void tcg_out_b_noaddr(TCGContext *s)
{
    //  This sets up a unconditional branch to an adress that will be emited later
    //  by either translation block linking or a reloc
    s->code_ptr += 3;
    //  Unconditional branch opcode
    tcg_out8(s, 0b000101 << 2);
}

static inline void tcg_out_b_cond_noaddr(TCGContext *s)
{
    //  This sets up a conditional branch to an adress that will be emited later by a condbr_19 reloc
    s->code_ptr += 3;
    //  Conditional branch opcode
    tcg_out8(s, 0b010101 << 2);
}

//  Value used for addend when nothing extra is needed by the reloc
#define TCG_UNUSED_CONSTANT 31337
static inline void tcg_out_movi64(TCGContext *s, int reg1, tcg_target_long imm);
static inline void tcg_out_goto_label(TCGContext *s, int cond, int label_index)
{
    tcg_debug_assert(label_index < TCG_MAX_LABELS);
    tcg_debug_assert(label_index >= 0);
    TCGLabel *l = &s->labels[label_index];

    if(l->has_value) {
        //  Label has a target address so we just branch to it
        //  Because we don't know if the label was created targeting the rw or rx buffer we have to check
        if(is_ptr_in_rw_buf((void *)l->u.value)) {
            l->u.value = (tcg_target_ulong)rw_ptr_to_rx((void *)l->u.value);
        }
        if(cond == COND_AL) {
            //  Unconditional branch
            tcg_out_movi(s, TCG_TYPE_PTR, TCG_TMP_REG, l->u.value);
            tcg_out_br(s, TCG_TMP_REG);
        } else {
            //  Conditional branch, takes 19-bit PC-relative offset
            int offset = l->u.value - (tcg_target_long)s->code_ptr;
            if((abs(offset) >> 2) > 0x7FFFF) {
                //  Branch target is too far away for a direct branch
                //  So we use a unconditional branch, whith a inverted conditonal to jump over it
                tcg_out_b_cond(s, tcg_invert_cond(cond),
                               24);  //  24 becase we want to jump over 4 moves (for a 64-bit load immediate) and one branch
                tcg_out_movi64(s, TCG_TMP_REG, l->u.value);
                tcg_out_br(s, TCG_TMP_REG);
            } else {
                tcg_out_b_cond(s, cond, offset);
            }
        }
    } else {
        //  Label does not have the address so we need a reloc
        //  reloc names are based on the ones from the elf format
        //  but might not have exacly the same semantics, see patch_reloc() for details
        if(cond == COND_AL) {
            //  Unconditional branch
            tcg_out_reloc(s, s->code_ptr, R_AARCH64_JUMP26, label_index, TCG_UNUSED_CONSTANT);
            tcg_out_b_noaddr(s);
        } else {
            tcg_out_reloc(s, s->code_ptr, R_AARCH64_CONDBR19, label_index,
                          tcg_cond_to_arm_cond[cond]);  //  Reloc needs to set the condition bits correctly
            tcg_out_b_cond_noaddr(s);
        }
    }
}

//  Helper to generate function calls to constant address
static inline void tcg_out_calli(TCGContext *s, tcg_target_ulong addr)
{
    //  The target address can either be one we have generated, or something outside that.
    //  So we need to check if the target has to be translated
    tcg_target_ulong target;
    if(is_ptr_in_rw_buf((const void *)addr)) {
        target = (tcg_target_ulong)rw_ptr_to_rx((void *)addr);
    } else {
        target = addr;
    }
    tcg_out_movi(s, TCG_TYPE_PTR, TCG_TMP_REG, target);
    tcg_out_blr(s, TCG_TMP_REG);
}

//  Emit a memory barrier, a0 contains info about the type of memory barrier requested
static const int MB_SY = 0b1111;  //  Full system barrier
static inline void tcg_out_mb(TCGContext *s, TCGArg barrier_type)
{
    //  Even if a weaker barrier might suffice, for now just always emit the strongest, full system barrier
    tcg_out32(s, 0xd50330bf | (MB_SY << 8));
}

//  Helper function to emit STP, store pair instructions with offset adressing mode (i.e no changing the base register)
static inline void tcg_out_stp(TCGContext *s, int reg1, int reg2, int reg_base, tcg_target_long offset)
{
    //  Offset needs to be a multiple of 8, it gets encoded as offset/8
    tlib_assert(offset % 8 == 0);
    //  Offset is 7 bits
    tcg_out32(s, 0xa9000000 | (((offset / 8) & 0x7f) << 15) | (reg2 << 10) | (reg_base << 5) | (reg1 << 0));
}

//  Helper function to emit LDP, load pair instructions with offset adressing mode (i.e no changing the base register)
static inline void tcg_out_ldp(TCGContext *s, int reg1, int reg2, int reg_base, tcg_target_long offset)
{
    //  Offset needs to be a multiple of 8, it gets encoded as offset/8
    tlib_assert(offset % 8 == 0);
    //  Offset is 7 bits
    tcg_out32(s, 0xa9400000 | (((offset / 8) & 0x7f) << 15) | (reg2 << 10) | (reg_base << 5) | (reg1 << 0));
}

static inline void tcg_out_mov(TCGContext *s, TCGType type, TCGReg dest, TCGReg source)
{
    switch(type) {
        case TCG_TYPE_I32:
            tcg_out32(s, 0x2a000000 | (source << 16) | (0b11111 << 5) | (dest << 0));
            break;
        case TCG_TYPE_I64:
            tcg_out32(s, 0xaa000000 | (source << 16) | (0b11111 << 5) | (dest << 0));
            break;
        default:
            tlib_abortf("tcg_out_mov called for unsupported TCGType: %u", type);
            break;
    }
}

static const int SHIFT_0 = 0b00;
static const int SHIFT_16 = 0b01;
static const int SHIFT_32 = 0b10;
static const int SHIFT_48 = 0b11;

static inline void tcg_out_movi64(TCGContext *s, int reg1, tcg_target_long imm)
{
    tcg_out32(s, 0xd2800000 | (SHIFT_0 << 21) | ((0xffff & imm) << 5) | (reg1 << 0));  //  MOVZ
    imm = imm >> 16;
    tcg_out32(s, 0xf2800000 | (SHIFT_16 << 21) | ((0xffff & imm) << 5) | (reg1 << 0));  //  MOVK
    imm = imm >> 16;
    tcg_out32(s, 0xf2800000 | (SHIFT_32 << 21) | ((0xffff & imm) << 5) | (reg1 << 0));  //  MOVK
    imm = imm >> 16;
    tcg_out32(s, 0xf2800000 | (SHIFT_48 << 21) | ((0xffff & imm) << 5) | (reg1 << 0));  //  MOVK
}

static inline void tcg_out_movi32(TCGContext *s, int reg1, tcg_target_long imm)
{
    tcg_out32(s, 0x52800000 | (SHIFT_0 << 21) | ((0xffff & imm) << 5) | (reg1 << 0));  //  MOVZ
    imm = imm >> 16;
    tcg_out32(s, 0x72800000 | (SHIFT_16 << 21) | ((0xffff & imm) << 5) | (reg1 << 0));  //  MOVK
}

static inline void tcg_out_lsr_reg(TCGContext *s, int bits, int reg_dest, int reg_src, int reg_shift)
{
    //  Emit LSRV instruction
    switch(bits) {
        case 32:
            tcg_out32(s, 0x1AC02400 | reg_shift << 16 | reg_src << 5 | reg_dest << 0);
            break;
        case 64:
            tcg_out32(s, 0x9AC02400 | reg_shift << 16 | reg_src << 5 | reg_dest << 0);
            break;
        default:
            tcg_abortf("lsr_reg for %i bits not implemented", bits);
            break;
    }
}

static inline void tcg_out_lsl_reg(TCGContext *s, int bits, int reg_dest, int reg_src, int reg_shift)
{
    //  Emit LSLV instruction
    switch(bits) {
        case 32:
            tcg_out32(s, 0x1AC02000 | reg_shift << 16 | reg_src << 5 | reg_dest << 0);
            break;
        case 64:
            tcg_out32(s, 0x9AC02000 | reg_shift << 16 | reg_src << 5 | reg_dest << 0);
            break;
        default:
            tcg_abortf("lsl_reg for %i bits not implemented", bits);
            break;
    }
}

static inline void tcg_out_asr_reg(TCGContext *s, int bits, int reg_dest, int reg_src, int reg_shift)
{
    switch(bits) {
        case 32:
            tcg_out32(s, 0x1AC02800 | reg_shift << 16 | reg_src << 5 | reg_dest << 0);
            break;
        case 64:
            tcg_out32(s, 0x9AC02800 | reg_shift << 16 | reg_src << 5 | reg_dest << 0);
            break;
        default:
            tcg_abortf("asr_reg for %i bits not implemented", bits);
            break;
    }
    //  Emit ASRV instruction
}

static inline void tcg_out_lsr_imm(TCGContext *s, int bits, int reg_dest, int reg_src, tcg_target_long shift)
{
    //  Move the offset into a register, and then call the register versions
    tcg_out_movi(s, TCG_TYPE_I64, TCG_TMP_REG, shift);
    tcg_out_lsr_reg(s, bits, reg_dest, reg_src, TCG_TMP_REG);
}

static inline void tcg_out_lsl_imm(TCGContext *s, int bits, int reg_dest, int reg_src, tcg_target_long shift)
{
    //  Move the offset into a register, and then call the register versions
    tcg_out_movi(s, TCG_TYPE_I64, TCG_TMP_REG, shift);
    tcg_out_lsl_reg(s, bits, reg_dest, reg_src, TCG_TMP_REG);
}

static inline void tcg_out_asr_imm(TCGContext *s, int bits, int reg_dest, int reg_src, tcg_target_long shift)
{
    //  Move the offset into a register, and then call the register versions
    tcg_out_movi(s, TCG_TYPE_I64, TCG_TMP_REG, shift);
    tcg_out_asr_reg(s, bits, reg_dest, reg_src, TCG_TMP_REG);
}

//  Extracts #bits bits from the least significant bits of reg_src, sign extends it to 64-bits, and stores the result in reg_dest
static inline void tcg_out_sign_extend(TCGContext *s, int bits, int reg_dest, int reg_src)
{
    tlib_assert(bits <= 64);
    tlib_assert(bits >= 1);
    int bit_position = bits - 1;
    tcg_out32(s, 0x93400000 | (bit_position << 10) | (reg_src << 5) | (reg_dest << 0));
}

//  Fills reg_dest with nbit of data from reg_base + offset_reg
static inline void tcg_out_ld_reg_offset(TCGContext *s, int bits, bool sign_extend, int reg_dest, int reg_base, int offset_reg)
{
    switch(bits) {
        case 8:
            tcg_out32(s, 0x38600800 | (offset_reg << 16) | (0b011 << 13) | (reg_base << 5) | (reg_dest << 0));
            break;
        case 16:
            tcg_out32(s, 0x78600800 | (offset_reg << 16) | (0b011 << 13) | (reg_base << 5) | (reg_dest << 0));
            break;
        case 32:
            tcg_out32(s, 0xb8600800 | (offset_reg << 16) | (0b011 << 13) | (reg_base << 5) | (reg_dest << 0));
            break;
        case 64:
            tcg_out32(s, 0xf8600800 | (offset_reg << 16) | (0b011 << 13) | (reg_base << 5) | (reg_dest << 0));
            break;
        default:
            tcg_abortf("%i bit load not implemented", bits);
            break;
    }
    if(sign_extend) {
        tcg_out_sign_extend(s, bits, reg_dest, reg_dest);
    }
}

static inline void tcg_out_ld_offset(TCGContext *s, int bits, bool sign_extend, int reg_dest, int reg_base,
                                     tcg_target_long offset)
{
    tcg_out_movi64(s, TCG_TMP_REG, offset);
    tcg_out_ld_reg_offset(s, bits, sign_extend, reg_dest, reg_base, TCG_TMP_REG);
}

//  Read a specified TCGType from base + offset into dest
static inline void tcg_out_ld(TCGContext *s, TCGType type, TCGReg dest, TCGReg base, tcg_target_long offset)
{
    tcg_out_movi64(s, TCG_TMP_REG, offset);
    switch(type) {
        case TCG_TYPE_I32:
            tcg_out_ld_reg_offset(s, 32, false, dest, base, TCG_TMP_REG);
            break;
        case TCG_TYPE_I64:
            tcg_out_ld_reg_offset(s, 64, false, dest, base, TCG_TMP_REG);
            break;
        default:
            tcg_abortf("tcg_out_ld called for unsupported TCGType %i", type);
            break;
    }
}

//  Stores n-bits of data from reg_src to reg_base + offset_reg
static inline void tcg_out_st_reg_offset(TCGContext *s, int bits, int reg_src, int reg_base, int offset_reg)
{
    switch(bits) {
        case 8:
            //  The constant is to set some needed flags
            tcg_out32(s, 0x38200800 | (offset_reg << 16) | (0b011 << 13) | (reg_base << 5) | (reg_src << 0));
            break;
        case 16:
            tcg_out32(s, 0x78200800 | (offset_reg << 16) | (0b011 << 13) | (reg_base << 5) | (reg_src << 0));
            break;
        case 32:
            tcg_out32(s, 0xb8200800 | (offset_reg << 16) | (0b011 << 13) | (reg_base << 5) | (reg_src << 0));
            break;
        case 64:
            tcg_out32(s, 0xf8200800 | (offset_reg << 16) | (0b011 << 13) | (reg_base << 5) | (reg_src << 0));
            break;
        default:
            tcg_abortf("st %i bits wide not implemented", bits);
            break;
    }
}

static inline void tcg_out_st_offset(TCGContext *s, int bits, int reg_src, int reg_base, tcg_target_long offset)
{
    tcg_out_movi64(s, TCG_TMP_REG, offset);
    tcg_out_st_reg_offset(s, bits, reg_src, reg_base, TCG_TMP_REG);
}

static inline void tcg_out_st(TCGContext *s, TCGType type, TCGReg arg, TCGReg arg1, tcg_target_long offset)
{
    //  Write the content of arg to the address in arg1 + offset
    //  For offsets that fit in 9-bits this could be one instruction, future optimization work

    //  Move offset into designated tmp reg
    tcg_out_movi64(s, TCG_TMP_REG, offset);
    switch(type) {
        case TCG_TYPE_I32:
            tcg_out_st_reg_offset(s, 32, arg, arg1, TCG_TMP_REG);
            break;
        case TCG_TYPE_I64:
            tcg_out_st_reg_offset(s, 64, arg, arg1, TCG_TMP_REG);
            break;
        default:
            tcg_abortf("tcg_out_st called for unsupported TCGType %i", type);
            break;
    }
}

static const int SHIFT_LSL = 0b00;
static const int SHIFT_LSR = 0b01;
static const int SHIFT_ASR = 0b10;

static inline void tcg_out_subs_shift_reg(TCGContext *s, int bits, int reg_dest, int reg1, int reg2, int shift_type,
                                          tcg_target_long shift_amount)
{
    //  Shift amount is 6-bits
    shift_amount = shift_amount & 0x3F;
    switch(bits) {
        case 32:
            tcg_out32(s, 0x6b000000 | (shift_type << 22) | (reg2 << 16) | (shift_amount << 10) | (reg1 << 5) | (reg_dest << 0));
            break;
        case 64:
            tcg_out32(s, 0xeb000000 | (shift_type << 22) | (reg2 << 16) | (shift_amount << 10) | (reg1 << 5) | (reg_dest << 0));
            break;
        default:
            tcg_abortf("tcg_out_subs_shift_reg called with unsupported %i bits", bits);
    }
}

static inline void tcg_out_cmp_shift_reg(TCGContext *s, int bits, int reg1, int reg2, int shift_type,
                                         tcg_target_long shift_amount)
{
    //  Sets flags based on the result of reg1 - reg2, where reg2 is shifted in a manner and by an amount specified in the
    //  arguments. This is actually an alias for SUBS (shifted register) with the dest set to the zero register
    tcg_out_subs_shift_reg(s, bits, TCG_REG_RZR, reg1, reg2, shift_type, shift_amount);
}

static inline void tcg_out_cmp(TCGContext *s, int bits, int reg1, int reg2)
{
    //  cmp is just cmp shifted reg with shift set to zero
    tcg_out_cmp_shift_reg(s, bits, reg1, reg2, SHIFT_LSL, 0);
}

static inline void tcg_out_cmpi(TCGContext *s, int bits, int reg, tcg_target_long imm)
{
    //  Mov immediate into a register
    tcg_out_movi(s, 0, TCG_TMP_REG, imm);
    tcg_out_cmp(s, bits, reg, TCG_TMP_REG);
}

static inline void tcg_out_movi(TCGContext *s, TCGType type, TCGReg ret, tcg_target_long arg)
{
    //  TODO: Optimize this in case the immidiate fits in less than 64-bits
    tcg_out_movi64(s, ret, arg);
}

//  Helper function to emit SUB, with immediate
static inline void tcg_out_subi(TCGContext *s, int reg1, int reg2, tcg_target_long imm)
{
    tcg_out32(s, 0xd1000000 | (imm << 10) | (reg2 << 5) | (reg1 << 0));
}

//  Helper function to emit ADD, with immediate
static inline void tcg_out_addi(TCGContext *s, int reg1, int reg2, tcg_target_long imm)
{
    tcg_out32(s, 0x91000000 | (imm << 10) | (reg2 << 5) | (reg1 << 0));
}

static inline void tcg_out_add_shift_reg(TCGContext *s, int bits, bool set_flags, int reg_dest, int reg1, int reg2,
                                         int shift_type, tcg_target_long shift_amount)
{
    //  Shift amount is 6-bits
    switch(bits) {
        case 32:
            tcg_out32(s, 0x0b000000 | (set_flags << 29) | (shift_type << 22) | (reg2 << 16) | ((shift_amount & 0x3F) << 10) |
                             (reg1 << 5) | (reg_dest << 0));
            break;
        case 64:
            tcg_out32(s, 0x8b000000 | (set_flags << 29) | (shift_type << 22) | (reg2 << 16) | ((shift_amount & 0x3F) << 10) |
                             (reg1 << 5) | (reg_dest << 0));
            break;
        default:
            tcg_abortf("add_shift_reg called with unsupported bit width: %i", bits);
    }
}

//  Addc is add with carry
static inline void tcg_out_addc_reg(TCGContext *s, int bits, int reg_dest, int reg1, int reg2)
{
    switch(bits) {
        case 32:
            tcg_out32(s, 0x1a000000 | (reg2 << 16) | (reg1 << 5) | (reg_dest << 0));
            break;
        case 64:
            tcg_out32(s, 0x9a000000 | (reg2 << 16) | (reg1 << 5) | (reg_dest << 0));
            break;
        default:
            tcg_abortf("tcg_out_addc called with unsupported bit width: %i", bits);
    }
}

static inline void tcg_out_add_reg(TCGContext *s, int bits, int reg_dest, int reg1, int reg2)
{
    tcg_out_add_shift_reg(s, bits, false, reg_dest, reg1, reg2, SHIFT_LSL, 0);
}

static inline void tcg_out_add_imm(TCGContext *s, int bits, int reg_dest, int reg_in, tcg_target_long imm)
{
    switch(bits) {
        case 32:
            tcg_out_movi32(s, TCG_TMP_REG, imm);
            break;
        case 64:
            tcg_out_movi64(s, TCG_TMP_REG, imm);
            break;
        default:
            tcg_abortf("add_imm for %i bits not implemented", bits);
    }
    tcg_out_add_reg(s, bits, reg_dest, reg_in, TCG_TMP_REG);
}

//  Add with carry with immediate
static inline void tcg_out_addc_imm(TCGContext *s, int bits, int reg_dest, int reg_in, tcg_target_long imm)
{
    switch(bits) {
        case 32:
            tcg_out_movi32(s, TCG_TMP_REG, imm);
            break;
        case 64:
            tcg_out_movi64(s, TCG_TMP_REG, imm);
            break;
        default:
            tcg_abortf("addc_imm for %i bits not implemented", bits);
    }
    tcg_out_addc_reg(s, bits, reg_dest, reg_in, TCG_TMP_REG);
}

//  Performs addition using two registers to hold each operand and the result.
//  This can for example be used to do a 64-bit addition using only 32-bit registers.
//  That case is not relevant for the aarch64 target, but it is sometimes used by
//  the frontend to set overflow flags or similar.
static inline void tcg_out_add2(TCGContext *s, int bits, int reg_dest_low, int reg_dest_high, int reg_src1_low, int reg_src1_high,
                                int reg_src2_low, int reg_src2_high)
{
    switch(bits) {
        case 32:
            if(reg_dest_low == reg_src1_high || reg_dest_low == reg_src2_high) {
                //  Destination is also used as input, so first write the result to a temp register to preserve
                //  the value for the second computation
                tcg_out_add_shift_reg(s, 32, true, TCG_TMP_REG, reg_src1_low, reg_src2_low, SHIFT_LSL, 0);
                tcg_out_addc_reg(s, 32, reg_dest_high, reg_src1_high, reg_src2_high);
                //  Finally move the temp result to the correct destination
                tcg_out_mov(s, TCG_TYPE_I32, reg_dest_low, TCG_TMP_REG);
            } else {
                tcg_out_add_shift_reg(s, 32, true, reg_dest_low, reg_src1_low, reg_src2_low, SHIFT_LSL, 0);
                tcg_out_addc_reg(s, 32, reg_dest_high, reg_src1_high, reg_src2_high);
            }
            break;
        default:
            tcg_abortf("add2 only support 32 bits for now");
    }
}

static inline void tcg_out_sub_shift_reg(TCGContext *s, int bits, int reg_dest, int reg1, int reg2, int shift_type,
                                         tcg_target_long shift_amount)
{
    //  Shift amount is 6-bits
    switch(bits) {
        case 32:
            tcg_out32(s, 0x4b000000 | (shift_type << 22) | (reg2 << 16) | ((shift_amount & 0x3F) << 10) | (reg1 << 5) |
                             (reg_dest << 0));
            break;
        case 64:
            tcg_out32(s, 0xcb000000 | (shift_type << 22) | (reg2 << 16) | ((shift_amount & 0x3F) << 10) | (reg1 << 5) |
                             (reg_dest << 0));
            break;
        default:
            tcg_abortf("sub_shift_reg called with unsupported bit width: %i", bits);
    }
}

static inline void tcg_out_sub_reg(TCGContext *s, int bits, int reg_dest, int reg1, int reg2)
{
    tcg_out_sub_shift_reg(s, bits, reg_dest, reg1, reg2, SHIFT_LSL, 0);
}

static inline void tcg_out_sub_imm(TCGContext *s, int bits, int reg_dest, int reg_in, tcg_target_long imm)
{
    switch(bits) {
        case 32:
            tcg_out_movi32(s, TCG_TMP_REG, imm);
            break;
        case 64:
            tcg_out_movi64(s, TCG_TMP_REG, imm);
            break;
        default:
            tcg_abortf("sub_imm for %i bits not implemented", bits);
    }
    tcg_out_sub_reg(s, bits, reg_dest, reg_in, TCG_TMP_REG);
}

//  Helper for aarch64 MADD instructions, computes reg_dest = (reg_prod1 * reg_prod2) + reg_add
static inline void tcg_out_mul_add(TCGContext *s, int bits, int reg_dest, int reg_prod1, int reg_prod2, int reg_add)
{
    switch(bits) {
        case 32:
            tcg_out32(s, 0x1B000000 | (reg_prod2 << 16) | (reg_add << 10) | (reg_prod1 << 5) | (reg_dest << 0));
            break;
        case 64:
            tcg_out32(s, 0x9B000000 | (reg_prod2 << 16) | (reg_add << 10) | (reg_prod1 << 5) | (reg_dest << 0));
            break;
        default:
            tcg_abortf("mul_add for %i bits not implemented", bits);
            break;
    }
}

static inline void tcg_out_mul_reg(TCGContext *s, int bits, int reg_dest, int reg_prod1, int reg_prod2)
{
    //  Register multiplication is just MADD with the zero register as the addend
    tcg_out_mul_add(s, bits, reg_dest, reg_prod1, reg_prod2, TCG_REG_RZR);
}

static inline void tcg_out_mul_imm(TCGContext *s, int bits, int reg_dest, int reg_in, tcg_target_long imm)
{
    switch(bits) {
        case 32:
            tcg_out_movi32(s, TCG_TMP_REG, imm);
            break;
        case 64:
            tcg_out_movi64(s, TCG_TMP_REG, imm);
            break;
        default:
            tcg_abortf("mul_imm for %i bits not implemented", bits);
    }
    tcg_out_mul_reg(s, bits, reg_dest, reg_in, TCG_TMP_REG);
}

static inline void tcg_out_and_reg(TCGContext *s, int bits, int reg_dest, int reg1, int reg2)
{
    switch(bits) {
        case 32:
            tcg_out32(s, 0x0a000000 | (reg2 << 16) | (reg1 << 5) | (reg_dest << 0));
            break;
        case 64:
            tcg_out32(s, 0x8a000000 | (reg2 << 16) | (reg1 << 5) | (reg_dest << 0));
            break;
        default:
            tcg_abortf("and_reg called with unsupported bit width: %i", bits);
    }
}

static inline void tcg_out_and_imm(TCGContext *s, int bits, int reg_dest, int reg_in, tcg_target_long imm)
{
    switch(bits) {
        case 32:
            tcg_out_movi32(s, TCG_TMP_REG, imm);
            break;
        case 64:
            tcg_out_movi64(s, TCG_TMP_REG, imm);
            break;
        default:
            tcg_abortf("and_imm for %i bits not implemented", bits);
    }
    tcg_out_and_reg(s, bits, reg_dest, reg_in, TCG_TMP_REG);
}

static inline void tcg_out_or_reg(TCGContext *s, int bits, int reg_dest, int reg1, int reg2)
{
    switch(bits) {
        case 32:
            tcg_out32(s, 0x2a000000 | (reg2 << 16) | (reg1 << 5) | (reg_dest << 0));
            break;
        case 64:
            tcg_out32(s, 0xaa000000 | (reg2 << 16) | (reg1 << 5) | (reg_dest << 0));
            break;
        default:
            tcg_abortf("or_reg called with unsupported bit width: %i", bits);
    }
}

static inline void tcg_out_or_imm(TCGContext *s, int bits, int reg_dest, int reg_in, tcg_target_long imm)
{
    switch(bits) {
        case 32:
            tcg_out_movi32(s, TCG_TMP_REG, imm);
            break;
        case 64:
            tcg_out_movi64(s, TCG_TMP_REG, imm);
            break;
        default:
            tcg_abortf("or_imm for %i bits not implemented", bits);
    }
    tcg_out_or_reg(s, bits, reg_dest, reg_in, TCG_TMP_REG);
}

static inline void tcg_out_xor_shift_reg(TCGContext *s, int bits, int reg_dest, int reg1, int reg2, int shift_type,
                                         tcg_target_long shift_amount)
{
    //  Shift amount is 6-bits
    shift_amount = shift_amount & 0x3F;
    switch(bits) {
        case 32:
            tcg_out32(s, 0x4a000000 | (shift_type << 22) | (reg2 << 16) | (shift_amount << 10) | (reg1 << 5) | (reg_dest << 0));
            break;
        case 64:
            tcg_out32(s, 0xca000000 | (shift_type << 22) | (reg2 << 16) | (shift_amount << 10) | (reg1 << 5) | (reg_dest << 0));
            break;
        default:
            tcg_abortf("xor_shift_reg called with unsupported bit width: %i", bits);
    }
}

static inline void tcg_out_xor_reg(TCGContext *s, int bits, int reg_dest, int reg1, int reg2)
{
    //  This is just xor_shift_reg with shift amount set to 0
    tcg_out_xor_shift_reg(s, bits, reg_dest, reg1, reg2, SHIFT_LSL, 0);
}

static inline void tcg_out_xor_imm(TCGContext *s, int bits, int reg_dest, int reg_in, tcg_target_long imm)
{
    switch(bits) {
        case 32:
            tcg_out_movi32(s, TCG_TMP_REG, imm);
            break;
        case 64:
            tcg_out_movi64(s, TCG_TMP_REG, imm);
            break;
        default:
            tcg_abortf("xor_imm for %i bits not implemented", bits);
    }
    tcg_out_xor_reg(s, bits, reg_dest, reg_in, TCG_TMP_REG);
}

static inline void tcg_out_bswap(TCGContext *s, int bits, int reg_dest, int reg_src)
{
    switch(bits) {
        case 8:
            //  Noop, so just move it to dest
            tcg_out_mov(s, TCG_TYPE_I64, reg_dest, reg_src);
            break;
        case 16:
            tcg_out32(s, 0x5ac00400 | (reg_src << 5) | (reg_dest << 0));
            break;
        case 32:
            tcg_out32(s, 0x5ac00800 | (reg_src << 5) | (reg_dest << 0));
            break;
        case 64:
            tcg_out32(s, 0xdac00c00 | (reg_src << 5) | (reg_dest << 0));
            break;
        default:
            tcg_abortf("tcg_out_bswap call with unsupported %i bits", bits);
    }
}

//  Helper for the CSINC instruction (Conditional Select Increment)
//  Set reg_dest = reg_src_true if cond == true, reg_dest = reg_src_false + 1 otherwise
static inline void tcg_out_csinc(TCGContext *s, int bits, int reg_dest, int reg_src_true, int reg_src_false, tcg_target_long cond)
{
    switch(bits) {
        case 32:
            tcg_out32(s, 0x1a800400 | (reg_src_false << 16) | (tcg_cond_to_arm_cond[cond] << 12) | (reg_src_true << 5) |
                             (reg_dest << 0));
            break;
        case 64:
            tcg_out32(s, 0x9a800400 | (reg_src_false << 16) | (tcg_cond_to_arm_cond[cond] << 12) | (reg_src_true << 5) |
                             (reg_dest << 0));
            break;
        default:
            tcg_abortf("tcg_out_cinc called with unsupported %i bits", bits);
            break;
    }
}

//  set reg_dest to 1 if reg_cmp1 <cond> reg_cmp2 is True, otherwise set it to zero
static inline void tcg_out_setcond_reg(TCGContext *s, int bits, int reg_dest, int reg_cmp1, int reg_cmp2, tcg_target_long cond)
{
    //  Compare and set flags
    tcg_out_cmp(s, bits, reg_cmp1, reg_cmp2);
    //  the CSINC instruction with both source registers set to the zero register does the inverse of this operation
    //  so by inverting the condition we get the right result
    tcg_out_csinc(s, bits, reg_dest, TCG_REG_RZR, TCG_REG_RZR, tcg_invert_cond(cond));
}

//  Same as above but with an immidiate
static inline void tcg_out_setcond_imm(TCGContext *s, int bits, int reg_dest, int reg_cmp, tcg_target_long imm,
                                       tcg_target_long cond)
{
    tcg_out_cmpi(s, bits, reg_cmp, imm);
    tcg_out_csinc(s, bits, reg_dest, TCG_REG_RZR, TCG_REG_RZR, tcg_invert_cond(cond));
}

static inline bool has_host_atomics()
{
#if defined(__linux__)
    return !!(getauxval(AT_HWCAP) & HWCAP_ATOMICS);
#elif defined(__APPLE__)
    //  All arm macs support host atomics
    return true;
#else
#error "Only arm64 linux and macOS are supported"
#endif
}

//  Emits a load exclusive (LDXR) instruction, optionally with aquire memory ordering (LDAXR)
static inline void tcg_out_load_exclusive(TCGContext *s, int bits, int reg_dest, int reg_addr, bool acquire)
{
    switch(bits) {
        case 32:
            tcg_out32(s, 0x885F7C00 | (acquire << 15) | (reg_addr << 5) | (reg_dest << 0));
            break;
        case 64:
            tcg_out32(s, 0xC85F7C00 | (acquire << 15) | (reg_addr << 5) | (reg_dest << 0));
            break;
        default:
            tcg_abortf("tcg_out_load_exclusive called with unsupported %i bits", bits);
    }
}

//  Emits a store excluse (STXR) instruction, optionally with release memory ordering (STLXR)
static inline void tcg_out_store_exclusive(TCGContext *s, int bits, int reg_result, int reg_val, int reg_addr, bool release)
{
    switch(bits) {
        case 32:
            tcg_out32(s, 0x88007C00 | (reg_result << 16) | (release << 15) | (reg_addr << 5) | (reg_val << 0));
            break;
        case 64:
            tcg_out32(s, 0xC8007C00 | (reg_result << 16) | (release << 15) | (reg_addr << 5) | (reg_val << 0));
            break;
        default:
            tcg_abortf("tcg_out_store_exclusive called with unsupported %i bits", bits);
    }
}

static inline TCGType width_to_tcg_type(int bits)
{
    switch(bits) {
        case 32:
            return TCG_TYPE_I32;
        case 64:
            return TCG_TYPE_I64;
        default:
            tlib_abortf("width_to_tcg_type called on unsupported width %i", bits);
            __builtin_unreachable();
    }
}

//  Emits a compare and swap instruction, optionally with aquire and/or release memory ordering
static inline void tcg_out_compare_and_swap(TCGContext *s, int bits, int reg_expected, int reg_val, int reg_addr, bool acquire,
                                            bool release)
{
    switch(bits) {
        case 32:
            tcg_out32(s,
                      0x88a07c00 | (acquire << 22) | (reg_expected << 16) | (release << 15) | (reg_addr) << 5 | (reg_val << 0));
            break;
        case 64:
            tcg_out32(s,
                      0xc8a07c00 | (acquire << 22) | (reg_expected << 16) | (release << 15) | (reg_addr) << 5 | (reg_val << 0));
            break;
        default:
            tcg_abortf("tcg_out_compare_and_swap called with unsupported %i bits", bits);
            break;
    }
}

static inline void tcg_out_emulated_atomic_compare_and_swap(TCGContext *s, int bits, int reg_expected, int reg_val, int reg_addr)
{
    //  Move the address, value, and the original expected value into scratch registers
    tcg_out_mov(s, width_to_tcg_type(bits), TCG_REG_R0, reg_val);
    tcg_out_mov(s, TCG_TYPE_I64, TCG_REG_R1, reg_addr);
    tcg_out_mov(s, width_to_tcg_type(bits), TCG_REG_R2, reg_expected);
    //  Address for the start of the loop
    uint8_t *start = s->code_ptr;
    //  Load the value from memory
    tcg_out_load_exclusive(s, bits, reg_expected, TCG_REG_R1, /* acquire: */ true);
    //  compare the read (reg_expected) and original expected value (TCG_REG_R2)
    tcg_out_cmp(s, bits, TCG_REG_R2, reg_expected);
    //  If the values don't match branch over to the end.
    //  Offset of 12, since each instruction is 4 bytes, and we need to branch over this instruction, the store, and the loop
    //  branch
    tcg_out_b_cond(s, TCG_COND_NE, 12);
    //  Attempt to write back the value
    tcg_out_store_exclusive(s, bits, TCG_REG_R3, TCG_REG_R0, TCG_REG_R1, /* release: */ true);
    //  If it fails, retry until it works
    //  Arm forward progress guarantee that we can't livelock here
    tcg_out_compare_branch_non_zero(s, bits, TCG_REG_R3, start - s->code_ptr);
}

//  Emits a compare and swap with seq-const memory ordering
static inline void tcg_out_atomic_compare_and_swap(TCGContext *s, int bits, int reg_expected, int reg_val, int reg_addr)
{
    if(has_host_atomics()) {
        tcg_out_compare_and_swap(s, bits, reg_expected, reg_val, reg_addr, /* acquire: */ true, /* release: */ true);
    } else {
        //  If the core does not have native compare and swap a fallback using a Load/Store Exclusive loop is needed
        tcg_out_emulated_atomic_compare_and_swap(s, bits, reg_expected, reg_val, reg_addr);
    }
}

static inline void tcg_out_emulated_atomic_fetch_and_add(TCGContext *s, int bits, int reg_dest, int reg_addr, int reg_val)
{
    //  Move the address and value into a scratch register, in case reg_dest overlaps with any of them
    tcg_out_mov(s, TCG_TYPE_I64, TCG_REG_R1, reg_addr);
    tcg_out_mov(s, width_to_tcg_type(bits), TCG_REG_R0, reg_val);
    //  Address for the start of the loop
    uint8_t *start = s->code_ptr;
    //  Load the original value from memory
    tcg_out_load_exclusive(s, bits, reg_dest, TCG_REG_R1, true);
    //  Perform the add
    tcg_out_add_reg(s, bits, TCG_TMP_REG, reg_dest, TCG_REG_R0);
    //  Attempt to write back the value, putting the resulting status value in TCG_TMP_REG
    tcg_out_store_exclusive(s, bits, TCG_TMP_REG, TCG_TMP_REG, TCG_REG_R1, true);
    //  If it fails, retry until it works
    //  Arm forward progress guarantee that we can't livelock here
    tcg_out_compare_branch_non_zero(s, bits, TCG_TMP_REG, start - s->code_ptr);
}

//  Emits a fetch and add instruction (LDADD), optionally with aquire and/or release memory ordering
static inline void tcg_out_fetch_and_add(TCGContext *s, int bits, int reg_dest, int reg_addr, int reg_val, bool acquire,
                                         bool release)
{
    switch(bits) {
        case 32:
            tcg_out32(s, 0xb8200000 | (acquire << 23) | (release << 22) | (reg_val << 16) | (reg_addr << 5) | (reg_dest << 0));
            break;
        case 64:
            tcg_out32(s, 0xf8200000 | (acquire << 23) | (release << 22) | (reg_val << 16) | (reg_addr << 5) | (reg_dest << 0));
            break;
        default:
            tcg_abortf("tcg_out_fetch_and_add called with unsupported %i bits", bits);
            break;
    }
}

//  Emits a fetch and add with seq-const memory ordering
static inline void tcg_out_atomic_fetch_and_add(TCGContext *s, int bits, int reg_dest, int reg_addr, int reg_val)
{
    if(has_host_atomics()) {
        tcg_out_fetch_and_add(s, bits, reg_dest, reg_addr, reg_val, /* acquire : */ true, /* release: */ true);
    } else {
        //  If the core does not have native fetch and add a fallback using a Load/Store Exclusive loop is needed
        tcg_out_emulated_atomic_fetch_and_add(s, bits, reg_dest, reg_addr, reg_val);
    }
}

//  qemu_st/ld loads and stores to and from GUEST addresses, not host ones
static inline void tcg_out_qemu_st(TCGContext *s, int bits, int reg_data, int reg_addr, int mem_index)
{
    //  Store #bits number of bits from reg_data into the guest address in reg_addr
    //
    //  Clobbers R0, R1, and R2 since we need extra scratch registers

    //  Put the address in r0, data in R1, and mem_index in R2
    tcg_out_mov(s, TCG_TYPE_I64, TCG_REG_R0, reg_addr);
    tcg_out_sign_extend(s, bits, TCG_REG_R1, reg_data);
    tcg_out_movi(s, TCG_TYPE_I64, TCG_REG_R2, mem_index);

    //  Call tcg function to execute the store and populate tlb
    switch(bits) {
        case 8:
            tcg_out_calli(s, (tcg_target_long)tcg->stb);
            break;
        case 16:
            tcg_out_calli(s, (tcg_target_long)tcg->stw);
            break;
        case 32:
            tcg_out_calli(s, (tcg_target_long)tcg->stl);
            break;
        case 64:
            tcg_out_calli(s, (tcg_target_long)tcg->stq);
            break;
        default:
            tcg_abortf("tcg_out_qemu_st called with incorrect #%i bits as argument", bits);
    }
    //  Emit a memory barrier to make sure the store is observable on other cores before continueing
    //  Needed to make sure atomic instructions using this works correctly
    //  ST-LD barrier because we only need the store to be visible to future loads
    tcg_out_mb(s, TCG_MO_ST_LD);
}

static inline void tcg_out_qemu_ld(TCGContext *s, int bits, bool sign_extend, int reg_data, int reg_addr, int mem_index)
{
    //  Put the address in r0, and mem_index in r1
    tcg_out_mov(s, TCG_TYPE_I64, TCG_REG_R0, reg_addr);
    tcg_out_movi(s, TCG_TYPE_I64, TCG_REG_R1, mem_index);

    switch(bits) {
        case 8:
            tcg_out_calli(s, (tcg_target_long)tcg->ldb);
            break;
        case 16:
            tcg_out_calli(s, (tcg_target_long)tcg->ldw);
            break;
        case 32:
            tcg_out_calli(s, (tcg_target_long)tcg->ldl);
            break;
        case 64:
            tcg_out_calli(s, (tcg_target_long)tcg->ldq);
            break;
        default:
            tcg_abortf("tcg_out_qemu_ld called with incorrect #%i bits as argument", bits);
    }

    //  The tcg_ld functions put the data in r0
    if(sign_extend) {
        tcg_out_sign_extend(s, bits, reg_data, TCG_REG_R0);
    } else {
        tcg_out_mov(s, TCG_TYPE_I64, reg_data, TCG_REG_R0);
    }
}

//  Variable used to store the return address. Used in INDEX_op_exit_tb
static uint8_t *tb_ret_addr;

static inline void tcg_out_op(TCGContext *s, TCGOpcode opc, const TCGArg *args, const int *const_args)
{
    //  args contains actual arguments, const_args holds flags indicating if the argument is a constant
    //  const_args[n] == true => args[n] is a constant, otherwise it is a register
    switch(opc) {
        case INDEX_op_exit_tb:
            tcg_out_movi(s, TCG_TYPE_PTR, TCG_REG_R0, args[0]);  //  Put return value in output register
            tcg_out_movi(s, TCG_TYPE_PTR, TCG_TMP_REG, (tcg_target_ulong)tb_ret_addr);
            tcg_out_br(s, TCG_TMP_REG);
            break;
        case INDEX_op_goto_tb:
            if(s->tb_jmp_offset) {
                //  Direct branches only support 26-bit offset
                uint32_t offset = s->code_ptr - s->code_buf;
                tlib_assert(offset < (1 << 26));

                //  Direct jump
                //  The branch target will be written later during tb linking
                s->tb_jmp_offset[args[0]] = offset;
                tcg_out_b_noaddr(s);

            } else {
                //  Indirect jump
                tcg_abortf("op_goto_tb indirect jump not implemented");
            }
            s->tb_next_offset[args[0]] = s->code_ptr - s->code_buf;
            break;
        case INDEX_op_call:
            if(const_args[0]) {
                //  Target function adress is an immediate
                tcg_out_calli(s, args[0]);
            } else {
                //  Register target we can just branch and link directly
                tcg_out_blr(s, args[0]);
            }
            break;
        case INDEX_op_jmp:
            tcg_abortf("op_jmp not implemented");
            break;
        case INDEX_op_br:
            //  Unconditional branch to label
            tcg_out_goto_label(s, COND_AL, args[0]);
            break;
        case INDEX_op_mb:
            tcg_out_mb(s, args[0]);
            break;
        case INDEX_op_ld8u_i32:
            tcg_out_ld_offset(s, 8, false, args[0], args[1], args[2]);
            break;
        case INDEX_op_ld8u_i64:
            tcg_out_ld_offset(s, 8, false, args[0], args[1], args[2]);
            break;
        case INDEX_op_ld8s_i32:
            tcg_out_ld_offset(s, 8, true, args[0], args[1], args[2]);
            break;
        case INDEX_op_ld8s_i64:
            tcg_out_ld_offset(s, 8, true, args[0], args[1], args[2]);
            break;
        case INDEX_op_ld16s_i32:
            tcg_out_ld_offset(s, 16, true, args[0], args[1], args[2]);
            break;
        case INDEX_op_ld16s_i64:
            tcg_out_ld_offset(s, 16, true, args[0], args[1], args[2]);
            break;
        case INDEX_op_ld16u_i32:
            tcg_out_ld_offset(s, 16, false, args[0], args[1], args[2]);
            break;
        case INDEX_op_ld16u_i64:
            tcg_out_ld_offset(s, 16, false, args[0], args[1], args[2]);
            break;
        case INDEX_op_ld_i32:
            tcg_out_ld(s, TCG_TYPE_I32, args[0], args[1], args[2]);
            break;
        case INDEX_op_ld32u_i64:
            tcg_out_ld_offset(s, 32, false, args[0], args[1], args[2]);
            break;
        case INDEX_op_ld32s_i64:
            tcg_out_ld_offset(s, 32, true, args[0], args[1], args[2]);
            break;
        case INDEX_op_ld_i64:
            tcg_out_ld_offset(s, 64, false, args[0], args[1], args[2]);
            break;
        case INDEX_op_st8_i32:
            tcg_out_st_offset(s, 8, args[0], args[1], args[2]);
            break;
        case INDEX_op_st8_i64:
            tcg_out_st_offset(s, 8, args[0], args[1], args[2]);
            break;
        case INDEX_op_st16_i32:
            tcg_out_st_offset(s, 16, args[0], args[1], args[2]);
            break;
        case INDEX_op_st16_i64:
            tcg_out_st_offset(s, 16, args[0], args[1], args[2]);
            break;
        case INDEX_op_st_i32:
            tcg_out_st(s, TCG_TYPE_I32, args[0], args[1], args[2]);
            break;
        case INDEX_op_st32_i64:
            tcg_out_st_offset(s, 32, args[0], args[1], args[2]);
            break;
        case INDEX_op_st_i64:
            tcg_out_st(s, TCG_TYPE_I64, args[0], args[1], args[2]);
            break;
        case INDEX_op_mov_i32:
            tcg_abortf("op_mov_i32 not implemented");
            break;
        case INDEX_op_movi_i32:
            tcg_abortf("op_movi_i32 not implemented");
            break;
        case INDEX_op_add_i32:
            if(const_args[2]) {
                //  Add with immediate
                tcg_out_add_imm(s, 32, args[0], args[1], args[2]);
            } else {
                //  Add with registers
                tcg_out_add_reg(s, 32, args[0], args[1], args[2]);
            }
            break;
        case INDEX_op_add_i64:
            if(const_args[2]) {
                //  Add with immediate
                tcg_out_add_imm(s, 64, args[0], args[1], args[2]);
            } else {
                //  Add with registers
                tcg_out_add_reg(s, 64, args[0], args[1], args[2]);
            }
            break;
        case INDEX_op_sub_i32:
            if(const_args[2]) {
                //  Add with immediate
                tcg_out_sub_imm(s, 32, args[0], args[1], args[2]);
            } else {
                //  Add with registers
                tcg_out_sub_reg(s, 32, args[0], args[1], args[2]);
            }
            break;
        case INDEX_op_sub_i64:
            if(const_args[2]) {
                //  Add with immediate
                tcg_out_sub_imm(s, 64, args[0], args[1], args[2]);
            } else {
                //  Add with registers
                tcg_out_sub_reg(s, 64, args[0], args[1], args[2]);
            }
            break;
        case INDEX_op_and_i32:
            if(const_args[2]) {
                //  And with immediate
                tcg_out_and_imm(s, 32, args[0], args[1], args[2]);
            } else {
                //  And with registers
                tcg_out_and_reg(s, 32, args[0], args[1], args[2]);
            }
            break;
        case INDEX_op_and_i64:
            if(const_args[2]) {
                //  And with immediate
                tcg_out_and_imm(s, 64, args[0], args[1], args[2]);
            } else {
                //  And with registers
                tcg_out_and_reg(s, 64, args[0], args[1], args[2]);
            }
            break;
        case INDEX_op_andc_i32:
            tcg_abortf("op_andc_i32 not implemented");
            break;
        case INDEX_op_or_i32:
            if(const_args[2]) {
                //  Or with immediate
                tcg_out_or_imm(s, 32, args[0], args[1], args[2]);
            } else {
                //  Or with registers
                tcg_out_or_reg(s, 32, args[0], args[1], args[2]);
            }
            break;
        case INDEX_op_or_i64:
            if(const_args[2]) {
                //  Or with immediate
                tcg_out_or_imm(s, 64, args[0], args[1], args[2]);
            } else {
                //  Or with registers
                tcg_out_or_reg(s, 64, args[0], args[1], args[2]);
            }
            break;
        case INDEX_op_xor_i32:
            if(const_args[2]) {
                //  Xor with immediate
                tcg_out_xor_imm(s, 32, args[0], args[1], args[2]);
            } else {
                //  Xor with registers
                tcg_out_xor_reg(s, 32, args[0], args[1], args[2]);
            }
            break;
        case INDEX_op_xor_i64:
            if(const_args[2]) {
                //  Xor with immediate
                tcg_out_xor_imm(s, 64, args[0], args[1], args[2]);
            } else {
                //  Xor with registers
                tcg_out_xor_reg(s, 64, args[0], args[1], args[2]);
            }
            break;
        case INDEX_op_neg_i32:
            tcg_abortf("op_neg_i32 not implemented");
            break;
        case INDEX_op_not_i32:
            tcg_abortf("op_not_i32 not implemented");
            break;
        case INDEX_op_mul_i32:
            if(const_args[2]) {
                //  Mul with immediate
                tcg_out_mul_imm(s, 32, args[0], args[1], args[2]);
            } else {
                //  Mul with registers
                tcg_out_mul_reg(s, 32, args[0], args[1], args[2]);
            }
            break;
        case INDEX_op_mul_i64:
            if(const_args[2]) {
                //  Mul with immediate
                tcg_out_mul_imm(s, 64, args[0], args[1], args[2]);
            } else {
                //  Mul with registers
                tcg_out_mul_reg(s, 64, args[0], args[1], args[2]);
            }
            break;
        case INDEX_op_mulu2_i32:
            tcg_abortf("op_mulu2_i32 not implemented");
            break;
        case INDEX_op_muls2_i32:
            tcg_abortf("op_muls2_i32 not implemented");
            break;
        case INDEX_op_shl_i32:
            if(const_args[2]) {
                tcg_out_lsl_imm(s, 32, args[0], args[1], args[2]);
            } else {
                tcg_out_lsl_reg(s, 32, args[0], args[1], args[2]);
            }
            break;
        case INDEX_op_shl_i64:
            if(const_args[2]) {
                tcg_out_lsl_imm(s, 64, args[0], args[1], args[2]);
            } else {
                tcg_out_lsl_reg(s, 64, args[0], args[1], args[2]);
            }
            break;
        case INDEX_op_shr_i32:
            if(const_args[2]) {
                tcg_out_lsr_imm(s, 32, args[0], args[1], args[2]);
            } else {
                tcg_out_lsr_reg(s, 32, args[0], args[1], args[2]);
            }
            break;
        case INDEX_op_shr_i64:
            if(const_args[2]) {
                tcg_out_lsr_imm(s, 64, args[0], args[1], args[2]);
            } else {
                tcg_out_lsr_reg(s, 64, args[0], args[1], args[2]);
            }
            break;
        case INDEX_op_sar_i32:
            if(const_args[2]) {
                tcg_out_asr_imm(s, 32, args[0], args[1], args[2]);
            } else {
                tcg_out_asr_reg(s, 32, args[0], args[1], args[2]);
            }
            break;
        case INDEX_op_sar_i64:
            if(const_args[2]) {
                tcg_out_asr_imm(s, 64, args[0], args[1], args[2]);
            } else {
                tcg_out_asr_reg(s, 64, args[0], args[1], args[2]);
            }
            break;
        case INDEX_op_rotr_i32:
            tcg_abortf("op_rotr_i32 not implemented");
            break;
        case INDEX_op_rotl_i32:
            tcg_abortf("op_rotl_i32 not implemented");
            break;
        case INDEX_op_brcond_i32:
            if(const_args[1]) {
                //  Second arg is an immediate
                tcg_out_cmpi(s, 32, args[0], args[1]);
            } else {
                //  Second arg is a register
                tcg_out_cmp(s, 32, args[0], args[1]);
            }
            tcg_out_goto_label(s, args[2], args[3]);
            break;
        case INDEX_op_brcond_i64:
            if(const_args[1]) {
                //  Second arg is an immediate
                tcg_out_cmpi(s, 64, args[0], args[1]);
            } else {
                //  Second arg is a register
                tcg_out_cmp(s, 64, args[0], args[1]);
            }
            tcg_out_goto_label(s, args[2], args[3]);
            break;
        case INDEX_op_brcond2_i32:
            tcg_abortf("op_brcond2_i32 not implemented");
            break;
        case INDEX_op_setcond_i32:
            if(const_args[2]) {
                //  Third arg is an immediate
                tcg_out_setcond_imm(s, 32, args[0], args[1], args[2], args[3]);
            } else {
                //  Third arg is a registers
                tcg_out_setcond_reg(s, 32, args[0], args[1], args[2], args[3]);
            }
            break;
        case INDEX_op_setcond_i64:
            if(const_args[2]) {
                //  Third arg is an immediate
                tcg_out_setcond_imm(s, 64, args[0], args[1], args[2], args[3]);
            } else {
                //  Third arg is a registers
                tcg_out_setcond_reg(s, 64, args[0], args[1], args[2], args[3]);
            }
            break;
        case INDEX_op_setcond2_i32:
            tcg_abortf("op_setcond2_i32 not implemented");
            break;
        case INDEX_op_qemu_ld8u:
            tcg_out_qemu_ld(s, 8, false, args[0], args[1], args[2]);
            break;
        case INDEX_op_qemu_ld8s:
            tcg_out_qemu_ld(s, 8, true, args[0], args[1], args[2]);
            break;
        case INDEX_op_qemu_ld16u:
            tcg_out_qemu_ld(s, 16, false, args[0], args[1], args[2]);
            break;
        case INDEX_op_qemu_ld16s:
            tcg_out_qemu_ld(s, 16, true, args[0], args[1], args[2]);
            break;
        case INDEX_op_qemu_ld32:
            tcg_out_qemu_ld(s, 32, false, args[0], args[1], args[2]);
            break;
        case INDEX_op_qemu_ld32u:
            tcg_out_qemu_ld(s, 32, false, args[0], args[1], args[2]);
            break;
        case INDEX_op_qemu_ld32s:
            tcg_out_qemu_ld(s, 32, true, args[0], args[1], args[2]);
            break;
        case INDEX_op_qemu_ld64:
            tcg_out_qemu_ld(s, 64, false, args[0], args[1], args[2]);
            break;
        case INDEX_op_qemu_st8:
            tcg_out_qemu_st(s, 8, args[0], args[1], args[2]);
            break;
        case INDEX_op_qemu_st16:
            tcg_out_qemu_st(s, 16, args[0], args[1], args[2]);
            break;
        case INDEX_op_qemu_st32:
            tcg_out_qemu_st(s, 32, args[0], args[1], args[2]);
            break;
        case INDEX_op_qemu_st64:
            tcg_out_qemu_st(s, 64, args[0], args[1], args[2]);
            break;
        case INDEX_op_bswap16_i32:
            tcg_abortf("op_bswap16_i32 not implemented");
            break;
        case INDEX_op_bswap32_i32:
            tcg_abortf("op_bswap32_i32 not implemented");
            break;
        case INDEX_op_ext8s_i32:
            tcg_abortf("op_ext8s_i32 not implemented");
            break;
        case INDEX_op_ext16s_i32:
            tcg_abortf("op_ext16s_i32 not implemented");
            break;
        case INDEX_op_ext16u_i32:
            tcg_abortf("op_ext16u_i32 not implemented");
            break;
        case INDEX_op_add2_i32:
            //  The arm target assumes the arguments here are always registers, never constants.
            //  Making a similar assumption but guarding with an assert just in case
            if(unlikely(const_args[4])) {
                tlib_abortf("op_add2_i32 does not support constant arguments");
            }
            if(unlikely(const_args[5])) {
                tlib_abortf("op_add2_i32 does not support constant arguments");
            }
            tcg_out_add2(s, 32, args[0], args[1], args[2], args[3], args[4], args[5]);
            break;
        case INDEX_op_sub2_i32:
            tcg_abortf("op_sub2_i32 not implemented");
            break;
        case INDEX_op_atomic_compare_and_swap_intrinsic_i32:
            //  TCG does not behave correctly if the value in an input register is overwritten, so we use a temporary for the
            //  expected value which will be overwritten in the case of a failure
            tcg_out_mov(s, TCG_TYPE_I32, TCG_TMP_REG, args[1]);
            tcg_out_atomic_compare_and_swap(s, 32, TCG_TMP_REG, args[3], args[2]);
            tcg_out_mov(s, TCG_TYPE_I32, args[0], TCG_TMP_REG);
            break;
        case INDEX_op_atomic_compare_and_swap_intrinsic_i64:
            //  TCG does not behave correctly if the value in an input register is overwritten, so we use a temporary for the
            //  expected value which will be overwritten in the case of a failure
            tcg_out_mov(s, TCG_TYPE_I64, TCG_TMP_REG, args[1]);
            tcg_out_atomic_compare_and_swap(s, 64, TCG_TMP_REG, args[3], args[2]);
            tcg_out_mov(s, TCG_TYPE_I64, args[0], TCG_TMP_REG);
            break;
        case INDEX_op_atomic_compare_and_swap_intrinsic_i128:
            tcg_abortf("op_atomic_compare_and_swap_instrinsic_128 not implemented");
            break;
        case INDEX_op_atomic_fetch_add_intrinsic_i32:
            tcg_out_atomic_fetch_and_add(s, 32, args[0], args[1], args[2]);
            break;
        case INDEX_op_atomic_fetch_add_intrinsic_i64:
            tcg_out_atomic_fetch_and_add(s, 64, args[0], args[1], args[2]);
            break;
        default:
            tcg_abortf("TCGOpcode %u not implemented", opc);
    }
}

//  Used by parse_constraints and const_match to help tcg select apropriate registers
//  and to determine what immediates fit in instructions
//
//  List taken from i386 target, not all of these are supported or even enabled currently
//  but even having an incorrect definition here gives more helpful error messages
//  instead of just segfaulting
static const TCGTargetOpDef arm_op_defs[] = {
    { INDEX_op_exit_tb, {} },
    { INDEX_op_goto_tb, {} },
    { INDEX_op_call, { "ri" } },
    { INDEX_op_jmp, { "ri" } },
    { INDEX_op_br, {} },
    { INDEX_op_mb, {} },

    { INDEX_op_mov_i32, { "r", "r" } },
    { INDEX_op_movi_i32, { "r" } },
    { INDEX_op_mov_i64, { "r", "r" } },
    { INDEX_op_movi_i64, { "r" } },

    { INDEX_op_ld8u_i32, { "r", "r" } },
    { INDEX_op_ld8u_i64, { "r", "r" } },
    { INDEX_op_ld8s_i32, { "r", "r" } },
    { INDEX_op_ld8s_i64, { "r", "r" } },

    { INDEX_op_ld16u_i32, { "r", "r" } },
    { INDEX_op_ld16u_i64, { "r", "r" } },
    { INDEX_op_ld16s_i32, { "r", "r" } },
    { INDEX_op_ld16s_i64, { "r", "r" } },

    { INDEX_op_ld32u_i64, { "r", "r" } },
    { INDEX_op_ld32s_i64, { "r", "r" } },
    { INDEX_op_ld_i32, { "r", "r" } },
    { INDEX_op_ld_i64, { "r", "r" } },

    { INDEX_op_ld32u_i64, { "r", "r" } },
    { INDEX_op_ld_i64, { "r", "r" } },

    { INDEX_op_st8_i32, { "r", "r" } },
    { INDEX_op_st8_i64, { "r", "r" } },
    { INDEX_op_st16_i32, { "r", "r" } },
    { INDEX_op_st16_i64, { "r", "r" } },
    { INDEX_op_st32_i64, { "r", "r" } },
    { INDEX_op_st_i32, { "r", "r" } },
    { INDEX_op_st_i64, { "r", "r" } },

    { INDEX_op_add_i32, { "r", "r", "r" } },
    { INDEX_op_add_i64, { "r", "r", "r" } },
    { INDEX_op_sub_i32, { "r", "r", "r" } },
    { INDEX_op_sub_i64, { "r", "r", "r" } },
    { INDEX_op_mul_i32, { "r", "r", "r" } },
    { INDEX_op_mul_i64, { "r", "r", "r" } },

    { INDEX_op_and_i32, { "r", "r", "r" } },
    { INDEX_op_and_i64, { "r", "r", "r" } },
    { INDEX_op_or_i32, { "r", "r", "r" } },
    { INDEX_op_or_i64, { "r", "r", "r" } },
    { INDEX_op_xor_i32, { "r", "r", "r" } },
    { INDEX_op_xor_i64, { "r", "r", "r" } },

    { INDEX_op_shl_i32, { "r", "r", "r" } },
    { INDEX_op_shl_i64, { "r", "r", "r" } },
    { INDEX_op_shr_i32, { "r", "r", "r" } },
    { INDEX_op_shr_i64, { "r", "r", "r" } },
    { INDEX_op_sar_i32, { "r", "r", "r" } },
    { INDEX_op_sar_i64, { "r", "r", "r" } },
    { INDEX_op_rotl_i32, { "r", "r", "r" } },
    { INDEX_op_rotl_i64, { "r", "r", "r" } },
    { INDEX_op_rotr_i32, { "r", "r", "r" } },
    { INDEX_op_rotr_i64, { "r", "r", "r" } },

    { INDEX_op_brcond_i32, { "r", "ri" } },
    { INDEX_op_brcond_i64, { "r", "r" } },

    { INDEX_op_bswap16_i32, { "r", "r" } },
    { INDEX_op_bswap16_i64, { "r", "r" } },
    { INDEX_op_bswap32_i32, { "r", "r" } },
    { INDEX_op_bswap32_i64, { "r", "r" } },
    { INDEX_op_bswap64_i64, { "r", "r" } },

    { INDEX_op_neg_i32, { "r", "r" } },
    { INDEX_op_neg_i64, { "r", "r" } },

    { INDEX_op_not_i32, { "r", "r" } },
    { INDEX_op_not_i64, { "r", "r" } },

    { INDEX_op_ext8s_i32, { "r", "r" } },
    { INDEX_op_ext16s_i32, { "r", "r" } },
    { INDEX_op_ext8u_i32, { "r", "r" } },
    { INDEX_op_ext16u_i32, { "r", "r" } },

    { INDEX_op_ext8s_i64, { "r", "r" } },
    { INDEX_op_ext16s_i64, { "r", "r" } },
    { INDEX_op_ext32s_i64, { "r", "r" } },
    { INDEX_op_ext8u_i64, { "r", "r" } },
    { INDEX_op_ext16u_i64, { "r", "r" } },
    { INDEX_op_ext32u_i64, { "r", "r" } },

    { INDEX_op_setcond_i32, { "r", "r", "r" } },
    { INDEX_op_setcond_i64, { "r", "r", "r" } },
    { INDEX_op_movcond_i32, { "r", "r", "r", "r", "r" } },
    { INDEX_op_movcond_i64, { "r", "r", "r", "r", "r" } },

    { INDEX_op_deposit_i32, { "r", "r", "r" } },
    { INDEX_op_deposit_i64, { "r", "r", "r" } },

    { INDEX_op_extract_i32, { "r", "r" } },

    { INDEX_op_qemu_ld8u, { "r", "l" } },
    { INDEX_op_qemu_ld8s, { "r", "l" } },
    { INDEX_op_qemu_ld16u, { "r", "l" } },
    { INDEX_op_qemu_ld16s, { "r", "l" } },
    { INDEX_op_qemu_ld32, { "r", "l" } },
    { INDEX_op_qemu_ld32u, { "r", "l" } },
    { INDEX_op_qemu_ld32s, { "r", "l" } },
    { INDEX_op_qemu_ld64, { "r", "l" } },

    { INDEX_op_qemu_st8, { "s", "s" } },
    { INDEX_op_qemu_st16, { "s", "s" } },
    { INDEX_op_qemu_st32, { "s", "s" } },
    { INDEX_op_qemu_st64, { "s", "s" } },

    { INDEX_op_add2_i32, { "r", "r", "r", "r", "r", "r" } },
    { INDEX_op_sub2_i32, { "r", "r", "r", "r", "r", "r" } },
    { INDEX_op_brcond2_i32, { "r", "r", "r", "r" } },
    { INDEX_op_setcond2_i32, { "r", "r", "r", "r", "r" } },

    { INDEX_op_atomic_fetch_add_intrinsic_i32, { "x", "x", "x" } },
    { INDEX_op_atomic_fetch_add_intrinsic_i64, { "x", "x", "x" } },

    { INDEX_op_atomic_compare_and_swap_intrinsic_i32, { "c", "c", "c", "c" } },
    { INDEX_op_atomic_compare_and_swap_intrinsic_i64, { "c", "c", "c", "c" } },
    { INDEX_op_atomic_compare_and_swap_intrinsic_i128, { "r", "r", "r", "r", "r", "r", "r" } },

    //  { -1 } indicates the end of the list
    { -1 },
};

static void tcg_target_init(TCGContext *s)
{
    /* fail safe */ if((1 << CPU_TLB_ENTRY_BITS) != sizeof_CPUTLBEntry) { tcg_abort(); }

    tcg_regset_set32(tcg_target_available_regs[TCG_TYPE_I32], 0, 0xffff);
    tcg_regset_set32(tcg_target_available_regs[TCG_TYPE_I64], 0, 0xffff);
    //  Set wich registers can get clobbered by a function call
    tcg_regset_set32(tcg_target_call_clobber_regs, 0,
                     //  Parameter and results registers
                     (1 << TCG_REG_R0) | (1 << TCG_REG_R1) | (1 << TCG_REG_R2) | (1 << TCG_REG_R3) | (1 << TCG_REG_R4) |
                         (1 << TCG_REG_R5) | (1 << TCG_REG_R6) | (1 << TCG_REG_R7) |
                         //  Indirect result registers
                         (1 << TCG_REG_R8) |
                         //  Corruptable registers
                         (1 << TCG_REG_R9) | (1 << TCG_REG_R10) | (1 << TCG_REG_R11) | (1 << TCG_REG_R12) | (1 << TCG_REG_R13) |
                         (1 << TCG_REG_R14) | (1 << TCG_REG_R15) |
                         //  Intra-procedure-call corruptable registers
                         (1 << TCG_REG_R16) | (1 << TCG_REG_R17) | (1 << TCG_REG_R18) |
                         //  Frame pointer
                         (1 << TCG_REG_R29) |
                         //  Link register
                         (1 << TCG_REG_R30));

    //  Setup reserved registers
    tcg_regset_clear(s->reserved_regs);
    tcg_regset_set_reg(s->reserved_regs, TCG_REG_CALL_STACK);  //  SP
    tcg_regset_set_reg(s->reserved_regs, TCG_TMP_REG);         //  R28 to use as an intermediate
    tcg_regset_set_reg(s->reserved_regs, TCG_REG_PC);          //  PC
    tcg_regset_set_reg(s->reserved_regs, TCG_REG_R18);         //  R18 is reserved for OS use on macOS

    tcg_add_target_add_op_defs(arm_op_defs);
    //  temp_buf_offset is set by init_tcg(), here used to find the start of the tcg frame
    tcg_set_frame(s, TCG_AREG0, temp_buf_offset, CPU_TEMP_BUF_NLONGS * sizeof(long));
}

static void tcg_target_qemu_prologue(TCGContext *s)
{
    //  Prologue

    //  The ARMv8 calling conventions allow compilers to put temporary values
    //  up to 16 bytes BELOW the current stack pointer, the so called red zone
    //  Normally the compiler will not do this for external calls, but the jump to generated
    //  code is not treated as an external function call so there might be data we must preserve there
    //  To do this we first move the SP 16 bytes down before doing anything else
    tcg_out_subi(s, TCG_REG_SP, TCG_REG_SP, 16);
    //  Make space on the stack for callee saved registers
    tcg_out_subi(s, TCG_REG_SP, TCG_REG_SP, 80);
    //  aarch64 calling conventions needs us to save R19-R30
    tcg_out_stp(s, TCG_REG_R19, TCG_REG_R20, TCG_REG_SP, 0);
    tcg_out_stp(s, TCG_REG_R21, TCG_REG_R22, TCG_REG_SP, 16);
    tcg_out_stp(s, TCG_REG_R23, TCG_REG_R24, TCG_REG_SP, 32);
    tcg_out_stp(s, TCG_REG_R25, TCG_REG_R26, TCG_REG_SP, 48);
    tcg_out_stp(s, TCG_REG_R27, TCG_REG_R28, TCG_REG_SP, 64);
    tcg_out_stp(s, TCG_REG_R29, TCG_REG_R30, TCG_REG_SP, 80);

    //  Branch to code
    tcg_out_mov(s, TCG_TYPE_PTR, TCG_AREG0, tcg_target_call_iarg_regs[0]);
    tcg_out_br(s, tcg_target_call_iarg_regs[1]);

    //  Epilogue
    tb_ret_addr = rw_ptr_to_rx(s->code_ptr);
    //  Load all the registers we saved above to restore system state
    tcg_out_ldp(s, TCG_REG_R29, TCG_REG_R30, TCG_REG_SP, 80);
    tcg_out_ldp(s, TCG_REG_R27, TCG_REG_R28, TCG_REG_SP, 64);
    tcg_out_ldp(s, TCG_REG_R25, TCG_REG_R26, TCG_REG_SP, 48);
    tcg_out_ldp(s, TCG_REG_R23, TCG_REG_R24, TCG_REG_SP, 32);
    tcg_out_ldp(s, TCG_REG_R21, TCG_REG_R22, TCG_REG_SP, 16);
    tcg_out_ldp(s, TCG_REG_R19, TCG_REG_R20, TCG_REG_SP, 0);
    tcg_out_addi(s, TCG_REG_SP, TCG_REG_SP, 80);  //  Pop the stack
    //  Restore the SP above the possible temporary values
    tcg_out_addi(s, TCG_REG_SP, TCG_REG_SP, 16);
    //  Return based on the link address
    tcg_out_ret(s, TCG_REG_R30);
}
