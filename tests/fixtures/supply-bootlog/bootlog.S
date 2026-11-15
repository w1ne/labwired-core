/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 * SPDX-License-Identifier: MIT
 *
 * STM32F401 boot logger for the supply-supervisor tests
 * (crates/core/tests/analog/supply_supervisor.rs, crates/wasm/src/cosim_tests.rs).
 *
 * On every reset it reads RCC_CSR — the reset cause, as ST's HAL
 * __HAL_RCC_GET_FLAG does — appends it to a log in .noinit RAM, counts the
 * boot, and clears the flags with RMVF, as a firmware that reports its reset
 * cause must. Then it spins, counting, so a test can tell running from held.
 *
 *   0x20000000  magic 0xB007C0DE once the log is initialised
 *   0x20000004  boots so far
 *   0x20000008  heartbeat (increments while running)
 *   0x20000010  RCC_CSR at boot 1, 2, ... (8 entries, then it stops logging)
 *
 * Rebuild with ./build.sh (arm-none-eabi binutils).
 */
    .syntax unified
    .cpu cortex-m4
    .thumb

    .section .vectors, "a"
    .word 0x20018000            /* initial SP: top of 96 KiB SRAM */
    .word reset + 1

    .text
    .global reset
    .thumb_func
reset:
    ldr   r4, =0x20000000       /* log base */
    ldr   r5, =0xB007C0DE
    ldr   r0, [r4]
    cmp   r0, r5
    beq   1f
    movs  r0, #0
    str   r0, [r4, #4]          /* boots = 0 */
    str   r0, [r4, #8]          /* heartbeat = 0 */
    str   r5, [r4]              /* magic */
1:
    ldr   r6, =0x40023874       /* RCC_CSR */
    ldr   r1, [r6]              /* reset cause */
    ldr   r2, [r4, #4]          /* boots */
    cmp   r2, #8
    bhs   2f
    add   r3, r4, #0x10
    str   r1, [r3, r2, lsl #2]  /* log[boots] = RCC_CSR */
2:
    adds  r2, r2, #1
    str   r2, [r4, #4]
    ldr   r0, [r6]
    orr   r0, r0, #(1 << 24)    /* RMVF: clear the reset flags */
    str   r0, [r6]
spin:
    ldr   r0, [r4, #8]
    adds  r0, r0, #1
    str   r0, [r4, #8]
    b     spin
    .ltorg
