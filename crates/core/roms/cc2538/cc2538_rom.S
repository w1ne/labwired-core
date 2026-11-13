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
 *     §8.10      FLASH_CTRL: FCTL/FADDR/FWDATA, DIECFG0 CHIPID [31:16] and
 *                FLASH_SIZE [6:4] (Table 8-2: 001-100 = 128/256/384/512 KB)
 *
 * The API table sits right after the Cortex-M3 vector table, at 0x48, which
 * is where Contiki-NG `rom-util.h` (ROM_UTIL_API) and TI driverlib `rom.h`
 * read it. The routine bodies are ours; only the entry order is TI's.
 *
 * NOT modelled (the real ROM does these):
 * - the serial boot loader (UART0/SSI0) and the boot-loader backdoor pin:
 *   with no valid image this ROM parks in WFI instead;
 * - RAM BIST and the security certificates;
 * - PageErase/ProgramFlash leave FCTL.CM alone (TI driverlib saves and
 *   restores it around the calls because the real ROM changes it).
 *
 * PageErase/ProgramFlash drive FLASH_CTRL as SWRU319C §8.3.1 (write
 * sequence: FADDR, FCTL.WRITE, FWDATA while FULL = 0) and §8.5 (page erase:
 * FADDR, FCTL.ERASE, poll BUSY) describe, with interrupts masked for the
 * duration (§8.3.4) and FCTL updated read-modify-write so a caller's
 * UPPER_PAGE_ACCESS survives. FCTL.ABORT after an operation -> -1.
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
    .equ FLASH_CTRL_FCTL,    0x400D3008
    .equ FLASH_CTRL_FADDR,   0x400D300C
    .equ FLASH_CTRL_FWDATA,  0x400D3010
    .equ FLASH_CTRL_DIECFG0, 0x400D3014
    .equ FCTL_ERASE,      0x01
    .equ FCTL_WRITE,      0x02
    .equ FCTL_ABORT,      0x20
    .equ FCTL_FULL,       0x40
    .equ FCTL_BUSY,       0x80
    .equ PAGE_SIZE,       0x800

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

/* unsigned long ROM_GetFlashSize(void): bytes, from DIECFG0.FLASH_SIZE
 * (Table 8-2: code n = n x 128 KB; driverlib FlashSizeGet: 0 = 64 KB). */
    .thumb_func
rom_get_flash_size:
    ldr   r0, =FLASH_CTRL_DIECFG0
    ldr   r0, [r0]
    ubfx  r1, r0, #4, #3
    lsls  r0, r1, #17
    it    eq
    moveq r0, #0x10000
    bx    lr

/* unsigned long ROM_GetChipId(void): DIECFG0.CHIPID. */
    .thumb_func
rom_get_chip_id:
    ldr   r0, =FLASH_CTRL_DIECFG0
    ldr   r0, [r0]
    lsrs  r0, r0, #16
    bx    lr

/* Wait for FCTL.BUSY = 0; r12 = FCTL address. Clobbers r3. */
    .thumb_func
fctl_wait_idle:
1:  ldr   r3, [r12]
    tst   r3, #FCTL_BUSY
    bne   1b
    bx    lr

/* long ROM_PageErase(unsigned long addr, unsigned long size)
 * -2: addr not word-aligned or [addr, addr+size) outside the main array;
 * -1: the controller aborted (locked page). */
    .thumb_func
rom_page_erase:
    push  {r4-r6, lr}
    bl    check_range                  /* r0 = addr, r1 = size */
    cbnz  r2, 9f
    cbz   r1, 7f                       /* nothing to erase */
    add   r5, r0, r1                   /* end */
    lsrs  r4, r0, #11                  /* first page: addr & ~0x7FF */
    lsls  r4, r4, #11
    mrs   r6, primask
    cpsid i
    ldr   r12, =FLASH_CTRL_FCTL
1:  cmp   r4, r5
    bhs   2f
    bl    fctl_wait_idle
    ldr   r3, =FLASH_BASE
    sub   r3, r4, r3
    str   r3, [r12, #(FLASH_CTRL_FADDR - FLASH_CTRL_FCTL)]
    ldr   r3, [r12]
    orr   r3, r3, #FCTL_ERASE
    str   r3, [r12]
    bl    fctl_wait_idle
    tst   r3, #FCTL_ABORT
    bne   3f
    add   r4, r4, #PAGE_SIZE
    b     1b
2:  movs  r0, #0
    b     4f
3:  mov   r0, #-1
4:  msr   primask, r6
    pop   {r4-r6, pc}
7:  movs  r0, #0
    pop   {r4-r6, pc}
9:  mov   r0, r2
    pop   {r4-r6, pc}

/* long ROM_ProgramFlash(unsigned long *data, unsigned long addr,
 *                       unsigned long count)
 * -2: addr or count not a multiple of 4, or the range outside the array;
 * -1: the write sequence aborted (locked page) or timed out. */
    .thumb_func
rom_program_flash:
    push  {r4-r6, lr}
    mov   r4, r0                       /* data */
    mov   r0, r1
    mov   r1, r2
    tst   r1, #3
    bne   8f
    bl    check_range                  /* r0 = addr, r1 = count */
    cbnz  r2, 9f
    cbz   r1, 2f
    mov   r5, r1                       /* bytes left */
    mrs   r6, primask
    cpsid i
    ldr   r12, =FLASH_CTRL_FCTL
    bl    fctl_wait_idle
    ldr   r3, =FLASH_BASE
    sub   r3, r0, r3
    str   r3, [r12, #(FLASH_CTRL_FADDR - FLASH_CTRL_FCTL)]
    ldr   r3, [r12]
    orr   r3, r3, #FCTL_WRITE
    str   r3, [r12]
1:  ldr   r3, [r12]                    /* §8.3.1 step 4: wait FULL = 0 */
    tst   r3, #FCTL_BUSY               /* sequence ended early: abort or */
    beq   3f                           /* time-out (step 5) */
    tst   r3, #FCTL_FULL
    bne   1b
    ldr   r3, [r4], #4
    str   r3, [r12, #(FLASH_CTRL_FWDATA - FLASH_CTRL_FCTL)]
    subs  r5, r5, #4
    bne   1b
    bl    fctl_wait_idle               /* last word programmed */
    tst   r3, #FCTL_ABORT
    bne   3f
    msr   primask, r6
2:  movs  r0, #0
    pop   {r4-r6, pc}
3:  msr   primask, r6
    mov   r0, #-1
    pop   {r4-r6, pc}
8:  mvn   r0, #1                       /* -2 */
    pop   {r4-r6, pc}
9:  mov   r0, r2
    pop   {r4-r6, pc}

/* r0 = addr, r1 = length: r2 = 0 if addr is word-aligned and
 * [addr, addr + length) lies in the main array, else -2. */
    .thumb_func
check_range:
    mvn   r2, #1
    tst   r0, #3
    it    ne
    bxne  lr
    ldr   r3, =FLASH_BASE
    cmp   r0, r3
    it    lo
    bxlo  lr
    ldr   r3, =FLASH_END
    cmp   r0, r3
    it    hs
    bxhs  lr
    sub   r3, r3, r0                   /* room left */
    cmp   r1, r3
    it    hi
    bxhi  lr
    movs  r2, #0
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
