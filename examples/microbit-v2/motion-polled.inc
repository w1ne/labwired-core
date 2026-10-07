/* SPDX-License-Identifier: MIT
 * Original polled LSM303AGR driver proof, no vendor firmware.
 * TWIM0 on SCL=P0.08/SDA=P0.16. Actual EasyDMA buffers are guest RAM.
 * 20000100: identities (two bytes), +04/+08 sample counters, +0c errors,
 * +10/+18 signed XYZ words, +24/+28 TX/RX amount receipts, +2c polls.
 * r8=TWIM, r9=TX buffer, r10=state; preserve the display's low registers.
 */
.align 2
.thumb_func
motion_init:
    push {r0-r6,lr}
    ldr r8, =0x40003000
    ldr r9, =0x20000200
    ldr r10, =0x20000100
    movs r0, #8
    str r0, [r8, #0x508]
    movs r0, #16
    str r0, [r8, #0x50c]
    ldr r0, =0x06400000 /* 400kHz */
    str r0, [r8, #0x524]
    movs r0, #6
    str r0, [r8, #0x500]
    movs r0, #0x19
    movs r1, #0x0f
    movs r2, #1
    mov r3, r10
    bl motion_read
    movs r0, #0x1e
    movs r1, #0x4f
    add r3, r10, #1
    bl motion_read
    movs r0, #0x19
    movs r1, #0x23 /* BDU, normal 10-bit, +/-2g */
    movs r2, #0x80
    bl motion_write
    movs r1, #0x20 /* 100Hz, XYZ enabled */
    movs r2, #0x57
    bl motion_write
    movs r0, #0x1e
    movs r1, #0x62 /* Magnetometer BDU; GPIO IRQ is deliberately not enabled. */
    movs r2, #0x10
    bl motion_write
    movs r1, #0x60 /* 100Hz continuous; required COMP_TEMP_EN (ST Rev11 8.39). */
    movs r2, #0x8c
    bl motion_write
    pop {r0-r6,pc}

.thumb_func
motion_poll:
    push {r0-r6,lr}
    ldr r4, [r10, #0x2c]
    adds r4, #1
    str r4, [r10, #0x2c]
    movs r0, #0x19
    movs r1, #0x27
    movs r2, #1
    add r3, r10, #0x30
    bl motion_read
    ldrb r4, [r3]
    tst r4, #8
    beq motion_mag
    movs r1, #0xa8 /* SUB7 auto-increment, actual six-byte XYZ burst */
    movs r2, #6
    add r3, r10, #0x10
    bl motion_read
    ldr r4, [r8, #0x54c]
    ldr r5, [r8, #0x53c]
    orr r4, r4, r5, lsl #16
    str r4, [r10, #0x24]
    ldr r4, [r10, #4]
    adds r4, #1
    str r4, [r10, #4]
motion_mag:
    movs r0, #0x1e
    movs r1, #0x67
    movs r2, #1
    add r3, r10, #0x34
    bl motion_read
    ldrb r4, [r3]
    tst r4, #8
    beq motion_poll_done
    movs r1, #0xe8
    movs r2, #6
    add r3, r10, #0x18
    bl motion_read
    ldr r4, [r8, #0x54c]
    ldr r5, [r8, #0x53c]
    orr r4, r4, r5, lsl #16
    str r4, [r10, #0x28]
    ldr r4, [r10, #8]
    adds r4, #1
    str r4, [r10, #8]
motion_poll_done:
    pop {r0-r6,pc}

/* r0=slave,r1=register,r2=value. */
.thumb_func
motion_write:
    push {r0-r6,lr}
    str r0, [r8, #0x588]
    strb r1, [r9]
    strb r2, [r9, #1]
    str r9, [r8, #0x544]
    movs r4, #2
    str r4, [r8, #0x548]
    movw r4, #0x200 /* LASTTX -> STOP */
    str r4, [r8, #0x200]
    bl motion_clear_events
    movs r4, #1
    str r4, [r8, #8]
    bl motion_wait
    pop {r0-r6,pc}

/* r0=slave,r1=SUB,r2=length,r3=guest RAM destination. */
.thumb_func
motion_read:
    push {r0-r6,lr}
    str r0, [r8, #0x588]
    strb r1, [r9]
    str r9, [r8, #0x544]
    movs r4, #1
    str r4, [r8, #0x548]
    str r3, [r8, #0x534]
    str r2, [r8, #0x538]
    movw r4, #0x1080 /* LASTTX -> STARTRX; LASTRX -> STOP */
    str r4, [r8, #0x200]
    bl motion_clear_events
    movs r4, #1
    str r4, [r8, #8]
    bl motion_wait
    pop {r0-r6,pc}

.thumb_func
motion_clear_events:
    movs r4, #0
    str r4, [r8, #0x104]
    str r4, [r8, #0x124]
    movs r4, #7
    str r4, [r8, #0x4c4] /* ERRORSRC W1C */
    bx lr

/* Bounded wait. Errors/timeouts are persistent guest receipts, never ignored. */
.thumb_func
motion_wait:
    push {r4-r6,lr}
    movw r4, #60000
motion_wait_loop:
    ldr r5, [r8, #0x124]
    cbnz r5, motion_bus_error
    ldr r5, [r8, #0x104]
    cbnz r5, motion_wait_done
    subs r4, #1
    bne motion_wait_loop
    ldr r6, [r10, #0x0c]
    orr r6, r6, #0x80000000
    b motion_record_error
motion_bus_error:
    ldr r5, [r8, #0x4c4]
    ldr r6, [r10, #0x0c]
    orrs r6, r5
    orr r6, r6, #0x10000
motion_record_error:
    str r6, [r10, #0x0c]
    movs r5, #1
    str r5, [r8, #0x14]
motion_wait_done:
    pop {r4-r6,pc}
.ltorg
