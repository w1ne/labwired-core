/* LabWired clean-room Apollo4 Plus boot-ROM helper table (AMAP42KP).
 *
 * NOT an Ambiq ROM dump: Ambiq publishes no ROM image. Written from the
 * interface AmbiqSuite SDK 4.4 (hal_ambiq, BSD-3-Clause) calls:
 *   mcu/apollo4p/hal/mcu/am_hal_bootrom_helper.c  g_am_hal_bootrom_helper:
 *     the entry addresses (Thumb, so +1) of each helper;
 *   mcu/apollo4p/hal/am_hal_utils.h / mcu/am_hal_utils.c:
 *     br_util_delay_cycles "executes at 3 cycles per iteration";
 *     br_util_read_word / br_util_write_word: one 32-bit access.
 * Each 4-byte entry is a b.w to the body below; only the addresses are
 * Ambiq's.
 *
 * Modelled: br_util_read_word, br_util_write_word, br_util_delay_cycles
 * (a two-instruction loop: subs + taken bne, 3 cycles per iteration on a
 * Cortex-M4; 0 iterations returns at once). LabWired's Cortex-M model
 * charges the taken branch 1 cycle (no pipeline refill), so the loop runs
 * at 2 cycles per iteration and am_hal_delay_us waits about 2/3 of the
 * requested time.
 *
 * NOT modelled: the MRAM (NV) helpers. nv_mass_erase, nv_page_erase,
 * nv_program_main, nv_program_info_area, nv_program_main2, nv_info_erase
 * and nv_recovery change nothing and return -1 (non-zero = failure to
 * am_hal_mram.c). No other ROM content (the secure boot loader is not run).
 *
 * Rebuild: crates/core/roms/apollo4/build.sh (arm-none-eabi binutils). */

    .syntax unified
    .cpu cortex-m4
    .thumb

    .section .text, "ax"
    .global _rom_start
_rom_start:

    .org 0x4C
    b.w   nv_unmodelled                /* 0x4C nv_mass_erase */
    b.w   nv_unmodelled                /* 0x50 nv_page_erase */
    b.w   nv_unmodelled                /* 0x54 nv_program_main */
    b.w   nv_unmodelled                /* 0x58 nv_program_info_area */
    .org 0x6C
    b.w   nv_unmodelled                /* 0x6C nv_program_main2 */
    .org 0x74
    b.w   br_util_read_word            /* 0x74 */
    b.w   br_util_write_word           /* 0x78 */
    .org 0x80
    b.w   nv_unmodelled                /* 0x80 nv_info_erase */
    .org 0x98
    b.w   nv_unmodelled                /* 0x98 nv_recovery */
    b.w   br_util_delay_cycles         /* 0x9C */

    .org 0x100
/* uint32_t br_util_read_word(uint32_t *addr) */
    .thumb_func
br_util_read_word:
    ldr   r0, [r0]
    bx    lr

/* void br_util_write_word(uint32_t *addr, uint32_t data) */
    .thumb_func
br_util_write_word:
    str   r1, [r0]
    bx    lr

/* void br_util_delay_cycles(uint32_t iterations) */
    .thumb_func
br_util_delay_cycles:
    cbz   r0, 2f
1:  subs  r0, r0, #1
    bne   1b
2:  bx    lr

/* MRAM helpers: not modelled, report failure. */
    .thumb_func
nv_unmodelled:
    mov   r0, #-1
    bx    lr
