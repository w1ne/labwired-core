/* Classic ESP32 Main System Watchdog (TIMG0 MWDT) — fed fixture.
 *
 * Identical arming to `wdt_starve.S` (same 16-cycle stage-0 hold, same
 * interrupt action, same masked CPU interrupts), except that every loop
 * iteration writes WDTFEED (@0x60). One loop iteration is a few CPU cycles,
 * inside both the scheduler-mode hold (16 APB cycles = 48 CPU cycles) and the
 * walk-mode hold (16 peripheral tick intervals), so the chain never reaches
 * stage 0's expiry. After 20,000 iterations (tens of thousands of CPU cycles,
 * thousands of holds) the fixture reads INT_RAW_TIMERS bit 2: it must still be
 * clear.
 *
 * Prints WDT_FED_OK when no latch appeared, WDT_LATCHED when one did.
 */
    .section .data
s_arm:    .ascii "WDT_ARM\r\n"
s_ok:     .ascii "WDT_FED_OK\r\n"
s_bad:    .ascii "WDT_LATCHED\r\n"

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

    movi    a6, 20000               /* iterations */
2:  s32i    a4, a3, 0x60            /* WDTFEED: any value feeds */
    addi    a6, a6, -1
    bnez    a6, 2b

    l32i    a5, a3, 0x9C            /* INT_RAW_TIMERS */
    extui   a5, a5, 2, 1
    bnez    a5, 4f

    movi    a4, s_ok
    movi    a6, 12
5:  l8ui    a7, a4, 0
    s32i    a7, a2, 0
    addi    a4, a4, 1
    addi    a6, a6, -1
    bnez    a6, 5b
    j       6f

4:  movi    a4, s_bad
    movi    a6, 13
7:  l8ui    a7, a4, 0
    s32i    a7, a2, 0
    addi    a4, a4, 1
    addi    a6, a6, -1
    bnez    a6, 7b

6:  j       6b
