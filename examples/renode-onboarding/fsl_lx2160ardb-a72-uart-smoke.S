/*
 * AArch64 UART smoke for the imported NXP LX2160A-RDB profile. One
 * Cortex-A72 PE enables UART1 (PL011, the board console) at 0x021C0000,
 * polls UARTFR.TXFF before each byte and writes a banner to UARTDR, then
 * spins in an integer loop. No GIC, timer, clock or pin-mux setup is
 * exercised.
 */
.equ UART_DR, 0x00
.equ UART_FR, 0x18
.equ UART_CR, 0x30
.equ FR_TXFF, 5
.equ CR_UARTEN_TXE, 0x101

.section .text.start, "ax"
.global _start
.type _start, %function

_start:
    movz    x3, #0x021C, lsl #16
    mov     w6, #CR_UARTEN_TXE
    str     w6, [x3, #UART_CR]
    adr     x4, banner
1:
    ldrb    w5, [x4], #1
    cbz     w5, 3f
2:
    ldr     w6, [x3, #UART_FR]
    tbnz    w6, #FR_TXFF, 2b
    str     w5, [x3, #UART_DR]
    b       1b
3:
    mov     x0, #0x123
    mov     x1, #0x321
4:
    eor     x2, x0, x1
    add     x0, x2, #1
    eor     x1, x2, x0
    b       4b

.section .rodata
banner:
    .asciz "LX2160A A72 UART SMOKE\n"
