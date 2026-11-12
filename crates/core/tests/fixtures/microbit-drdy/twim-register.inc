/* SPDX-License-Identifier: MIT
 * Source-built TWIM0 register transfers shared by the DRDY guests.
 * r8=TWIM, r9=TX buffer, r10=receipt RAM. No status-polling sample loop.
 */
.align 2
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
