/* LabWired clean-room RP2040 boot ROM (Arm view, 0x0000_0000).
 *
 * NOT a Raspberry Pi ROM dump, and no Raspberry Pi ROM code was copied,
 * disassembled or ported. Only the interface is Raspberry Pi's, taken from
 * the public RP2040 datasheet section 2.8 "Bootrom":
 *   2.8.2  the fixed words at 0x00..0x1b: initial SP and reset/NMI/HardFault
 *          vectors, the 'M','u',0x01 magic at 0x10, the version byte at 0x13,
 *          the 16-bit function-table (0x14) and data-table (0x16) pointers
 *          and the 16-bit rom_table_lookup pointer (0x18);
 *   2.8.3  the lookup codes and each function's signature and behaviour:
 *          Table 164 bit ops (P3 R3 L3 T3), 2.8.3.1.2 memory fill/copy
 *          (MS S4 MC C4), Tables 169/170 the soft-float 'SF' and soft-double
 *          'SD' tables (V2/V3: 32 entries, offsets 0x00..0x7c, 0x48 = sincos
 *          in V3). The word count before each float table is what pico-sdk
 *          (pico_float/pico_double init) asserts for a V2+ ROM.
 * The routine bodies and the layout are ours.
 *
 * Bytes 0x00..0x5b are the earlier minimal stub, unchanged, so every
 * existing entry keeps its address.
 *
 * Floating point is high-level emulated: each SF/SD entry is
 * `bkpt #imm ; bx lr` with imm = 0xC0 + offset/4 (float) or 0xE0 + offset/4
 * (double). The Cortex-M core serves those immediates natively in
 * crates/core/src/cpu/cortex_m/rp2040_rom_hle.rs, with the datasheet's
 * 2.8.3.2.1 rules (denormal in/out flushed to zero, NaN in/out as infinity,
 * round to nearest even), and only when this ROM's magic is mapped at 0x10.
 *
 * Rebuild: ./build.sh (arm-none-eabi binutils), then commit bootrom.bin.
 */
    .syntax unified
    .cpu cortex-m0plus
    .thumb
    .text
    .global _rom_start
_rom_start:
    .word 0x20042000            /* 0x00 initial SP */
    .word dead + 1              /* 0x04 reset */
    .word dead + 1              /* 0x08 NMI */
    .word dead + 1              /* 0x0c HardFault */
    .byte 'M', 'u', 0x01        /* 0x10 magic */
    .byte 3                     /* 0x13 version: V3 (SF/SD tables, sincos) */
    .hword func_table           /* 0x14 */
    .hword data_table           /* 0x16 */
    .hword table_lookup + 1     /* 0x18 rom_table_lookup */
    .hword 0

/* 0x1c  void *rom_table_lookup(uint16_t *table, uint32_t code) */
table_lookup:
    movs  r3, #0
1:  ldrh  r2, [r0]
    cmp   r2, r3
    beq   2f
    ldrh  r3, [r0, #2]
    adds  r0, #4
    cmp   r1, r2
    bne   table_lookup
2:  movs  r0, r3
    bx    lr

/* 0x30  reset / reset_usb_boot / wait_for_vector: nothing to boot into */
dead:
    wfi
    b     dead

/* 0x34  flash helpers: the twin maps flash directly, nothing to do */
ret:
    bx    lr
    nop

/* 0x38  memset(ptr r0, c r1, n r2) -> r0 (also memset4) */
memset:
    push  {r4, lr}
    movs  r3, #0
1:  cmp   r3, r2
    bcs   2f
    strb  r1, [r0, r3]
    adds  r3, #1
    b     1b
2:  pop   {r4, pc}

/* 0x48  memcpy(dst r0, src r1, n r2) -> r0 (also memcpy44) */
memcpy:
    push  {r4, lr}
    movs  r3, #0
1:  cmp   r3, r2
    bcs   2f
    ldrb  r4, [r1, r3]
    strb  r4, [r0, r3]
    adds  r3, #1
    b     1b
2:  pop   {r4, pc}
    nop

/* 0x5c */
func_table:
    .byte 'M','S'; .hword memset + 1
    .byte 'S','4'; .hword memset + 1
    .byte 'M','C'; .hword memcpy + 1
    .byte 'C','4'; .hword memcpy + 1
    .byte 'U','B'; .hword dead + 1
    .byte 'D','T'; .hword ret + 1
    .byte 'D','E'; .hword ret + 1
    .byte 'W','V'; .hword dead + 1
    .byte 'I','F'; .hword ret + 1
    .byte 'E','X'; .hword ret + 1
    .byte 'R','E'; .hword ret + 1
    .byte 'R','P'; .hword ret + 1
    .byte 'F','C'; .hword ret + 1
    .byte 'C','X'; .hword ret + 1
    .byte 'P','3'; .hword popcount32 + 1
    .byte 'R','3'; .hword reverse32 + 1
    .byte 'L','3'; .hword clz32 + 1
    .byte 'T','3'; .hword ctz32 + 1
    .hword 0, 0

data_table:
    .byte 'G','R'; .hword git_rev
    .byte 'S','F'; .hword sf_table
    .byte 'S','D'; .hword sd_table
    .hword 0, 0

    .balign 4
git_rev:
    .word 0x00000001

/* Bit ops (datasheet Table 164). Only r0 changes. */
popcount32:                     /* number of 1 bits */
    push  {r1, r2}
    movs  r1, #0
1:  cmp   r0, #0
    beq   2f
    subs  r2, r0, #1
    ands  r0, r2                /* clear the lowest set bit */
    adds  r1, #1
    b     1b
2:  movs  r0, r1
    pop   {r1, r2}
    bx    lr

reverse32:                      /* bit order reversed */
    push  {r1, r2}
    movs  r1, #0
    movs  r2, #32
1:  lsrs  r0, r0, #1            /* C = next low bit */
    adcs  r1, r1                /* shift it in at the bottom */
    subs  r2, #1
    bne   1b
    movs  r0, r1
    pop   {r1, r2}
    bx    lr

clz32:                          /* leading zeros; 32 for 0 */
    push  {r1}
    movs  r1, #32
1:  cmp   r0, #0
    beq   2f
    lsrs  r0, r0, #1
    subs  r1, #1
    b     1b
2:  movs  r0, r1
    pop   {r1}
    bx    lr

ctz32:                          /* trailing zeros; 32 for 0 */
    push  {r1}
    movs  r1, #32
    cmp   r0, #0
    beq   2f
    movs  r1, #0
1:  lsrs  r0, r0, #1
    bcs   2f
    adds  r1, #1
    b     1b
2:  movs  r0, r1
    pop   {r1}
    bx    lr

/* Soft-float / soft-double tables. pico-sdk reads the entry count from
 * the byte two below the table. */
    .macro hle_stubs base
    .irp n, 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31
    bkpt  #(\base + \n)
    bx    lr
    .endr
    .endm

    .macro hle_table stubs
    .irp n, 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31
    .word \stubs + 4 * \n + 1
    .endr
    .endm

    .balign 4
sf_stubs:
    hle_stubs 0xC0
sd_stubs:
    hle_stubs 0xE0

    .balign 4
    .hword 0
    .byte 32, 0                 /* sf_table - 2: entry count */
sf_table:
    hle_table sf_stubs
    .hword 0
    .byte 32, 0                 /* sd_table - 2: entry count */
sd_table:
    hle_table sd_stubs
