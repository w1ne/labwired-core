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

/* ── TWIM21 — I²C master with EasyDMA (DT twi21 @ 0xc7000) ────────────────
 *
 * The smart-ring's four I²C sensors (BMI270 0x68, MAX30102 0x57, TMP117 0x48,
 * DRV2605 0x5A) all hang off TWIM21. As with UARTE20 this is the nRF54L
 * generation layout, NOT the nRF52 TWIM at a new base: EasyDMA moved into a
 * DMA.{RX,TX} cluster and the task/event surface was renumbered.
 *
 *   function            nRF52   nRF54L15
 *   start TX            0x008   0x050  TASKS_DMA.TX.START
 *   start RX            0x000   0x028  TASKS_DMA.RX.START
 *   STOP task           0x014   0x004  TASKS_STOP
 *   STOPPED event       0x104   0x104  EVENTS_STOPPED (coincides)
 *   ERROR event         0x124   0x114  EVENTS_ERROR
 *   RX complete         0x15C   0x14C  EVENTS_DMA.RX.END
 *   TX complete         0x160   0x168  EVENTS_DMA.TX.END
 *   SHORTS              0x200   0x200  (bit positions unchanged)
 *   ERRORSRC            0x4C4   0x4C4
 *   ENABLE              0x500   0x500  (value 6, coincides)
 *   FREQUENCY           0x524   0x524
 *   ADDRESS             0x588   0x588  (coincides)
 *   PSEL.SCL            0x508   0x508
 *   PSEL.SDA            0x50C   0x50C
 *   RX pointer          0x534   0x704  DMA.RX.PTR
 *   RX length           0x538   0x708  DMA.RX.MAXCNT
 *   TX pointer          0x544   0x73C  DMA.TX.PTR
 *   TX length           0x548   0x740  DMA.TX.MAXCNT
 *
 * Offsets taken from the nRF54L TWIM model (Nordic MDK SVD GLOBAL_TWIM21_S),
 * matching crates/core/src/peripherals/nrf54l/twim.rs.
 */
#define TWIM21_BASE                 0x500C7000UL
#define TWIM_TASKS_STOP(b)          REG32((b) + 0x004)
#define TWIM_TASKS_DMA_RX_START(b)  REG32((b) + 0x028)
#define TWIM_TASKS_DMA_TX_START(b)  REG32((b) + 0x050)
#define TWIM_EVENTS_STOPPED(b)      REG32((b) + 0x104)
#define TWIM_EVENTS_ERROR(b)        REG32((b) + 0x114)
#define TWIM_EVENTS_DMA_RX_END(b)   REG32((b) + 0x14C)
#define TWIM_EVENTS_DMA_TX_END(b)   REG32((b) + 0x168)
#define TWIM_SHORTS(b)              REG32((b) + 0x200)
#define TWIM_ERRORSRC(b)            REG32((b) + 0x4C4)
#define TWIM_ENABLE(b)              REG32((b) + 0x500)
#define TWIM_PSEL_SCL(b)            REG32((b) + 0x508)
#define TWIM_PSEL_SDA(b)            REG32((b) + 0x50C)
#define TWIM_FREQUENCY(b)           REG32((b) + 0x524)
#define TWIM_ADDRESS(b)             REG32((b) + 0x588)
#define TWIM_DMA_RX_PTR(b)          REG32((b) + 0x704)
#define TWIM_DMA_RX_MAXCNT(b)       REG32((b) + 0x708)
#define TWIM_DMA_TX_PTR(b)          REG32((b) + 0x73C)
#define TWIM_DMA_TX_MAXCNT(b)       REG32((b) + 0x740)

#define TWIM_ENABLE_ENABLED         6u
#define TWIM_FREQUENCY_K400         0x06400000u

/* SHORTS bit positions — unchanged from nRF52. The canonical register read is
 * write-pointer (TX) -> repeated START -> read (RX) -> STOP, driven entirely by
 * shorts so the CPU never has to intervene between the two legs. */
#define TWIM_SHORT_LASTTX_DMA_RX_START (1u << 7)
#define TWIM_SHORT_LASTRX_STOP         (1u << 12)

/* ERRORSRC: bit1 = ANACK (address NACK — an unpopulated bus address). */
#define TWIM_ERRORSRC_ANACK            (1u << 1)

/* TWIM21 pins on the ring: SCL = P1.02, SDA = P1.03 (see smart-ring.yaml). */
#define TWIM21_PIN_SCL              PSEL(1, 2)
#define TWIM21_PIN_SDA              PSEL(1, 3)

#endif /* NRF54L15_H */
