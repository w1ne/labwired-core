/*
 * AArch64 UART smoke for the imported Zynq UltraScale+ MPSoC profiles.
 * One Cortex-A53 PE writes a banner to PS UART0 (Cadence, UG1087) at
 * 0xFF000000, polling Channel_sts.TXFULL before each byte, then spins in
 * an integer loop. No GIC, timer, clock or MIO setup is exercised.
 */
.equ UART0_BASE, 0xFF000000
.equ UART_SR, 0x2C
.equ UART_FIFO, 0x30
.equ SR_TXFULL, 4

.section .text.start, "ax"
.global _start
.type _start, %function

_start:
    movz    x3, #0xFF00, lsl #16
    adr     x4, banner
1:
    ldrb    w5, [x4], #1
    cbz     w5, 3f
2:
    ldr     w6, [x3, #UART_SR]
    tbnz    w6, #SR_TXFULL, 2b
    str     w5, [x3, #UART_FIFO]
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
    .asciz "ZYNQMP A53 UART SMOKE\n"
