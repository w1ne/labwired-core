/* LabWired clean-room RP2350 boot-ROM API window (Arm view, 0x0000_0000).
 *
 * NOT a Raspberry Pi ROM dump. Only the interface is Raspberry Pi's, taken
 * from the RP2350 datasheet section 5.4 "Bootrom APIs":
 *   Table 453 (5.4.1): fixed Arm words at 0x00..0x19 -- initial SP and the
 *     reset / NMI / HardFault vectors, the 'M','u',0x02 magic at 0x10, the
 *     version byte at 0x13, and three 16-bit pointers: 0x14 ROM entry table,
 *     0x16 rom_table_lookup_val(), 0x18 rom_table_lookup_entry().
 *   5.4.8: the API codes ('S','R' = 0x5253 ...), the RT_FLAG_* lookup masks
 *     (pico-sdk boot/bootrom_constants.h) and each function's signature.
 * The routine bodies and the table layout are ours.
 *
 * Why it exists: pico-sdk code (crt0 runtime_init, arduino-pico's OTA stage)
 * does `rom_table_lookup = *(uint16_t *)0x16; rom_table_lookup(code, mask)`.
 * Without a ROM at address 0 the Cortex-M flash alias serves the image's own
 * vector table there; the halfword at 0x16 is the upper half of a 0x1000_xxxx
 * vector (0x1000), so the "lookup" branches into flash offset 0x1000 and
 * dies on whatever bytes are there (UsageFault UNDEFINSTR at 0x1008 in the
 * arduino-pico rp2350-zero image).
 *
 * Table layout (ours): per entry .hword code, .hword flags, then one .hword
 * value per set flag bit, lowest bit first; code 0 ends the table. Values
 * for function flags are Thumb entry addresses (bit 0 set); RT_FLAG_DATA
 * values are data addresses.
 *
 * Boot path (rom_reset, taken when the chip descriptor sets
 * `boot_from_rom: true`; datasheet 5.2 "Boot sequence" and 5.9 "Executable
 * images", the flash-image subset):
 *   1. QMI M0 is set to the slow 03h serial XIP read (CLKDIV 12) -- the
 *      mode flash_enter_cmd_xip documents (5.4.8.5).
 *   2. The 256-byte XIP setup function (xip_setup_func, which re-applies
 *      that mode) is copied to the start of boot RAM, 0x400E0000. pico-sdk
 *      hardware_flash copies it out of boot RAM before a program/erase and
 *      calls the copy afterwards to restore XIP (flash.c,
 *      flash_init_boot2_copyout / flash_enable_xip_via_boot2).
 *   3. The first 4 KiB of flash are scanned for a block (PICOBIN marker
 *      0xffffded3 ... 0xab123579, pico-sdk boot/picobin.h). The block loop
 *      is followed through each LAST item's link; exactly one block may
 *      carry IMAGE_TYPE, which must be EXE, Arm, RP2350 and Secure or
 *      unspecified security.
 *   4. VTOR is set to the VECTOR_TABLE item (default: the image start,
 *      0x10000000); MSP/PC come from ENTRY_POINT when present, else from
 *      the vector table; the ROM branches to the entry.
 *   Anything else -- no block, a second IMAGE_DEF, NS/RISC-V/RP2040
 *   images, LOAD_MAP, ROLLING_WINDOW_DELTA, PARTITION_TABLE, HASH_DEF,
 *   SIGNATURE, try-before-you-buy -- executes BKPT, which halts the twin
 *   (silicon would fall back to USB/UART boot). No hash or signature is
 *   ever checked, so no image that asks for one is booted.
 *
 * APIs (rom_table_lookup_val / rom_table_lookup_entry, any mask, NULL on
 * miss):
 *   'S','R' bootrom_state_reset  -- no ROM state to reset: returns
 *   'I','F' connect_internal_flash -- the QSPI pads are not modelled (the
 *     QMI is always connected), so there is nothing to reconnect: returns
 *   'E','X' flash_exit_xip -- QMI direct mode on (EN, CLKDIV 12), basic 03h
 *     XIP, then one FFh byte on CS0 (continuous-read exit)
 *   'R','E' flash_range_erase(addr, count, block_size, block_cmd) -- per
 *     step: 06h WREN, then block_cmd when addr is block_size aligned and a
 *     whole block remains, else 20h (4 KiB); 05h WIP poll
 *   'R','P' flash_range_program(addr, data, count) -- 06h WREN, 02h page
 *     program split at 256-byte page boundaries, 05h WIP poll
 *   'F','C' flash_flush_cache -- the XIP cache is not modelled: DSB/ISB
 *   'C','X' flash_enter_cmd_xip -- runs xip_setup_func (03h, CLKDIV 12)
 *   'G','S' get_sys_info -- CHIP_INFO only (first word reports the subset);
 *     package_sel = 1 (QFN-60, RP2350A), device id = fixed 64-bit value
 *     0x2350_0000_4C57_0001 (the twin has no OTP CHIPID rows)
 *   'R','B' reboot -- REBOOT_TYPE_NORMAL requests a system reset through
 *     AIRCR.SYSRESETREQ (the delay and the watchdog path are not modelled);
 *     every other reboot type (BOOTSEL, RAM_IMAGE, ...) returns
 *     BOOTROM_ERROR_NOT_PERMITTED
 *   'F','D' flash_devinfo16_ptr (data) -- points at a ROM word holding the
 *     address of a FLASH_DEVINFO halfword = 0x0C00 (CS0 16 MB, the
 *     datasheet's default without OTP FLASH_DEVINFO). It lives in ROM, so
 *     flash_devinfo_set_* writes are dropped.
 *
 * NOT modelled (lookup returns NULL): flash_op, flash_select_xip_read_mode,
 * flash_reset_address_trans, OTP access, partition tables, chain_image,
 * secure_call and every other API. Arm-NS callers get the same table as
 * Arm-S.
 *
 * Rebuild: crates/core/roms/rp2350/build.sh (arm-none-eabi binutils). */

    .syntax unified
    .cpu cortex-m33
    .thumb

    .section .text, "ax"
    .global _rom_start
