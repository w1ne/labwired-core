/*
 * Minimal nRF54L15 register definitions for the onboarding smoke firmware.
 *
 * Addresses are the SECURE alias (0x5000_0000 window), matching
 * configs/chips/nrf54l15.yaml and the Zephyr devicetree default. Only what the
 * smoke path touches is declared here — this is deliberately not a CMSIS
 * header, because a hand-checked 30-line subset is easier to audit against the
 * chip profile than a generated 20,000-line one.
 */
#ifndef NRF54L15_H
#define NRF54L15_H

#include <stdint.h>

#define REG32(addr) (*(volatile uint32_t *)(addr))

/* ── UARTE20 — the DK console (DT: uart20 @ 0xc6000, +0x50000000) ────────── */
#define UARTE20_BASE            0x500C6000UL

#define UARTE_TASKS_STARTTX(b)  REG32((b) + 0x008)
#define UARTE_TASKS_STOPTX(b)   REG32((b) + 0x00C)
#define UARTE_EVENTS_ENDTX(b)   REG32((b) + 0x120)
#define UARTE_EVENTS_TXSTARTED(b) REG32((b) + 0x150)
#define UARTE_ENABLE(b)         REG32((b) + 0x500)
#define UARTE_PSEL_TXD(b)       REG32((b) + 0x50C)
#define UARTE_PSEL_RXD(b)       REG32((b) + 0x514)
#define UARTE_BAUDRATE(b)       REG32((b) + 0x524)
#define UARTE_TXD_PTR(b)        REG32((b) + 0x544)
#define UARTE_TXD_MAXCNT(b)     REG32((b) + 0x548)

#define UARTE_ENABLE_UARTE      8u
#define UARTE_BAUD_115200       0x01D7E000u

/*
 * PSEL encoding: bit31 = DISCONNECTED, bits 5..6 = port, bits 0..4 = pin.
 * The nRF54L15-DK routes UARTE20 to P1.04 (TX) / P1.05 (RX).
 */
#define PSEL(port, pin)         ((uint32_t)(((port) << 5) | (pin)))
#define UARTE20_PIN_TXD         PSEL(1, 4)
#define UARTE20_PIN_RXD         PSEL(1, 5)

/* ── GPIO P2 — DK LED0 is P2.09 (DT: gpio2 @ 0x50400, +0x50000000) ───────── */
#define GPIO_P2_BASE            0x50050400UL
#define GPIO_OUTSET(b)          REG32((b) + 0x008)
#define GPIO_OUTCLR(b)          REG32((b) + 0x00C)
#define GPIO_DIRSET(b)          REG32((b) + 0x518)
#define LED0_PIN                9u

#endif /* NRF54L15_H */
