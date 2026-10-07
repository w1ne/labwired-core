/* SPDX-License-Identifier: MIT */
/* Original Cortex-M4 board proof; no Nordic/CODAL/MakeCode firmware copied.
 * Scan a diagonal across all five rows/columns using silicon P0/P1 addresses.
 * RAM20000000 = current pressed mask (A=1,B=2), RAM20000004 = scan count.
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
    ldr r0, =0x50000000
    ldr r1, =0x50000300
    ldr r2, =((1<<21)|(1<<22)|(1<<15)|(1<<24)|(1<<19)|(1<<28)|(1<<11)|(1<<31)|(1<<30))
    str r2, [r0, #0x518]
    movs r2, #32
    str r2, [r1, #0x518]
    /* A/B inputs connected, pull-up; keep them separate from row outputs. */
    movs r2, #12
    str r2, [r0, #0x738]
    str r2, [r0, #0x75c]
    ldr r7, =0x20000000
    movs r2, #0
    str r2, [r7]
    str r2, [r7, #4]
#ifdef MICROBIT_MOTION_IO
    bl motion_init
#endif
scan:
#ifdef MICROBIT_MOTION_IO
    bl motion_poll
#endif
    ldr r4, =row_masks
    ldr r5, =col_masks
    movs r6, #0
row:
    ldr r2, =((1<<21)|(1<<22)|(1<<15)|(1<<24)|(1<<19))
    str r2, [r0, #0x50c]
    ldr r2, =((1<<28)|(1<<11)|(1<<31)|(1<<30))
    str r2, [r0, #0x508]
    movs r2, #32
    str r2, [r1, #0x508]
    cmp r6, #3
    bne col0
    str r2, [r1, #0x50c] /* column4=P1.05; real OUTCLR=5000080c */
    b select_row
col0:
    ldr r2, [r5, r6, lsl #2]
    str r2, [r0, #0x50c]
select_row:
    ldr r2, [r4, r6, lsl #2]
    str r2, [r0, #0x508]
    ldr r2, [r0, #0x510]
    mvns r2, r2
    lsrs r3, r2, #14
    and r3, r3, #1
    lsrs r2, r2, #22
    and r2, r2, #2
    orrs r2, r3
    str r2, [r7]
    /* Real guest delay, not a terminal spin or host-authored pixel fixture. */
    movw r2, #2048
delay:
    subs r2, #1
    bne delay
    adds r6, #1
    cmp r6, #5
    bne row
    ldr r2, [r7, #4]
    adds r2, #1
    str r2, [r7, #4]
    b scan
.align 2
row_masks: .word (1<<21),(1<<22),(1<<15),(1<<24),(1<<19)
col_masks: .word (1<<28),(1<<11),(1<<31),0,(1<<30)
#ifdef MICROBIT_MOTION_IO
#include "motion-polled.inc"
#endif
