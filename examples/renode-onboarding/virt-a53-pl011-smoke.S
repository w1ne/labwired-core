/*
 * AArch64 PL011 smoke for the imported Renode "virt"-style Cortex-A53/A78
 * profiles (cortex-a53-gicv2/gicv3/gicv3_smp, cortex-a78, cortex_a53_console,
 * cortex_a53_virtio). One PE checks that UARTFR reads TXFE|RXFE after reset
 * (PL011 TRM DDI 0183G: FR reset 0x90), enables the UART, writes a banner to
 * UARTDR at 0x09000000 polling FR.TXFF, then spins in an integer loop.
 * A window answering zeros fails the FR check and prints the FAIL banner.
 * No GIC, generic timer, RTC or virtio device is exercised.
 */
.equ UART0_BASE, 0x09000000
.equ UARTDR, 0x00
.equ UARTFR, 0x18
.equ UARTCR, 0x30
.equ FR_TXFF, 5
.equ FR_IDLE_MASK, 0x90

.section .text.start, "ax"
.global _start
.type _start, %function

_start:
    movz    x3, #0x0900, lsl #16
    adr     x4, banner_ok
    ldr     w6, [x3, #UARTFR]
    mov     w8, #FR_IDLE_MASK
    and     w6, w6, w8
    cmp     w6, w8
    b.eq    0f
    adr     x4, banner_fail
0:
    mov     w7, #0x301              /* UARTEN | TXE | RXE */
    str     w7, [x3, #UARTCR]
1:
    ldrb    w5, [x4], #1
    cbz     w5, 3f
2:
    ldr     w6, [x3, #UARTFR]
    tbnz    w6, #FR_TXFF, 2b
    str     w5, [x3, #UARTDR]
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
    .asciz "VIRT A53 PL011 SMOKE FR=OK\n"
banner_fail:
    .asciz "VIRT A53 PL011 SMOKE FR=BAD\n"
