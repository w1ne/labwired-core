/*
 * Bounded Arm Cortex-R52 FVP-style import smoke (Renode cortex-r52*.repl):
 * one Cortex-R52 PE enters at its Hyp-mode reset state, executing A32 from
 * DRAM at 0x0. It reads MIDR and spins without output unless the part number
 * is Cortex-R52 (0xD13), then enables UART0 (PL011) at 0x9C090000, polls
 * UARTFR.TXFF before each byte and writes a banner to UARTDR. No GIC,
 * generic timer, MPU programming or other UART is exercised.
 */
.syntax unified
.cpu cortex-r52
.arm

.equ UART_DR, 0x00
.equ UART_FR, 0x18
.equ UART_CR, 0x30
.equ FR_TXFF, 1 << 5
.equ CR_UARTEN_TXE, 0x101

.section .text.start, "ax", %progbits
.global _start
.type _start, %function
_start:
    mrc p15, 0, r0, c0, c0, 0      @ MIDR
    ubfx r0, r0, #4, #12           @ PartNum
    movw r1, #0xd13
    cmp r0, r1
    bne 4f
    ldr r4, =message
    ldr r6, =0x9C090000
    ldr r7, =CR_UARTEN_TXE
    str r7, [r6, #UART_CR]
1:
    ldrb r5, [r4], #1
    cmp r5, #0
    beq 3f
2:
    ldr r7, [r6, #UART_FR]
    tst r7, #FR_TXFF
    bne 2b
    str r5, [r6, #UART_DR]
    b 1b
3:
    b 3b
4:
    b 4b
.size _start, . - _start

.section .rodata, "a", %progbits
message:
    .asciz "CORTEX-R52 PL011 UART SMOKE\n"
