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
 * Modelled:
 *   rom_table_lookup_val / rom_table_lookup_entry (any mask, NULL on miss)
 *   'S','R' bootrom_state_reset  -- no ROM state to reset: returns
 *   'I','F' connect_internal_flash, 'E','X' flash_exit_xip,
 *   'F','C' flash_flush_cache, 'C','X' flash_enter_cmd_xip -- the twin maps
 *     flash directly at XIP_BASE (no QMI/XIP state machine), so XIP is
 *     always in its "entered" state and these change nothing
 *   'G','S' get_sys_info -- CHIP_INFO only (first word reports the subset);
 *     package_sel read from SYSINFO.PACKAGE_SEL, device id = fixed 64-bit value
 *     0x2350_0000_4C57_0001 (the twin has no OTP CHIPID rows)
 *   'R','B' reboot -- REBOOT_TYPE_NORMAL requests a system reset through
 *     AIRCR.SYSRESETREQ (the delay and the watchdog path are not modelled);
 *     every other reboot type (BOOTSEL, RAM_IMAGE, ...) returns
 *     BOOTROM_ERROR_NOT_PERMITTED
 *   'F','D' flash_devinfo16_ptr (data) -- points at a ROM word holding the
 *     address of a writable FLASH_DEVINFO halfword in BOOTRAM at 0x400E0100,
 *     initially 0x0C00 (CS0 16 MB, the documented no-OTP default). The
 *     exact bundled clean-room image receives this post-ROM state and a
 *     relocatable cacheless XIP-restore helper from SystemBus::from_config.
 *     Replacement ROMs retain control of their own BOOTRAM initialization.
 *
 *   'R','E' flash_range_erase and 'R','P' flash_range_program -- functional
 *     mapped-flash operations, shared with the RP2040 clean-room model:
 *     erase sets bytes to 0xFF; program applies NOR bitwise-AND. No serial
 *     flash command timing, concurrent-XIP exclusion or partition checking.
 *
 * NOT modelled (lookup returns NULL): flash_op,
 * OTP access, partition tables, chain_image, secure_call and every other
 * API; the boot path itself (IMAGE_DEF scan, XIP setup) -- the simulator
 * enters the image's vector table directly. Arm-NS callers get the same
 * table as Arm-S.
 *
 * Rebuild: crates/core/roms/rp2350/build.sh (arm-none-eabi binutils). */

    .syntax unified
    .cpu cortex-m33
    .thumb

    .section .text, "ax"
    .global _rom_start
_rom_start:

    /* 0x00: Arm boot vectors (Table 453). The twin never takes them; a core
     * that lands here parks. */
    .word 0x20082000
    .word rom_park + 1
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

/* void bootrom_state_reset(uint32_t flags) and the XIP-state calls. */
rom_nop:
    bx    lr

/* int get_sys_info(uint32_t *out, uint32_t out_words, uint32_t flags) */
get_sys_info:
    ands  r2, r2, #1            /* supported subset: CHIP_INFO */
    cbz   r1, too_small
    str   r2, [r0]
    cbz   r2, 1f
    cmp   r1, #4
    blo   too_small
    ldr   r3, =0x40000004       /* SYSINFO.PACKAGE_SEL: 0 = QFN80, 1 = QFN60 */
    ldr   r3, [r3]
    and   r3, r3, #1
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

/* RP2350 datasheet 5.4.8.10/11: low-level flash offsets and byte counts.
 * Legal caller inputs are sector-aligned (4096) for erase and page-aligned
 * (256) for program. As on silicon, these low-level APIs do not validate the
 * arguments; flash_op, which performs bounds/alignment/permission checks,
 * remains unsupported. The mapped flash extent still enforces bus bounds.
 * The existing non-TrustZone model exposes the same functional routine for
 * both Arm lookup masks; this does not model Non-secure flash permissions. */
flash_range_erase:
    push  {r4, r5, lr}
    movs  r4, #1
    lsls  r4, r4, #28           /* XIP_BASE 0x10000000 */
    adds  r0, r4
    movs  r4, #0xff
    movs  r5, #0
1:  cmp   r5, r1
    bcs   2f
    strb  r4, [r0, r5]
    adds  r5, #1
    b     1b
2:  pop   {r4, r5, pc}

flash_range_program:
    push  {r4, r5, lr}
    movs  r3, #1
    lsls  r3, r3, #28           /* XIP_BASE 0x10000000 */
    adds  r0, r3
    movs  r3, #0
1:  cmp   r3, r2
    bcs   2f
    ldrb  r4, [r1, r3]
    ldrb  r5, [r0, r3]
    ands  r4, r5
    strb  r4, [r0, r3]
    adds  r3, #1
    b     1b
2:  pop   {r4, r5, pc}

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
    .word 0x400e0100            /* writable FD in post-ROM BOOTRAM; default 0x0c00 */

    .align 1
/* RT_FLAG_FUNC_ARM_SEC = 0x0004, RT_FLAG_FUNC_ARM_NONSEC = 0x0010,
 * RT_FLAG_DATA = 0x0040. Functions carry both Arm flags (S value, then NS). */
rom_table:
    .hword 0x5253, 0x0014, rom_nop + 1, rom_nop + 1            /* 'S','R' */
    .hword 0x4649, 0x0014, rom_nop + 1, rom_nop + 1            /* 'I','F' */
    .hword 0x5845, 0x0014, rom_nop + 1, rom_nop + 1            /* 'E','X' */
    .hword 0x4346, 0x0014, rom_nop + 1, rom_nop + 1            /* 'F','C' */
    .hword 0x5843, 0x0014, rom_nop + 1, rom_nop + 1            /* 'C','X' */
    .hword 0x4552, 0x0014, flash_range_erase + 1, flash_range_erase + 1 /* 'R','E' */
    .hword 0x5052, 0x0014, flash_range_program + 1, flash_range_program + 1 /* 'R','P' */
    .hword 0x5347, 0x0014, get_sys_info + 1, get_sys_info + 1  /* 'G','S' */
    .hword 0x4252, 0x0014, reboot + 1, reboot + 1              /* 'R','B' */
    .hword 0x4446, 0x0040, flash_devinfo16_ptr                 /* 'F','D' */
    .hword 0x0000

    /* Pad to the 32 KiB ROM window (datasheet 2.2: ROM 0x0000_0000, 32 KB). */
    .org 0x8000
