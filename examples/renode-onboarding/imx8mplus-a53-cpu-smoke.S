/*
 * Minimal AArch64 CPU exercise for the bounded FRDM i.MX 8M Plus profile.
 * It deliberately avoids MMIO: peripherals, secondary cores, and the board
 * boot flow are not part of this measurement.
 */
.section .text.start, "ax"
.global _start
.type _start, %function

_start:
    mov     x0, #0x123
    mov     x1, #0x321
1:
    eor     x2, x0, x1
    add     x0, x2, #1
    eor     x1, x2, x0
    b       1b
