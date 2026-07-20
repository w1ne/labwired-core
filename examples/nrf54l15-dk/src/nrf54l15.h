/*
 * Minimal nRF54L15 register definitions for the onboarding smoke firmware.
 *
 * Addresses are the SECURE alias (0x5000_0000 window) and every offset below
 * is taken from the Nordic MDK — SVD `nrf54l15_application.svd` and
 * `nrf54l15_global.h` / `nrf54l15_types.h`. NOT from the nRF52 headers.
 *
 * That distinction is the whole point of this file. The first version of this
 * firmware used nRF52 UARTE and GPIO offsets, and it "worked" — because the
 * simulator's chip profile had the same nRF52 models mapped. Firmware and
 * model agreed with each other and both disagreed with silicon, so the boot
 * test passed while proving nothing. Real Zephyr, which uses the real offsets,
 * hung immediately.
 *
 * If you extend this header, take the offset from the MDK/SVD, not from a
 * working nRF52 example.
 */
#ifndef NRF54L15_H
#define NRF54L15_H

#include <stdint.h>

#define REG32(addr) (*(volatile uint32_t *)(addr))

/* ── UARTE20 — DK console (DT uart20 @ 0xc6000) ───────────────────────────
 *
 * nRF54L-generation layout: EasyDMA lives in a DMA.{RX,TX} cluster and the
 * task/event surface is renumbered relative to nRF52.
 *
 *   function           nRF52   nRF54L15
 *   start TX           0x008   0x050  TASKS_DMA.TX.START
 *   TX complete        0x120   0x168  EVENTS_DMA.TX.END
 *   TX stopped         0x158   0x130  EVENTS_TXSTOPPED
 *   TX pointer         0x544   0x73C  DMA.TX.PTR
 *   TX length          0x548   0x740  DMA.TX.MAXCNT
 *   PSEL.TXD           0x50C   0x604
 *
 * ENABLE and BAUDRATE happen to match on both generations.
 */
#define UARTE20_BASE                0x500C6000UL
#define UARTE_TASKS_DMA_TX_START(b) REG32((b) + 0x050)
#define UARTE_TASKS_DMA_TX_STOP(b)  REG32((b) + 0x054)
#define UARTE_EVENTS_TXSTOPPED(b)   REG32((b) + 0x130)
#define UARTE_EVENTS_DMA_TX_END(b)  REG32((b) + 0x168)
#define UARTE_ENABLE(b)             REG32((b) + 0x500)
#define UARTE_BAUDRATE(b)           REG32((b) + 0x524)
#define UARTE_PSEL_TXD(b)           REG32((b) + 0x604)
#define UARTE_PSEL_RXD(b)           REG32((b) + 0x60C)
#define UARTE_DMA_TX_PTR(b)         REG32((b) + 0x73C)
#define UARTE_DMA_TX_MAXCNT(b)      REG32((b) + 0x740)
#define UARTE_DMA_TX_AMOUNT(b)      REG32((b) + 0x744)

#define UARTE_ENABLE_UARTE          8u
#define UARTE_BAUD_115200           0x01D7E000u

/*
 * PSEL encoding: bit31 = DISCONNECTED, bits 5..6 = port, bits 0..4 = pin.
 * The nRF54L15-DK routes UARTE20 to P1.04 (TX) / P1.05 (RX).
 */
#define PSEL(port, pin)             ((uint32_t)(((port) << 5) | (pin)))
#define UARTE20_PIN_TXD             PSEL(1, 4)
#define UARTE20_PIN_RXD             PSEL(1, 5)

/* ── GPIO P2 — DK LED0 is P2.09 ───────────────────────────────────────────
 *
 * MDK `NRF_P2_S_BASE = 0x50050400`, and on THIS family `NRF_GPIO_Type` puts
 * OUT at offset 0x000 with no leading reserved words. Nordic changed that
 * prefix every generation — nRF52840 has OUT at +0x504, nRF5340 at +0x004,
 * nRF54L15 at +0x000 — so an offset copied from another Nordic part is wrong
 * here even though the register names match.
 */
#define GPIO_P2_BASE                0x50050400UL
#define GPIO_OUT(b)                 REG32((b) + 0x000)
#define GPIO_OUTSET(b)              REG32((b) + 0x004)
#define GPIO_OUTCLR(b)              REG32((b) + 0x008)
#define GPIO_IN(b)                  REG32((b) + 0x00C)
#define GPIO_DIR(b)                 REG32((b) + 0x010)
#define GPIO_DIRSET(b)              REG32((b) + 0x014)
#define LED0_PIN                    9u

#endif /* NRF54L15_H */
