/*
 * Minimal nRF54LM20A register definitions for the AMOLED Snake lab.
 *
 * Addresses are the SECURE alias (0x5000_0000 window). Every offset below is
 * taken from the Nordic MDK vendor SVD `nrf54lm20a_application.svd`, which is
 * vendored at tests/fixtures/real_world/nrf54lm20a.svd. NOT from the nRF52
 * headers, and NOT from the nRF54L15 profile.
 *
 * That distinction is the whole point of this file, and the sibling example's
 * header records why: its first version used nRF52 UARTE and GPIO offsets and
 * "worked", because the simulator's chip profile had the same nRF52 models
 * mapped. Firmware and model agreed with each other and both disagreed with
 * silicon, so the boot test passed while proving nothing.
 *
 * The same trap has a second door on this part. nRF54LM20A is NOT nRF54L15
 * with more memory: TAMPC moved 0x500DC000 -> 0x500EF000, RRAMC moved
 * 0x5004B000 -> 0x5004E000, CLOCK's IRQ moved 261 -> 270, and P1 grew from 17
 * pins to 32. Take an address from the SVD, not from the sibling.
 */
#ifndef NRF54LM20A_H
#define NRF54LM20A_H

#include <stdint.h>

#define REG32(addr) (*(volatile uint32_t *)(addr))

/* ── PSEL encoding (SVD GLOBAL_SPIM00.PSEL.SCK) ───────────────────────────
 *
 *   [4:0]  PIN
 *   [7:5]  PORT      <- THREE bits on this family, not the nRF52's one
 *   [31]   CONNECT   1 = Disconnected (and the reset value)
 *
 * The wider PORT field is what lets this part address four ports. An nRF52
 * encoding that packs the port into bit 5 alone can only ever name P0 and P1.
 */
#define PSEL(port, pin)  ((uint32_t)(((port) << 5) | (pin)))

/* ── UARTE20 — DK console (DT uart20 @ 0xc6000, P1.16 TX / P1.17 RX) ──────
 *
 * nRF54L-generation layout: EasyDMA lives in a DMA.{RX,TX} cluster and the
 * task/event surface is renumbered relative to nRF52.
 *
 *   function           nRF52   nRF54L
 *   start TX           0x008   0x050  TASKS_DMA.TX.START
 *   TX complete        0x120   0x168  EVENTS_DMA.TX.END
 *   TX pointer         0x544   0x73C  DMA.TX.PTR
 *   TX length          0x548   0x740  DMA.TX.MAXCNT
 *   PSEL.TXD           0x50C   0x604
 *
 * ENABLE and BAUDRATE happen to match on both generations, which is exactly
 * what disguises a wrong header as a working one.
 */
#define UARTE20_BASE                0x500C6000UL
#define UARTE_TASKS_DMA_TX_START(b) REG32((b) + 0x050)
#define UARTE_EVENTS_TXSTOPPED(b)   REG32((b) + 0x130)
#define UARTE_EVENTS_DMA_TX_END(b)  REG32((b) + 0x168)
#define UARTE_ENABLE(b)             REG32((b) + 0x500)
#define UARTE_BAUDRATE(b)           REG32((b) + 0x524)
#define UARTE_PSEL_TXD(b)           REG32((b) + 0x604)
#define UARTE_PSEL_RXD(b)           REG32((b) + 0x60C)
#define UARTE_DMA_TX_PTR(b)         REG32((b) + 0x73C)
#define UARTE_DMA_TX_MAXCNT(b)      REG32((b) + 0x740)

#define UARTE_ENABLE_UARTE          8u
#define UARTE_BAUD_115200           0x01D7E000u
/* DK console pins: P1.16 TX, P1.17 RX (board pinctrl uart20_default). */
#define UARTE20_PIN_TXD             PSEL(1, 16)
#define UARTE20_PIN_RXD             PSEL(1, 17)

/* ── SPIM22 — the DK's expansion-header SPI ───────────────────────────────
 *
 * Zephyr's board files label this node `nordic_expansion_spi: &spi22`, so it
 * is the bus an add-on panel actually lands on. Offsets from SVD
 * GLOBAL_SPIM22, which derives from GLOBAL_SPIM00.
 *
 *   function        nRF52   nRF54L
 *   TASKS_START     0x010   0x000     <- 0x010 is TASKS_RESUME here
 *   EVENTS_END      0x118   0x108
 *   TX pointer      0x544   0x73C     DMA.TX.PTR
 *   TX length       0x548   0x740     DMA.TX.MAXCNT
 *   bit rate        0x524   0x52C     FREQUENCY enum -> PRESCALER divisor
 *   PSEL.SCK        0x508   0x600
 *
 * DCXCNT and PSEL.DCX/CSN have no nRF52 equivalent at all: this controller
 * drives the panel's data/command line and its chip select in HARDWARE.
 */
#define SPIM22_BASE                 0x500C8000UL
#define SPIM_TASKS_START(b)         REG32((b) + 0x000)
#define SPIM_EVENTS_END(b)          REG32((b) + 0x108)
#define SPIM_ENABLE(b)              REG32((b) + 0x500)
#define SPIM_PRESCALER(b)           REG32((b) + 0x52C)
#define SPIM_CONFIG(b)              REG32((b) + 0x554)
#define SPIM_DCXCNT(b)              REG32((b) + 0x5B4)
#define SPIM_PSEL_SCK(b)            REG32((b) + 0x600)
#define SPIM_PSEL_MOSI(b)           REG32((b) + 0x604)
#define SPIM_PSEL_MISO(b)           REG32((b) + 0x608)
#define SPIM_PSEL_DCX(b)            REG32((b) + 0x60C)
#define SPIM_PSEL_CSN(b)            REG32((b) + 0x610)
#define SPIM_DMA_TX_PTR(b)          REG32((b) + 0x73C)
#define SPIM_DMA_TX_MAXCNT(b)       REG32((b) + 0x740)

