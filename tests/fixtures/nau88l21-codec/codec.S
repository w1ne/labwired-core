/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 * SPDX-License-Identifier: MIT
 *
 * NAU88L21 codec bring-up over an i.MX RT LPI2C master, for the codec e2e
 * test (crates/cli/tests/e2e_nau88l21_codec.rs). Bare registers, no SDK.
 *
 * It uses the codec's control protocol (NAU88L21 datasheet Rev 3.3, §7):
 * 7-bit address 0x54 (CSB high), 16-bit register address, 16-bit data, both
 * MSB first; a read is the address write, a repeated START with R/W=1 and
 * the data word. The LPI2C master is at 0x403F0000 (i.MX RT LPI2C1).
 *
 *   read  R58 (I2C_DEVICE_ID)
 *   write R00 = 0x0000 (reset), R01 = 0x0FFF, R1C = 0x000E, R1D = 0x0000
 *   read  R1C back
 *
 * Results in RAM:
 *   0x20000000  0xC0DEC0DE when the sequence is done
 *   0x20000004  R58 as read (0xFFFF when the read was not acknowledged)
 *   0x20000008  R1C as read back (0xFFFF when not acknowledged)
 *   0x2000000C  number of transfers that got a NACK
 *
 * Rebuild with ./build.sh (arm-none-eabi binutils).
 */
    .syntax unified
    .cpu cortex-m7
    .thumb

    .equ MCR,    0x10
    .equ MSR,    0x14
    .equ MCCR0,  0x48
    .equ MFSR,   0x5C
    .equ MTDR,   0x60
    .equ MRDR,   0x70
    .equ NDF,    0x400
    .equ SDF,    0x200
    .equ START_W, 0x4A8          /* CMD 100: START + (0x54 << 1) | W */
    .equ START_R, 0x4A9          /* CMD 100: START + (0x54 << 1) | R */
    .equ RECV_2,  0x101          /* CMD 001: receive DATA + 1 = 2 bytes */
    .equ STOP,    0x200          /* CMD 010: STOP */

    .section .vectors, "a"
    .word 0x20008000            /* initial SP */
    .word reset + 1

    .text
    .global reset
    .thumb_func
reset:
    ldr   r7, =0x403F0000       /* LPI2C1 */
    ldr   r6, =0x20000000       /* results */
    movs  r0, #0
    str   r0, [r6, #0x0C]       /* nacks = 0 */
    ldr   r0, =0x00000F0F       /* MCCR0: CLKLO = CLKHI = 15 */
    str   r0, [r7, #MCCR0]
    movs  r0, #1                /* MCR.MEN */
    str   r0, [r7, #MCR]

    movs  r0, #0x58
    bl    read_reg
    str   r0, [r6, #4]

    movs  r0, #0x00
    movs  r1, #0
    bl    write_reg
    movs  r0, #0x01
    ldr   r1, =0x0FFF
    bl    write_reg
    movs  r0, #0x1C
    movs  r1, #0x0E
    bl    write_reg
    movs  r0, #0x1D
    movs  r1, #0
    bl    write_reg

    movs  r0, #0x1C
    bl    read_reg
    str   r0, [r6, #8]

    ldr   r0, =0xC0DEC0DE
    str   r0, [r6]
spin:
    b     spin

/* Push command r0 into MTDR when the 4-entry FIFO has room. After a NACK
 * the master stops executing, so the command is dropped (wait_done cleans
 * up). Uses r3. */
    .thumb_func
push:
1:  ldr   r3, [r7, #MSR]
    tst   r3, #NDF
    bne   2f
    ldr   r3, [r7, #MFSR]
    and   r3, r3, #7
    cmp   r3, #4
    bhs   1b
    str   r0, [r7, #MTDR]
2:  bx    lr

/* Wait for the STOP. r0 = 0 on success, 1 after a NACK (the FIFOs and the
 * flags are cleared and the NACK is counted). Uses r3. */
    .thumb_func
wait_done:
1:  ldr   r3, [r7, #MSR]
    tst   r3, #NDF
    bne   2f
    tst   r3, #SDF
    beq   1b
    ldr   r3, =0x300            /* w1c SDF | EPF */
    str   r3, [r7, #MSR]
    movs  r0, #0
    bx    lr
2:  ldr   r3, =0x301            /* MEN | RTF | RRF: flush the FIFOs */
    str   r3, [r7, #MCR]
    ldr   r3, =0x700            /* w1c NDF | SDF | EPF */
    str   r3, [r7, #MSR]
    ldr   r3, [r6, #0x0C]
    adds  r3, r3, #1
    str   r3, [r6, #0x0C]
    movs  r0, #1
    bx    lr

/* write_reg(r0 = register, r1 = value) */
    .thumb_func
write_reg:
    push  {r4, r5, lr}
    mov   r4, r0
    mov   r5, r1
    ldr   r0, =START_W
    bl    push
    lsrs  r0, r4, #8
    bl    push
    uxtb  r0, r4
    bl    push
    lsrs  r0, r5, #8
    bl    push
    uxtb  r0, r5
    bl    push
    ldr   r0, =STOP
    bl    push
    bl    wait_done
    pop   {r4, r5, pc}

/* read_reg(r0 = register) -> r0 = value, or 0xFFFF after a NACK */
    .thumb_func
read_reg:
    push  {r4, lr}
    mov   r4, r0
    ldr   r0, =START_W
    bl    push
    lsrs  r0, r4, #8
    bl    push
    uxtb  r0, r4
    bl    push
    ldr   r0, =START_R
    bl    push
    ldr   r0, =RECV_2
    bl    push
    ldr   r0, =STOP
    bl    push
    bl    wait_done
    cbnz  r0, 1f
    ldr   r0, [r7, #MRDR]
    uxtb  r0, r0
    ldr   r1, [r7, #MRDR]
    uxtb  r1, r1
    orr   r0, r1, r0, lsl #8
    pop   {r4, pc}
1:  ldr   r0, =0xFFFF
    pop   {r4, pc}
    .ltorg
