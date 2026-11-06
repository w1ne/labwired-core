/* Minimal Cortex-M4 Thumb integer loop for the bounded Colibri VF61 profile. */
.syntax unified
.cpu cortex-m4
.thumb

.section .text.start, "ax", %progbits
.global _start
.type _start, %function
.thumb_func
_start:
    movs    r0, #0x12
    movs    r1, #0x34
1:
    eors    r0, r1
    adds    r1, r0, #1
    eors    r0, r1
    b       1b
