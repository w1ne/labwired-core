/* SPDX-License-Identifier: MIT
 * Sample P0.25 and count falling-edge GPIOTE channel events in guest RAM.
 * The host performs real TWIM transfers; it never writes the input pad.
 */
.file "shared-gpiote.S"
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
    ldr r0, =0x50000764    /* GPIO0.PIN_CNF[25] */
    movs r1, #0x0c        /* input connected, internal pull-up enabled */
    str r1, [r0]
    ldr r0, =0x50000000
    ldr r1, =0x20000100
    ldr r4, =0x40006100    /* EVENTS_IN[0] */
    ldr r6, =0x40006510    /* CONFIG[0] */
    movs r2, #0
    str r2, [r4]
    ldr r2, =0x00021901   /* P0.25 Event mode, high to low */
    str r2, [r6]
    movs r3, #0
    movs r5, #0
    str r5, [r1, #8]
loop:
    ldr r2, [r0, #0x510]
    lsrs r2, r2, #25
    and r2, r2, #1
    str r2, [r1]
    adds r3, #1
    str r3, [r1, #4]
    ldr r2, [r4]
    cbz r2, no_event
    adds r5, #1
    str r5, [r1, #8]
    movs r2, #0
    str r2, [r4]
no_event:
    b loop
