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
 * NOT modelled (lookup returns NULL): flash_range_erase/program, flash_op,
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
    .hword 0x5845, 0x0014, rom_nop + 1, rom_nop + 1            /* 'E','X' */
    .hword 0x4346, 0x0014, rom_nop + 1, rom_nop + 1            /* 'F','C' */
    .hword 0x5843, 0x0014, rom_nop + 1, rom_nop + 1            /* 'C','X' */
    .hword 0x5347, 0x0014, get_sys_info + 1, get_sys_info + 1  /* 'G','S' */
    .hword 0x4252, 0x0014, reboot + 1, reboot + 1              /* 'R','B' */
    .hword 0x4446, 0x0040, flash_devinfo16_ptr                 /* 'F','D' */
    .hword 0x0000

    /* Pad to the 32 KiB ROM window (datasheet 2.2: ROM 0x0000_0000, 32 KB). */
    .org 0x8000