_rom_start:

    /* 0x00: Arm boot vectors (Table 453). Reset runs the boot path when the
     * descriptor boots from ROM; NMI/HardFault park. */
    .word 0x20082000
    .word rom_reset + 1
    .word rom_park + 1
    .word rom_park + 1

    /* 0x10: magic 'M','u',0x02 and bootrom version 2 (A2). */
    .byte 'M', 'u', 0x02, 0x02
    /* 0x14 / 0x16 / 0x18: 16-bit well-known pointers. */
    .hword rom_table
    .hword rom_table_lookup_val + 1
    .hword rom_table_lookup_entry + 1

    .align 2
rom_park:
    b     rom_park

/* void *rom_table_lookup_val(uint32_t code, uint32_t mask) */
rom_table_lookup_val:
    push  {r4, lr}
    bl    rom_table_lookup_entry
    cbz   r0, 1f
    ldrh  r0, [r0]
1:  pop   {r4, pc}

/* uint16_t *rom_table_lookup_entry(uint32_t code, uint32_t mask) */
rom_table_lookup_entry:
    push  {r4, r5, lr}
    ldr   r2, =rom_table
next_entry:
    ldrh  r3, [r2], #2          /* code */
    cbz   r3, not_found
    ldrh  r4, [r2], #2          /* flags */
    cmp   r3, r0
    bne   skip_values
    tst   r4, r1
    beq   not_found
    movs  r5, #1
scan:
    tst   r4, r5
    beq   scan_next
    tst   r1, r5
    bne   found
    adds  r2, #2
scan_next:
    lsls  r5, r5, #1
    b     scan
found:
    mov   r0, r2
    pop   {r4, r5, pc}
skip_values:
    cmp   r4, #0
    beq   next_entry
    tst   r4, #1
    it    ne
    addne r2, r2, #2
    lsrs  r4, r4, #1
    b     skip_values
not_found:
    movs  r0, #0
    pop   {r4, r5, pc}

/* void bootrom_state_reset(uint32_t flags), connect_internal_flash. */
rom_nop:
    bx    lr

    .equ QMI_BASE,      0x400D0000
    .equ BOOTRAM_BASE,  0x400E0000
    .equ XIP_BASE,      0x10000000
    .equ VTOR,          0xE000ED08
    .equ BLOCK_START,   0xffffded3
    .equ BLOCK_END,     0xab123579

/* ---- Boot path -------------------------------------------------------- */
rom_reset:
    bl    xip_setup_func
    /* Install the XIP setup function in boot RAM (64 words). */
    ldr   r0, =xip_setup_func
    ldr   r1, =BOOTRAM_BASE
    movs  r2, #64
1:  ldr   r3, [r0], #4
    str   r3, [r1], #4
    subs  r2, #1
    bne   1b
    /* First block in the first 4 KiB of flash. */
    ldr   r4, =XIP_BASE
    add   r5, r4, #0x1000
    ldr   r3, =BLOCK_START
