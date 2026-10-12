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
#pragma once

//  33 total registers, 31 general purpose
#define TCG_TARGET_NB_REGS 33
#define TCG_TARGET_GP_REGS 31

//  Taken from arm32 target, needed for i386 guest
#define TCG_TARGET_DEFAULT_MO 0

typedef enum {
    TCG_REG_R0 = 0,
    TCG_REG_R1,
    TCG_REG_R2,
    TCG_REG_R3,
    TCG_REG_R4,
    TCG_REG_R5,
    TCG_REG_R6,
    TCG_REG_R7,
    TCG_REG_R8,  //  XR - Indirect result register
    TCG_REG_R9,
    TCG_REG_R10,
    TCG_REG_R11,
    TCG_REG_R12,
    TCG_REG_R13,
    TCG_REG_R14,
    TCG_REG_R15,
    TCG_REG_R16,  //  IP0
    TCG_REG_R17,  //  IP1
    TCG_REG_R18,
    TCG_REG_R19,
    TCG_REG_R20,
    TCG_REG_R21,
    TCG_REG_R22,
    TCG_REG_R23,
    TCG_REG_R24,
    TCG_REG_R25,
    TCG_REG_R26,
    TCG_REG_R27,
    TCG_REG_R28,
    TCG_REG_R29,  //  FP
    TCG_REG_R30,  //  LR
    //  Not general purpose
    TCG_REG_SP,  //  SP - note: has to be 16-bit aligned
    TCG_REG_PC,  //  PC
} TCGReg;
//  Register to use as an internal intermediate
#define TCG_TMP_REG TCG_REG_R28

//  The zero registers is actually just the stack pointer id
#define TCG_REG_RZR TCG_REG_SP

//  Function call generation constants
#define TCG_REG_CALL_STACK           TCG_REG_SP
#define TCG_TARGET_STACK_ALIGN       16
#define TCG_TARGET_CALL_STACK_OFFSET 0

//  Optional instructions, all disabled for mvp version
#define TCG_TARGET_HAS_andc_i32                              0
#define TCG_TARGET_HAS_bswap16_i32                           0
#define TCG_TARGET_HAS_bswap32_i32                           0
#define TCG_TARGET_HAS_deposit_i32                           0
#define TCG_TARGET_HAS_div_i32                               0
#define TCG_TARGET_HAS_eqv_i32                               0
#define TCG_TARGET_HAS_ext16s_i32                            0
#define TCG_TARGET_HAS_ext16u_i32                            0
#define TCG_TARGET_HAS_ext8s_i32                             0
#define TCG_TARGET_HAS_ext8u_i32                             0
#define TCG_TARGET_HAS_extract_i32                           0
#define TCG_TARGET_HAS_movcond_i32                           0
#define TCG_TARGET_HAS_muls2_i32                             0
#define TCG_TARGET_HAS_mulu2_i32                             0
#define TCG_TARGET_HAS_nand_i32                              0
#define TCG_TARGET_HAS_neg_i32                               0
#define TCG_TARGET_HAS_nor_i32                               0
#define TCG_TARGET_HAS_not_i32                               0
#define TCG_TARGET_HAS_orc_i32                               0
#define TCG_TARGET_HAS_rot_i32                               0
#define TCG_TARGET_HAS_MEMORY_BSWAP                          0
#define TCG_TARGET_HAS_atomic_fetch_add_intrinsic_i32        1
#define TCG_TARGET_HAS_atomic_compare_and_swap_intrinsic_i32 1

//  64-bit optional instructions
#define TCG_TARGET_HAS_andc_i64                               0
#define TCG_TARGET_HAS_bswap16_i64                            0
#define TCG_TARGET_HAS_bswap32_i64                            0
#define TCG_TARGET_HAS_bswap64_i64                            0
#define TCG_TARGET_HAS_deposit_i64                            0
#define TCG_TARGET_HAS_div2_i64                               0
#define TCG_TARGET_HAS_eqv_i64                                0
#define TCG_TARGET_HAS_ext16s_i64                             0
#define TCG_TARGET_HAS_ext16u_i64                             0
#define TCG_TARGET_HAS_ext32s_i64                             0
#define TCG_TARGET_HAS_ext32u_i64                             0
#define TCG_TARGET_HAS_ext8s_i64                              0
#define TCG_TARGET_HAS_ext8u_i64                              0
#define TCG_TARGET_HAS_movcond_i64                            0
#define TCG_TARGET_HAS_muls2_i64                              0
#define TCG_TARGET_HAS_mulu2_i64                              0
#define TCG_TARGET_HAS_nand_i64                               0
#define TCG_TARGET_HAS_neg_i64                                0
#define TCG_TARGET_HAS_nor_i64                                0
#define TCG_TARGET_HAS_not_i64                                0
#define TCG_TARGET_HAS_orc_i64                                0
#define TCG_TARGET_HAS_qemu_st8_i32                           0
#define TCG_TARGET_HAS_rot_i64                                0
#define TCG_TARGET_HAS_atomic_fetch_add_intrinsic_i64         1
#define TCG_TARGET_HAS_atomic_compare_and_swap_intrinsic_i64  1
#define TCG_TARGET_HAS_atomic_compare_and_swap_intrinsic_i128 0

/* Whether the host has any atomic intrinsics implemented at all. */
#define TCG_TARGET_HAS_INTRINSIC_ATOMICS                                                                             \
    ((TCG_TARGET_HAS_atomic_fetch_add_intrinsic_i32 == 1) || (TCG_TARGET_HAS_atomic_fetch_add_intrinsic_i64 == 1) || \
     (TCG_TARGET_HAS_atomic_compare_and_swap_intrinsic_i32 == 1) ||                                                  \
     (TCG_TARGET_HAS_atomic_compare_and_swap_intrinsic_i64 == 1) ||                                                  \
     (TCG_TARGET_HAS_atomic_compare_and_swap_intrinsic_i128 == 1))

//  Comments in other targets says this must be synced with cpu-defs.h, but unclear what
//  ascpect needs to be syncronized
//  The 32-bit arm target uses a high numbered callee-saved register, so we're picking
//  a similar one
enum {
    TCG_AREG0 = TCG_REG_R27,
};
//  Taken from arm32 target, unclear if it is needed on aarch64
static inline void flush_icache_range(unsigned long start, unsigned long stop)
{
#if defined(__GNUC__)
    __builtin___clear_cache((char *)start, (char *)stop);
#else
    register unsigned long _beg __asm("a1") = start;
    register unsigned long _end __asm("a2") = stop;
    register unsigned long _flg __asm("a3") = 0;
    __asm __volatile__("swi 0x9f0002" : : "r"(_beg), "r"(_end), "r"(_flg));
#endif
}
