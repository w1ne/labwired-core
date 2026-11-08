/*
 * Xtensa core-ISA smoke for the imported Renode sample controller.
 * Execution starts at the reset vector, the rom window at 0x50000000; the
 * stack and result words live in instram0 (0x40000000, 128 KiB). Only the
 * Xtensa core ISA and the code-density (.n) forms the assembler picks are
 * used: no MUL, no windowed calls, no SIMCALL (Renode's semihosting UART is
 * not modelled), no interrupts.
 *   bit0: counted loop sums 1..100 = 5050
 *   bit1: CALL0/RET round trip returns the subroutine's value
 *   bit2: L32R reads a literal from the rom window
 *   bit3: S8I/S16I then L8UI/L16UI/L32I read back the same bytes
 *   bit4: SRLI/SRAI/EXTUI/SLLI give the expected shifts
 *   bit5: BLTU/BGE/BEQ take signed and unsigned branches correctly
 * The pass word 0x5A5C0000|bits is stored at 0x40000000 (all six = 0x5A5C003F)
 * and the loop sum at 0x40000004.
 */
    .section .text.start, "ax"
    .global _start
    .align 4
_start:
    j       main

    .literal_position
    .literal rom_word, 0x58C0FFEE
    .align 4
main:
    movi    a1, 0x40020000          /* stack top: end of instram0 */
    movi    a2, 0x40000000          /* result block */
    movi    a3, 0
    s32i    a3, a2, 0
    s32i    a3, a2, 4
    movi    a4, 0                   /* a4 = pass bits */

    /* bit0: sum 1..100 */
    movi    a5, 0
    movi    a6, 100
1:  add     a5, a5, a6
    addi    a6, a6, -1
    bnez    a6, 1b
    s32i    a5, a2, 4
    movi    a7, 5050
    bne     a5, a7, 2f
    movi    a7, 0x01
    or      a4, a4, a7
2:
    /* bit1: CALL0 / RET */
    movi    a8, 0
    call0   sub_marker
    movi    a7, 0x0CA11ED0
    bne     a8, a7, 3f
    movi    a7, 0x02
    or      a4, a4, a7
3:
    /* bit2: literal in rom */
    l32r    a9, rom_word
    l32i    a10, a2, 0              /* still zero: guards against an aliased write */
    bnez    a10, 4f
    movi    a7, 0x58C0FFEE
    bne     a9, a7, 4f
    movi    a7, 0x04
    or      a4, a4, a7
4:
    /* bit3: byte / halfword stores and loads */
    movi    a7, 0xA5
    s8i     a7, a2, 8
    movi    a7, 0x5A
    s8i     a7, a2, 9
    movi    a7, 0xBEEF
    s16i    a7, a2, 10
    l8ui    a10, a2, 9
    movi    a7, 0x5A
    bne     a10, a7, 5f
    l16ui   a10, a2, 10
    movi    a7, 0xBEEF
    bne     a10, a7, 5f
    l32i    a10, a2, 8
    movi    a7, 0xBEEF5AA5
    bne     a10, a7, 5f
    movi    a7, 0x08
    or      a4, a4, a7
5:
    /* bit4: shifts and field extract */
    movi    a7, 0x80000000
    srai    a10, a7, 4              /* 0xF8000000 */
    movi    a11, 0xF8000000
    bne     a10, a11, 6f
    srli    a10, a7, 4              /* 0x08000000 */
    movi    a11, 0x08000000
    bne     a10, a11, 6f
    movi    a7, 0xA5C3
    extui   a10, a7, 4, 8           /* 0x5C */
    movi    a11, 0x5C
    bne     a10, a11, 6f
    slli    a10, a10, 12            /* 0x5C000 */
    movi    a11, 0x5C000
    bne     a10, a11, 6f
    movi    a7, 0x10
    or      a4, a4, a7
6:
    /* bit5: signed vs unsigned compares */
    movi    a10, -1
    movi    a11, 1
    bltu    a10, a11, 7f            /* 0xFFFFFFFF <u 1 must not branch */
    bge     a10, a11, 7f            /* -1 >= 1 must not branch */
    blt     a10, a11, 8f            /* -1 < 1 must branch */
    j       7f
8:  beq     a11, a11, 9f
    j       7f
9:  movi    a7, 0x20
    or      a4, a4, a7
7:
    movi    a7, 0x5A5C0000
    or      a4, a4, a7
    s32i    a4, a2, 0
    memw
idle:
    j       idle

    .align 4
sub_marker:
    movi    a8, 0x0CA11ED0
    ret
