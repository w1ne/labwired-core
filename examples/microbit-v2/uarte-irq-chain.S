/* SPDX-License-Identifier: MIT */
/* Original interrupt-chain regression, not a CODAL implementation.
 * ENDTX queues the next byte before this same handler inspects TXSTOPPED.
 * An unsolicited stop must not mark that new transfer idle. The final byte
 * explicitly requests STOPTX and requires its real TXSTOPPED notification.
 * State: done=1/pass,2/unsolicited-stop,3/timeout; sent; unsolicited stops;
 * busy; next message index; explicit stop requested.
 */
.syntax unified
.cpu cortex-m4
.thumb
.section .vectors,"a"
.word 0x20020000
.word reset
.space 16 * 4
.word uarte_irq /* external interrupt 2 */
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
    str r0, [r7, #12]
    str r0, [r7, #16]
    str r0, [r7, #20]
    movs r0, #8
    str r0, [r5, #0x500]
    movs r0, #6
    str r0, [r5, #0x50c]
    ldr r0, =0x10000000 /* 1 Mbaud */
    str r0, [r5, #0x524]
    ldr r0, =0x00400100 /* ENDTX + TXSTOPPED interrupts */
    str r0, [r5, #0x304]
    ldr r1, =0xe000e100 /* NVIC ISER0 */
    movs r0, #4
    str r0, [r1]
    cpsie i
    bl send_next
    ldr r6, =100000
wait_done:
    ldr r0, [r7]
    cmp r0, #0
    bne parked
    subs r6, #1
    bne wait_done
    movs r0, #3
    str r0, [r7]
parked:
    b parked

.thumb_func
send_next:
    push {r4, lr}
    ldr r4, =message
    ldr r1, [r7, #16]
    ldrb r0, [r4, r1]
    cmp r0, #0
    beq stop_requested
    adds r1, #1
    str r1, [r7, #16]
    movs r1, #1
    str r1, [r7, #12]
    sub sp, #8
    strb r0, [sp]
    mov r0, sp
    str r0, [r5, #0x544]
    str r1, [r5, #0x548]
    str r1, [r5, #0x008]
    add sp, #8
    pop {r4, pc}
stop_requested:
    movs r0, #1
    str r0, [r7, #20]
    str r0, [r5, #0x00c]
    pop {r4, pc}

.thumb_func
uarte_irq:
    push {r4, lr}
    ldr r0, [r5, #0x120]
    cmp r0, #0
    beq check_stop
    movs r0, #0
    str r0, [r5, #0x120]
    ldr r0, [r7, #4]
    adds r0, #1
    str r0, [r7, #4]
    bl send_next
check_stop:
    ldr r0, [r5, #0x158]
    cmp r0, #0
    beq irq_return
    movs r0, #0
    str r0, [r5, #0x158]
    str r0, [r7, #12]
    ldr r0, [r7, #20]
    cmp r0, #1
    beq expected_stop
    ldr r0, [r7, #8]
    adds r0, #1
    str r0, [r7, #8]
    movs r0, #2
    str r0, [r7]
    b irq_return
expected_stop:
    movs r0, #1
    str r0, [r7]
irq_return:
    pop {r4, pc}

.section .rodata,"a"
message:
    .asciz "A10 IRQ chain OK\n"
