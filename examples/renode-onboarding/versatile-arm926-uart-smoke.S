/*
 * Bounded Versatile/PB import smoke: one ARM926EJ-S
 * executes from SDRAM at 0x00000000, enables UART0 (PL011) at
 * 0x101F1000, polls UARTFR.TXFF before each byte and writes a banner to
 * UARTDR, then spins. No VIC, SIC, timers, system controller or other
 * peripheral is exercised.
 */
.syntax unified
.cpu arm926ej-s
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
    ldr r4, =message
    ldr r6, =0x101F1000
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
.size _start, . - _start

.section .rodata, "a", %progbits
message:
    .asciz "VERSATILE ARM926 UART SMOKE\n"
