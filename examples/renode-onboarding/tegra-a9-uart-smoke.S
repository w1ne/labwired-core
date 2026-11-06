/*
 * Bounded NVIDIA Tegra 2/3 import smoke: one Cortex-A9 PE executes from
 * SDRAM at 0x00000000 and writes a banner to UARTA (16550, 4-byte register
 * stride) at 0x70006000, polling LSR.THRE before each byte, then spins.
 * No boot ROM, AVP, other PEs, SCU, GIC, timers, clock/reset or pinmux
 * setup is exercised.
 */
.syntax unified
.cpu cortex-a9
.arm

.equ UART_THR, 0x00
.equ UART_LSR, 0x14
.equ LSR_THRE, 1 << 5

.section .text.start, "ax", %progbits
.global _start
.type _start, %function
_start:
    ldr r4, =message
    ldr r6, =0x70006000
1:
    ldrb r5, [r4], #1
    cmp r5, #0
    beq 3f
2:
    ldr r7, [r6, #UART_LSR]
    tst r7, #LSR_THRE
    beq 2b
    str r5, [r6, #UART_THR]
    b 1b
3:
    b 3b
.size _start, . - _start

.section .rodata, "a", %progbits
message:
    .asciz "TEGRA A9 UART SMOKE\n"