2:  ldr   r2, [r4]
    cmp   r2, r3
    beq   3f
    adds  r4, #4
    cmp   r4, r5
    blo   2b
    b     boot_fail
3:  mov   r0, r4
    bl    parse_block
    adds  r5, r0, #1            /* r0 == -1: malformed/unsupported */
    beq   boot_fail
    mov   r8, r0                /* IMAGE_TYPE word or 0 */
    mov   r9, r1                /* vector table or 0 */
    mov   r10, r2               /* entry pc or 0 */
    mov   r11, r3               /* entry sp */
    movs  r6, #16               /* bound the loop walk */
    mov   r7, r12               /* next block */
4:  cmp   r7, r4
    beq   5f
    subs  r6, #1
    beq   boot_fail
    ldr   r2, [r7]
    ldr   r3, =BLOCK_START
    cmp   r2, r3
    bne   boot_fail
    mov   r0, r7
    bl    parse_block
    adds  r5, r0, #1
    beq   boot_fail
    cbz   r0, 6f
    cmp   r8, #0                /* a second IMAGE_DEF: not modelled */
    bne   boot_fail
    mov   r8, r0
    mov   r9, r1
    mov   r10, r2
    mov   r11, r3
6:  mov   r7, r12
    b     4b
5:  /* Validate IMAGE_TYPE: EXE, Arm, RP2350, not NS, not TBYB. */
    cmp   r8, #0
    beq   boot_fail
    lsr   r0, r8, #16
    and   r1, r0, #0xf
    cmp   r1, #1                /* EXE */
    bne   boot_fail
    ubfx  r1, r0, #4, #2        /* security: 0 unspecified, 2 S */
    cmp   r1, #1
    beq   boot_fail
    cmp   r1, #3
    beq   boot_fail
    tst   r0, #0x0700           /* CPU must be Arm (0) */
    bne   boot_fail
    ubfx  r1, r0, #12, #3
    cmp   r1, #1                /* chip RP2350 */
    bne   boot_fail
    tst   r0, #0x8000           /* try-before-you-buy */
    bne   boot_fail
    /* Hand off. */
    cmp   r9, #0
    it    eq
    ldreq r9, =XIP_BASE
    ldr   r0, =VTOR
    str   r9, [r0]
    dsb
    isb
    cmp   r10, #0
    beq   7f
    msr   msp, r11
    orr   r10, r10, #1
    bx    r10
