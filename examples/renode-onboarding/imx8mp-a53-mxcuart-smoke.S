/*
 * AArch64 i.MX UART smoke for the imported NXP i.MX 8M Plus profiles. One
 * Cortex-A53 PE checks that USR1 reads TRDY set and RRDY clear after reset
 * (i.MX 8M Plus RM, UARTx_USR1 reset 0x2040), enables the transmitter and
 * receiver (UCR2 SRST|RXEN|TXEN|WS|IRTS, UCR1 UARTEN), writes a banner to UTXD
 * polling USR1.TRDY, then spins in an integer loop. A window answering zeros
 * fails the USR1 check and prints the FAIL banner.
 *
 * UART_BASE is chosen at assembly time (--defsym): 0x30890000 (UART2, the
 * console in Renode's nxp-imx8mplus_linux.resc) for nxp-mimx8ml and
 * 0x30A60000 (UART4, the console in nxp-imx8mplus_zephyr.resc) for
 * nxp-frdm-imx8mplus. No GIC, timer, clock or pin-mux block is exercised.
 */
.equ UTXD, 0x40
.equ UCR1, 0x80
.equ UCR2, 0x84
.equ USR1, 0x94
.equ USR1_TRDY, 13
.equ USR1_CHECK_MASK, 0x2200            /* TRDY | RRDY */
.equ USR1_CHECK_IDLE, 0x2000            /* TRDY set, RRDY clear */

.section .text.start, "ax"
.global _start
.type _start, %function

_start:
    movz    x3, #(UART_BASE >> 16), lsl #16
    adr     x4, banner_ok
    ldr     w6, [x3, #USR1]
    mov     w8, #USR1_CHECK_MASK
    and     w6, w6, w8
    mov     w8, #USR1_CHECK_IDLE
    cmp     w6, w8
    b.eq    0f
    adr     x4, banner_fail
0:
    mov     w7, #0x4027             /* IRTS | WS | TXEN | RXEN | SRST(n) */
    str     w7, [x3, #UCR2]
    mov     w7, #1                  /* UARTEN */
    str     w7, [x3, #UCR1]
1:
    ldrb    w5, [x4], #1
    cbz     w5, 3f
2:
    ldr     w6, [x3, #USR1]
    tbz     w6, #USR1_TRDY, 2b
    str     w5, [x3, #UTXD]
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
banner_ok:
    .asciz "IMX8MP A53 MXCUART SMOKE USR1=OK\n"
banner_fail:
    .asciz "IMX8MP A53 MXCUART SMOKE USR1=BAD\n"
