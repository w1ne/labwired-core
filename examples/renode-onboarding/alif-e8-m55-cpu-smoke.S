/*
 * Minimal Cortex-M55 integer smoke for the bounded Alif Ensemble E8 profile.
 * It runs from ITCM, uses the local DTCM stack, and deliberately avoids
 * Helium, MMIO, the other CPU cores, and boot firmware.
 */
.syntax unified
.cpu cortex-m33
.thumb

.section .isr_vector, "a", %progbits
.word __stack_top
.word Reset_Handler

.section .text.Reset_Handler, "ax", %progbits
.global Reset_Handler
.type Reset_Handler, %function
.thumb_func
Reset_Handler:
    movs    r0, #0x23
    movs    r1, #0x32
1:
    eors    r2, r0, r1
    adds    r0, r2, #1
    eors    r1, r2, r0
    b       1b
