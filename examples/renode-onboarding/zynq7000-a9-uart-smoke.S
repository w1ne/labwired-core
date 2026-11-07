/*
 * Bounded Zynq-7000 import smoke: one Cortex-A9 PE executes from DDR at
 * 0x00100000 and writes a banner to PS UART1 (Cadence, UG585) at 0xE0001000,
 * polling Channel_sts.TXFULL before each byte, then spins. No OCM/BootROM,
 * second PE, SCU, GIC, timer, SLCR clock or MIO setup is exercised.
 */
.syntax unified
.cpu cortex-a9
.arm

.equ UART_SR, 0x2C
.equ UART_FIFO, 0x30
.equ SR_TXFULL, 1 << 4

.section .text.start, "ax", %progbits
.global _start
.type _start, %function
_start:
    ldr r4, =message
    ldr r6, =0xE0001000
1:
    ldrb r5, [r4], #1
    cmp r5, #0
    beq 3f
2:
    ldr r7, [r6, #UART_SR]
    tst r7, #SR_TXFULL
    bne 2b
    str r5, [r6, #UART_FIFO]
    b 1b
3:
    b 3b
.size _start, . - _start

.section .rodata, "a", %progbits
message:
    .asciz "ZYNQ7000 A9 UART SMOKE\n"
