/* SPDX-License-Identifier: MIT */
/* Original register-level scan proof, not a microphone/audio implementation.
 * Sparse CH1/CH7 select held AIN3/AIN2. Host command at 20000100 releases
 * the second scan, allowing physical inputs to change between triggers.
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
    ldr r4, =0x40007000
    ldr r5, =0x20000100
    movs r0, #1
    str r0, [r4, #0x500] /* ENABLE */
    movs r0, #2
    str r0, [r4, #0x5f0] /* 12 bits */
    movs r0, #4
    str r0, [r4, #0x520] /* CH1 PSELP=AIN3 */
    movs r0, #3
    str r0, [r4, #0x580] /* CH7 PSELP=AIN2 */
    ldr r0, =0x20000200
    str r0, [r4, #0x62c]
    movs r0, #4
    str r0, [r4, #0x630]
    movs r0, #1
    str r0, [r4, #0]
    /* These live-register edits must affect NEXT START, not this buffer. */
    ldr r0, =0x20000220
    str r0, [r4, #0x62c]
    movs r0, #2
    str r0, [r4, #0x630]
    bl sample
    ldr r0, [r4, #0x634]
    str r0, [r5, #8]
    ldr r0, [r4, #0x104]
    str r0, [r5, #0x10]
    movs r0, #1
    str r0, [r5, #4]
wait_host:
    ldr r0, [r5]
    cmp r0, #2
    bne wait_host
    bl sample
    ldr r0, [r4, #0x634]
    str r0, [r5, #0xc]
    ldr r0, [r4, #0x104]
    str r0, [r5, #0x14]
    movs r0, #0
    str r0, [r4, #0x104]
    movs r0, #1
    str r0, [r4, #0]
    ldr r0, [r4, #0x634]
    str r0, [r5, #0x1c] /* START resets AMOUNT */
    bl sample
    ldr r0, [r4, #0x634]
    str r0, [r5, #0x20]
    ldr r0, [r4, #0x104]
    str r0, [r5, #0x24]
    movs r0, #3
    str r0, [r5, #4]
halt:
    b halt
.thumb_func
sample:
    movs r0, #0
    str r0, [r4, #0x10c]
    movs r0, #1
    str r0, [r4, #4]
    movw r1, #60000
wait_result:
    ldr r0, [r4, #0x10c]
    cbnz r0, sampled
    subs r1, #1
    bne wait_result
    movs r0, #1
    str r0, [r5, #0x18]
sampled:
    bx lr
