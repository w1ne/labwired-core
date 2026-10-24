/* SPDX-License-Identifier: MIT */
/* Original bounded Cortex-M4 guest; no Nordic, CODAL or MakeCode code copied.
 * putc hands UARTE a byte in its stack frame, returns, and its caller reuses
 * exactly that slot before polling ENDTX. Completion must retain the byte.
 * RAM 20000100 = done (1) / timeout (2), +4 = sent, +8 = reused slots.
 */
.syntax unified
.cpu cortex-m4
.thumb
.section .vectors,"a"
.word 0x20020000
.word reset
.section .text,"ax"
.thumb_func
.global reset
reset:
    ldr r5, =0x40002000
    ldr r7, =0x20000100
    movs r0, #0
    str r0, [r7]
    str r0, [r7, #4]
    str r0, [r7, #8]
    movs r0, #8
    str r0, [r5, #0x500] /* ENABLE: UARTE */
    movs r0, #6
    str r0, [r5, #0x50c] /* PSEL.TXD: P0.06 */
    ldr r0, =0x10000000
    str r0, [r5, #0x524] /* BAUDRATE: 1 Mbaud */
    ldr r4, =message
    movs r6, #0
next_byte:
    ldrb r0, [r4], #1
    cmp r0, #0
    beq finished
    bl stack_putc
    bl reuse_slot
    ldr r3, =10000
wait_endtx:
    ldr r1, [r5, #0x120]
    cmp r1, #0
    bne sent
    subs r3, #1
    bne wait_endtx
    movs r0, #2
    str r0, [r7]
    b stopped
sent:
    adds r6, #1
    str r6, [r7, #4]
    b next_byte
finished:
    movs r0, #1
    str r0, [r7]
stopped:
    b stopped

.thumb_func
stack_putc:
    push {r0, lr}
    movs r1, #0
    str r1, [r5, #0x120] /* clear ENDTX */
    mov r1, sp
    str r1, [r5, #0x544] /* TXD.PTR */
    movs r1, #1
    str r1, [r5, #0x548] /* TXD.MAXCNT */
    str r1, [r5, #0x008] /* STARTTX */
    pop {r0, pc}

.thumb_func
reuse_slot:
    sub sp, #8
    movs r1, #'?'
    strb r1, [sp]
    add sp, #8
    ldr r1, [r7, #8]
    adds r1, #1
    str r1, [r7, #8]
    bx lr

.section .rodata,"a"
message:
    .asciz "A10 stack reuse OK\n"