7:  ldr   r0, [r9]
    msr   msp, r0
    ldr   r0, [r9, #4]
    bx    r0

boot_fail:
    bkpt  #0x23
    b     boot_fail

/* parse_block(r0 = block start) -> r0 IMAGE_TYPE word (0 if absent, -1 on a
 * malformed or unsupported block), r1 vector table, r2 entry pc, r3 entry
 * sp, r12 next block (start + link). */
parse_block:
    push  {r4-r7, lr}
    adds  r4, r0, #4
    movs  r5, #0
    movs  r6, #0
    movs  r7, #0
    movs  r3, #0
pb_item:
    ldr   r1, [r4]
    uxtb  r2, r1
    cmp   r2, #0xff             /* LAST */
    beq   pb_last
    tst   r2, #0x80
    ite   ne
    ubfxne r12, r1, #8, #16
    ubfxeq r12, r1, #8, #8
    cmp   r12, #0
    beq   pb_bad
    cmp   r2, #0x42             /* IMAGE_TYPE */
    bne   1f
    cmp   r5, #0
    bne   pb_bad
    mov   r5, r1
    b     pb_next
1:  cmp   r2, #0x03             /* VECTOR_TABLE */
    bne   2f
    ldr   r6, [r4, #4]
    b     pb_next
2:  cmp   r2, #0x44             /* ENTRY_POINT */
    bne   3f
    ldr   r7, [r4, #4]
    ldr   r3, [r4, #8]
    b     pb_next
3:  cmp   r2, #0xfe             /* IGNORED */
    beq   pb_next
    cmp   r2, #0x41             /* NEXT_BLOCK_OFFSET */
    beq   pb_next
    b     pb_bad                /* LOAD_MAP, HASH_*, SIGNATURE, ... */
pb_next:
    add   r4, r4, r12, lsl #2
    b     pb_item
pb_last:
    ubfx  r12, r1, #8, #16      /* items size in words */
    adds  r2, r0, #4
    add   r2, r2, r12, lsl #2
    cmp   r2, r4
    bne   pb_bad
    ldr   r1, [r4, #4]          /* link */
    ldr   r2, [r4, #8]
    ldr   r12, =BLOCK_END
    cmp   r2, r12
    bne   pb_bad
    add   r12, r0, r1
    mov   r0, r5
    mov   r1, r6
    mov   r2, r7
    pop   {r4-r7, pc}
pb_bad:
    mov   r0, #-1
    pop   {r4-r7, pc}

    .ltorg

/* ---- QMI direct-mode helpers (r3 = QMI_BASE; clobber r0-r2) ------------ */
qmi_cs_assert:
    ldr   r1, [r3]
    orr   r1, r1, #5            /* EN | ASSERT_CS0N */
    str   r1, [r3]
    bx    lr

qmi_cs_release:
1:  ldr   r1, [r3]
    tst   r1, #2                /* BUSY */
    bne   1b
    bic   r1, r1, #4
    str   r1, [r3]
    bx    lr

/* r0 = byte out -> r0 = byte in */
qmi_xfer:
    str   r0, [r3, #4]
1:  ldr   r1, [r3]
    tst   r1, #0x10000          /* RXEMPTY */
    bne   1b
    ldr   r0, [r3, #8]
    uxtb  r0, r0
    bx    lr

/* r0 = opcode, r2 = 24-bit address: opcode + address, CS left asserted. */
qmi_cmd_addr:
    push  {r4, lr}
    mov   r4, r2
    bl    qmi_xfer
    lsr   r0, r4, #16
    bl    qmi_xfer
    lsr   r0, r4, #8
    bl    qmi_xfer
    mov   r0, r4
    bl    qmi_xfer
    pop   {r4, pc}

flash_wren:
    push  {lr}
    bl    qmi_cs_assert
    movs  r0, #0x06
    bl    qmi_xfer
    bl    qmi_cs_release
    pop   {pc}

flash_wait_ready:
    push  {lr}
    bl    qmi_cs_assert
    movs  r0, #0x05
    bl    qmi_xfer
1:  movs  r0, #0xff
    bl    qmi_xfer
    tst   r0, #1                /* WIP */
    bne   1b
    bl    qmi_cs_release
    pop   {pc}

/* void flash_exit_xip(void) */
flash_exit_xip:
    push  {r4, lr}
    bl    xip_setup_func
    ldr   r3, =QMI_BASE
    ldr   r1, =(12 << 22) | 1   /* CLKDIV 12, EN */
    str   r1, [r3]
    bl    qmi_cs_assert
    movs  r0, #0xff
    bl    qmi_xfer
    bl    qmi_cs_release
    pop   {r4, pc}

/* void flash_range_erase(uint32_t addr, size_t count, uint32_t block_size,
 *                        uint8_t block_cmd) */
flash_range_erase:
    push  {r4-r8, lr}
    bic   r4, r0, #0xff000000
    add   r5, r4, r1            /* end */
    mov   r6, r2
    mov   r7, r3
    ldr   r3, =QMI_BASE
1:  cmp   r4, r5
    bhs   9f
    movs  r8, #0x20
    mov   r12, #0x1000
    cbz   r6, 2f
    cmp   r7, #0
    beq   2f
    sub   r0, r6, #1
    tst   r4, r0                /* block aligned? */
    bne   2f
    sub   r0, r5, r4
    cmp   r0, r6                /* a whole block left? */
    blo   2f
    mov   r8, r7
    mov   r12, r6
2:  push  {r12}
    bl    flash_wren
    bl    qmi_cs_assert
    mov   r0, r8
    mov   r2, r4
    bl    qmi_cmd_addr
    bl    qmi_cs_release
    bl    flash_wait_ready
    pop   {r12}
    add   r4, r4, r12
    b     1b
9:  pop   {r4-r8, pc}

/* void flash_range_program(uint32_t addr, const uint8_t *data, size_t count) */
flash_range_program:
    push  {r4-r6, lr}
    bic   r4, r0, #0xff000000
    mov   r5, r1
    adds  r6, r4, r2            /* end */
    ldr   r3, =QMI_BASE
1:  cmp   r4, r6
    bhs   9f
    bl    flash_wren
    bl    qmi_cs_assert
    movs  r0, #0x02
    mov   r2, r4
    bl    qmi_cmd_addr
2:  ldrb  r0, [r5], #1
    bl    qmi_xfer
    adds  r4, #1
    cmp   r4, r6
    bhs   3f
    tst   r4, #0xff             /* page boundary ends the program */
    bne   2b
3:  bl    qmi_cs_release
    bl    flash_wait_ready
    b     1b
9:  pop   {r4-r6, pc}

/* void flash_flush_cache(void): no XIP cache is modelled. */
flash_flush_cache:
    dsb
    isb
    bx    lr

    .ltorg

/* XIP setup function: position independent, 256 bytes with its literals,
 * copied to boot RAM at boot. Also flash_enter_cmd_xip. QMI M0: 03h serial
 * read, 24-bit address, CLKDIV 12 (datasheet 5.4.8.5); direct mode off. */
    .align 2
xip_setup_func:
    ldr   r3, xip_lit_qmi
1:  ldr   r2, [r3]
    tst   r2, #2                /* wait for direct-mode BUSY to clear */
    bne   1b
    ldr   r1, xip_lit_timing
    str   r1, [r3, #0x0c]       /* M0_TIMING: COOLDOWN 1, CLKDIV 12 */
    movw  r1, #0x1000
    str   r1, [r3, #0x10]       /* M0_RFMT: 8-bit prefix, serial */
    movw  r1, #0xa003
    str   r1, [r3, #0x14]       /* M0_RCMD: 03h */
    bic   r2, r2, #5            /* EN and ASSERT_CS0N off */
    str   r2, [r3]
    dsb
    isb
    bx    lr
    .align 2
xip_lit_qmi:
    .word QMI_BASE
xip_lit_timing:
    .word 0x4000000c
    .space 256 - (. - xip_setup_func), 0

/* int get_sys_info(uint32_t *out, uint32_t out_words, uint32_t flags) */
get_sys_info:
    ands  r2, r2, #1            /* supported subset: CHIP_INFO */
    cbz   r1, too_small
    str   r2, [r0]
    cbz   r2, 1f
    cmp   r1, #4
    blo   too_small
    movs  r3, #1                /* PACKAGE_SEL: QFN-60 (RP2350A) */
    str   r3, [r0, #4]
    ldr   r3, =0x4C570001       /* device id low */
    str   r3, [r0, #8]
    ldr   r3, =0x23500000       /* device id high */
    str   r3, [r0, #12]
    movs  r0, #4
    bx    lr
1:  movs  r0, #1
    bx    lr
too_small:
    mvn   r0, #12               /* BOOTROM_ERROR_BUFFER_TOO_SMALL (-13) */
    bx    lr

/* int reboot(uint32_t flags, uint32_t delay_ms, uint32_t p0, uint32_t p1) */
reboot:
    tst   r0, #0xf              /* REBOOT_TYPE_MASK: 0 = NORMAL */
    bne   1f
    dsb
    ldr   r0, =0xE000ED0C       /* AIRCR */
    ldr   r1, =0x05FA0004       /* VECTKEY | SYSRESETREQ */
    str   r1, [r0]
    dsb
2:  b     2b
1:  mvn   r0, #3                /* BOOTROM_ERROR_NOT_PERMITTED (-4) */
    bx    lr

    .ltorg

    .align 2
flash_devinfo16_ptr:
    .word flash_devinfo16
flash_devinfo16:
    .hword 0x0C00               /* CS0_SIZE = 16 MB, no CS1 */

    .align 1
/* RT_FLAG_FUNC_ARM_SEC = 0x0004, RT_FLAG_FUNC_ARM_NONSEC = 0x0010,
 * RT_FLAG_DATA = 0x0040. Functions carry both Arm flags (S value, then NS). */
rom_table:
    .hword 0x5253, 0x0014, rom_nop + 1, rom_nop + 1            /* 'S','R' */
    .hword 0x4649, 0x0014, rom_nop + 1, rom_nop + 1            /* 'I','F' */
    .hword 0x5845, 0x0014, flash_exit_xip + 1, flash_exit_xip + 1 /* 'E','X' */
    .hword 0x4552, 0x0014, flash_range_erase + 1, flash_range_erase + 1 /* 'R','E' */
    .hword 0x5052, 0x0014, flash_range_program + 1, flash_range_program + 1 /* 'R','P' */
    .hword 0x4346, 0x0014, flash_flush_cache + 1, flash_flush_cache + 1 /* 'F','C' */
    .hword 0x5843, 0x0014, xip_setup_func + 1, xip_setup_func + 1 /* 'C','X' */
    .hword 0x5347, 0x0014, get_sys_info + 1, get_sys_info + 1  /* 'G','S' */
    .hword 0x4252, 0x0014, reboot + 1, reboot + 1              /* 'R','B' */
    .hword 0x4446, 0x0040, flash_devinfo16_ptr                 /* 'F','D' */
    .hword 0x0000

    /* Pad to the 32 KiB ROM window (datasheet 2.2: ROM 0x0000_0000, 32 KB). */
    .org 0x8000