#define SPIM_ENABLE_SPIM            7u
/*
 * PRESCALER is a real core-clock divisor, not the nRF52 FREQUENCY enumeration.
 * SPIM22's source is the 16 MHz PCLK and the DK's expansion header is rated to
 * 8 MHz (DT `max-frequency`), so divisor 2 is the fastest legal setting.
 */
#define SPIM_PRESCALER_DIV2         2u

/* Panel wiring on the expansion header. SCK/MOSI/MISO are the board's own
 * spi22 pinctrl; CS and DCX take the two free port-3 pins (P3.0/1/3 are the
 * bus itself, P3.2 and P3.4 are unused by any DK peripheral). */
#define PANEL_PIN_SCK               PSEL(3, 3)
#define PANEL_PIN_MOSI              PSEL(3, 0)
#define PANEL_PIN_MISO              PSEL(3, 1)
#define PANEL_PIN_CSN               PSEL(3, 2)
#define PANEL_PIN_DCX               PSEL(3, 4)

/* ── GPIO ─────────────────────────────────────────────────────────────────
 *
 * nRF54L compacted the port block. These are NOT the nRF52 offsets and they
 * are not the nRF52 offsets minus a constant either:
 *
 *   register   nRF52   nRF54L   delta
 *   OUT        0x504   0x000    0x504
 *   IN         0x510   0x00C    0x504
 *   DIR        0x514   0x010    0x504
 *   PIN_CNF    0x700   0x080    0x680   <- different delta
 *
 * PIN_CNF is the only register that configures a pull-up, so a header that
 * shifts everything by one constant reads buttons that never go low.
 */
#define P0_BASE                     0x5010A000UL
#define P1_BASE                     0x500D8200UL
#define P2_BASE                     0x50050400UL
#define P3_BASE                     0x500D8600UL

#define GPIO_OUT(b)                 REG32((b) + 0x000)
#define GPIO_OUTSET(b)              REG32((b) + 0x004)
#define GPIO_OUTCLR(b)              REG32((b) + 0x008)
#define GPIO_IN(b)                  REG32((b) + 0x00C)
#define GPIO_DIR(b)                 REG32((b) + 0x010)
#define GPIO_DIRSET(b)              REG32((b) + 0x014)
#define GPIO_PIN_CNF(b, n)          REG32((b) + 0x080 + 4u * (n))

/* PIN_CNF fields: DIR bit 0, INPUT bit 1 (0 = connect buffer), PULL bits 3:2. */
#define PIN_CNF_INPUT_PULLUP        0x0000000Cu  /* DIR=In, INPUT=Connect, PULL=Up */
#define PIN_CNF_OUTPUT              0x00000003u  /* DIR=Out, INPUT=Disconnect */

/* DK LEDs (nrf54lm20dk_common.dtsi, all active-high on P1). */
#define LED0_PIN                    22u
#define LED1_PIN                    25u
#define LED2_PIN                    27u
#define LED3_PIN                    28u

/*
 * DK buttons. All four are GPIO_PULL_UP | GPIO_ACTIVE_LOW, so a press reads
 * LOW and the pull-up must be configured through PIN_CNF or the pin floats.
 * Three are on P1 and the fourth is on P0 -- the ports are genuinely mixed.
 */
#define BTN_UP_PORT                 P1_BASE
#define BTN_UP_PIN                  26u   /* button0, P1.26 */
#define BTN_DOWN_PORT               P1_BASE
#define BTN_DOWN_PIN                9u    /* button1, P1.09 */
#define BTN_LEFT_PORT               P1_BASE
#define BTN_LEFT_PIN                8u    /* button2, P1.08 */
#define BTN_RIGHT_PORT              P0_BASE
#define BTN_RIGHT_PIN               5u    /* button3, P0.05 */

/* ── RM67162 AMOLED DCS commands ──────────────────────────────────────────*/
#define DCS_SWRESET                 0x01u
#define DCS_SLPOUT                  0x11u
#define DCS_DISPOFF                 0x28u
#define DCS_DISPON                  0x29u
#define DCS_CASET                   0x2Au
#define DCS_RASET                   0x2Bu
#define DCS_RAMWR                   0x2Cu
#define DCS_MADCTL                  0x36u
#define DCS_COLMOD                  0x3Au
#define DCS_WRDISBV                 0x51u

#define COLMOD_RGB565               0x55u

/* ── Map probes (see probe_map() in main.c) ───────────────────────────────
 *
 * Addresses NEAR THE TOP of each region, chosen so that a chip profile
 * carrying the sibling's smaller map cannot answer them: nRF54L15 has 1524 KB
 * of RRAM and 256 KB of SRAM, so both of these land outside it.
 */
#define RRAM_PROBE_ADDR             0x001F0000UL  /* 1984 KB: inside 2036, outside 1524 */
#define RAM_PROBE_ADDR              0x2007F000UL  /* 508 KB in: inside 511, outside 256 */

#define PANEL_W                     240u
#define PANEL_H                     536u

#endif /* NRF54LM20A_H */
