/*
 * Bounded Allwinner A20 import smoke: one Cortex-A7 PE executes from the
 * first DDR window and writes a banner to UART0's wide 16550 TX register.
 * No boot ROM, multicore, interrupt, baud-rate or other SoC peripheral path.
 */
.syntax unified
.cpu cortex-a7
.arm

.section .text.start, "ax", %progbits
.global _start
.type _start, %function
_start:
    ldr r4, =message
    ldr r6, =0x01C28000
1:
    ldrb r5, [r4], #1
    cmp r5, #0
    beq 2f
    str r5, [r6]
    b 1b
2:
    b 2b
.size _start, . - _start

.section .rodata, "a", %progbits
message:
    .asciz "A20 A7 UART SMOKE\n"
