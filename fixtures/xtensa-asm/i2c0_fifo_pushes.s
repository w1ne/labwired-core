    .section .text.entry, "ax"
    .align 4
i2c0_base:        .word 0x60013000

    .align 4
    .global _start
_start:
    /* Push three bytes into I2C0's TX FIFO via the DATA register
     * (offset 0x1c) and read SR back. Per the ESP32-S3 PAC SR has
     * TXFIFO_CNT in bits[23:18], so three pushes ⇒ SR & 0x00FC0000 =
     * 3 << 18 = 0x000C_0000.
     */
    l32r    a2, i2c0_base

    movi    a3, 0xAA
    s32i    a3, a2, 0x1C            /* DATA = 0xAA */
    movi    a3, 0xBB
    s32i    a3, a2, 0x1C            /* DATA = 0xBB */
    movi    a3, 0xCC
    s32i    a3, a2, 0x1C            /* DATA = 0xCC */

    /* Capture SR into a4 — caller checks bits 18..23 == 3. */
    l32i    a4, a2, 0x08
    break   1, 15
