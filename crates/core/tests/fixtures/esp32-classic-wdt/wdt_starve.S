/* Classic ESP32 Main System Watchdog (TIMG0 MWDT) — starved fixture.
 *
 * Register-level: no ROM, no IDF, no interrupt vector. The program arms
 * TIMG0's watchdog with a short stage-0 hold and an interrupt action, masks
 * maskable CPU interrupts with `rsil` (so the DPORT line is asserted but not
 * taken — the point here is the latch, not CPU entry), and polls
 * INT_RAW_TIMERS until bit 2 (WDT_INT_RAW) is set. It then writes
 * INT_CLR_TIMERS bit 2 and re-reads INT_RAW to prove the W1C clear.
 *
 * Offsets (ESP32 TRM v4.6 §16.4, esp-idf
 * components/soc/esp32/register/soc/timer_group_reg.h):
 *   TIMG0 base            0x3FF5_F000
 *   WDTCONFIG0  +0x48     EN bit31, STG0 [30:29], LEVEL_INT_EN bit21
 *   WDTCONFIG2  +0x50     STG0_HOLD
 *   WDTFEED     +0x60     write any value to feed
 *   WDTWPROTECT +0x64     WDT_WKEY = 0x50D83AA1
 *   INT_ENA     +0x98     bit2 = WDT
 *   INT_RAW     +0x9C     bit2 = WDT
 *   INT_CLR     +0xA4     bit2 = WDT (W1C)
 *
 * Console: UART0 at 0x3FF4_0000, one byte per store to offset 0.
 */
    .section .data
s_arm:    .ascii "WDT_ARM\r\n"
s_fire:   .ascii "WDT_FIRED\r\n"
s_clear:  .ascii "WDT_CLEARED\r\n"
s_fail:   .ascii "WDT_CLR_FAIL\r\n"
s_done:   .ascii "WDT_DONE\r\n"

    .section .text
    .literal_position
    .global _start
    .align 4
_start:
    movi    a2, 0x3FF40000          /* UART0 FIFO */
    movi    a3, 0x3FF5F000          /* TIMG0 */

    movi    a4, s_arm
    movi    a6, 9
1:  l8ui    a7, a4, 0
    s32i    a7, a2, 0
    addi    a4, a4, 1
    addi    a6, a6, -1
    bnez    a6, 1b

    rsil    a5, 15                  /* mask maskable interrupts: poll only */

    movi    a4, 0x50D83AA1          /* WDT_WKEY: unlock */
    s32i    a4, a3, 0x64
    movi    a4, 16                  /* STG0_HOLD, in APB (80 MHz) cycles */
    s32i    a4, a3, 0x50
    movi    a4, 0xA0200000          /* EN | STG0=1 (interrupt) | LEVEL_INT_EN */
    s32i    a4, a3, 0x48
    movi    a4, 4                   /* INT_ENA_TIMERS.WDT_INT_ENA (bit 2) */
    s32i    a4, a3, 0x98

2:  l32i    a5, a3, 0x9C            /* INT_RAW_TIMERS */
    extui   a5, a5, 2, 1
    beqz    a5, 2b

    movi    a4, s_fire
    movi    a6, 11
3:  l8ui    a7, a4, 0
    s32i    a7, a2, 0
    addi    a4, a4, 1
    addi    a6, a6, -1
    bnez    a6, 3b

    movi    a4, 4                   /* INT_CLR_TIMERS.WDT_INT_CLR (W1C) */
    s32i    a4, a3, 0xA4
    l32i    a5, a3, 0x9C
    extui   a5, a5, 2, 1
    bnez    a5, 4f

    movi    a4, s_clear
    movi    a6, 13
5:  l8ui    a7, a4, 0
    s32i    a7, a2, 0
    addi    a4, a4, 1
    addi    a6, a6, -1
    bnez    a6, 5b
    j       6f

4:  movi    a4, s_fail
    movi    a6, 14
7:  l8ui    a7, a4, 0
    s32i    a7, a2, 0
    addi    a4, a4, 1
    addi    a6, a6, -1
    bnez    a6, 7b

6:  movi    a4, s_done
    movi    a6, 10
8:  l8ui    a7, a4, 0
    s32i    a7, a2, 0
    addi    a4, a4, 1
    addi    a6, a6, -1
    bnez    a6, 8b

9:  j       9b
