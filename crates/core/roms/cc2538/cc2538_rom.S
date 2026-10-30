/* LabWired clean-room CC2538 boot ROM (CC2538SF53: 512 KiB flash).
 *
 * NOT a TI ROM dump. Written from the public interface in
 *   SWRU333A "CC2538 ROM User's Guide" (April 2013)
 *     §2.2   ROM start-up sequence (steps 1-4)
 *     §2.2.1 valid-image test (Image Valid == 0, entry point in flash,
 *            0x80-aligned)
 *     §3.2   utility API: prototypes, arguments and return values
 *   SWRU319C "CC2538 User's Guide"
 *     Table 4-1  ROM at 0x0000_0000-0x0001_FFFF, flash at 0x0020_0000
 *     §8.6       CCA in the top flash page (0x0027_FFD4 on the 512 KiB part)
 *     FLASH_CTRL_DIECFG0[31:16] CHIPID = 0xB964
 *
 * The API table sits right after the Cortex-M3 vector table, at 0x48, which
 * is where Contiki-NG `rom-util.h` (ROM_UTIL_API) and TI driverlib `rom.h`
 * read it. The routine bodies are ours; only the entry order is TI's.
 *
 * NOT modelled (the real ROM does these):
 * - the serial boot loader (UART0/SSI0) and the boot-loader backdoor pin:
 *   with no valid image this ROM parks in WFI instead;
 * - RAM BIST and the security certificates;
 * - PageErase/ProgramFlash: there is no flash-controller model, so both
 *   return -1 ("error encountered", §3.2.2.8/9) and change nothing;
 * - GetChipId/GetFlashSize read FLASH_CTRL_DIECFG0 on silicon; here they
 *   return the CC2538SF53 constants.
 *
 * Rebuild: crates/core/roms/cc2538/build.sh (arm-none-eabi binutils). */

    .syntax unified
    .cpu cortex-m3
    .thumb

    .equ CCA_IMAGE_VALID, 0x0027FFD8   /* CCA page bytes 2008-2011 */
    .equ FLASH_BASE,      0x00200000
    .equ FLASH_END,       0x00280000   /* 512 KiB */
    .equ SCB_VTOR,        0xE000ED08
    .equ SCB_AIRCR,       0xE000ED0C
    .equ CHIP_ID,         0xB964

    .section .text, "ax"
    .global _rom_start
_rom_start:
/* 0x00: minimal ROM vector table (§2.2 Note). Stack top = end of SRAM. */
vectors:
    .word 0x20008000
    .word rom_reset + 1
    .rept 16
    .word rom_halt + 1
    .endr
/* 0x48: utility function API table (SWRU333A §3.2, rom.h order). */
api_table:
    .word rom_crc32 + 1
    .word rom_get_flash_size + 1
    .word rom_get_chip_id + 1
    .word rom_page_erase + 1
    .word rom_program_flash + 1
    .word rom_reset_device + 1
    .word rom_memset + 1
    .word rom_memcpy + 1
    .word rom_memcmp + 1
    .word rom_memmove + 1

    .thumb_func
rom_reset:
    /* §2.2 step 1 / §2.2.1: valid image? */
    ldr   r0, =CCA_IMAGE_VALID
    ldr   r1, [r0]
    cbnz  r1, rom_no_image
    ldr   r1, [r0, #4]                 /* Application Entry Point */
    ldr   r2, =FLASH_BASE
    cmp   r1, r2
    blo   rom_no_image
    ldr   r2, =FLASH_END
    cmp   r1, r2
    bhs   rom_no_image
    tst   r1, #0x7F
    bne   rom_no_image
    /* step 2: VTOR = flash vector table */
    ldr   r2, =SCB_VTOR
    str   r1, [r2]
    /* step 3: SP from the flash table */
    ldr   r2, [r1]
    msr   msp, r2
    /* step 4: branch to the flash reset ISR */
    ldr   r2, [r1, #4]
    bx    r2

    .thumb_func
rom_no_image:                          /* real ROM: serial boot loader */
rom_halt:
    wfi
    b     rom_halt

/* unsigned long ROM_Crc32(unsigned char *data, unsigned long count)
 * CRC-32 IEEE 802.3: reflected 0xEDB88320, init and final XOR 0xFFFFFFFF. */
    .thumb_func
rom_crc32:
    push  {r4}
    mvn   r2, #0
    ldr   r3, =0xEDB88320
1:  cbz   r1, 3f
    ldrb  r12, [r0], #1
    eor   r2, r2, r12
    movs  r4, #8
2:  lsrs  r2, r2, #1
    it    cs
    eorcs r2, r2, r3
    subs  r4, r4, #1
    bne   2b
    subs  r1, r1, #1
    b     1b
3:  mvn   r0, r2
    pop   {r4}
    bx    lr

    .thumb_func
rom_get_flash_size:
    ldr   r0, =(FLASH_END - FLASH_BASE)
    bx    lr

    .thumb_func
rom_get_chip_id:
    movw  r0, #CHIP_ID
    bx    lr

/* long ROM_PageErase(addr, size) / ROM_ProgramFlash(data, addr, count) */
    .thumb_func
rom_page_erase:
    .thumb_func
rom_program_flash:
    mov   r0, #-1
    bx    lr

/* void ROM_ResetDevice(void): system reset (AIRCR.SYSRESETREQ). */
    .thumb_func
rom_reset_device:
    ldr   r0, =SCB_AIRCR
    ldr   r1, =0x05FA0004
    dsb
    str   r1, [r0]
    dsb
1:  b     1b

/* void *ROM_memset(void *dst, int c, size_t n) -> dst */
    .thumb_func
rom_memset:
    mov   r3, r0
1:  cbz   r2, 2f
    strb  r1, [r3], #1
    subs  r2, r2, #1
    b     1b
2:  bx    lr

/* void *ROM_memcpy(void *dst, const void *src, size_t n) -> dst */
    .thumb_func
rom_memcpy:
    mov   r3, r0
1:  cbz   r2, 2f
    ldrb  r12, [r1], #1
    strb  r12, [r3], #1
    subs  r2, r2, #1
    b     1b
2:  bx    lr

/* int ROM_memcmp(const void *s1, const void *s2, size_t n) */
    .thumb_func
rom_memcmp:
1:  cbz   r2, 2f
    ldrb  r3, [r0], #1
    ldrb  r12, [r1], #1
    subs  r3, r3, r12
    bne   3f
    subs  r2, r2, #1
    b     1b
2:  movs  r0, #0
    bx    lr
3:  mov   r0, r3
    bx    lr

/* void *ROM_memmove(void *dst, const void *src, size_t n) -> dst
 * Forward copy unless dst lies inside (src, src+n). */
    .thumb_func
rom_memmove:
    cmp   r0, r1
    bls   rom_memcpy
    add   r3, r1, r2
    cmp   r0, r3
    bhs   rom_memcpy
    add   r3, r0, r2                   /* backward from the ends */
    add   r1, r1, r2
1:  cbz   r2, 2f
    ldrb  r12, [r1, #-1]!
    strb  r12, [r3, #-1]!
    subs  r2, r2, #1
    b     1b
2:  bx    lr

    .ltorg
