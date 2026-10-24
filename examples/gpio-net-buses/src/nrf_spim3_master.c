/* nRF52840 SPIM3 master on gpio nets against the STM32G071 SPI1 slave
 * (spi_slave.c, hardware NSS). Mode 0, MSB first, 125 kHz (FREQUENCY =
 * K125). Unlike SPIM0..2, SPIM3 drives its own chip select and D/CX
 * (nRF52840 PS v1.x §6.25):
 *
 *   P0.02 CSN (PSEL.CSN)   P0.03 SCK   P0.04 MOSI   P0.05 MISO
 *   P0.06 DCX (PSELDCX)
 *
 * IFTIMING.CSNDUR = 64 cycles of the 64 MHz clock (1 us) between the CSN edge
 * and the nearest SCK edge; DCXCNT = 1, so D/CX is low for the first byte
 * (a command) and high for the other three. CSNPOL = 0 (active low), or 1
 * with -DCSNPOL=1 (active high: the G071, whose NSS is active low, is then
 * selected only while the SPIM is idle and never takes a frame).
 *
 * Sends A5 3C 5A C3 in one EasyDMA transfer and waits for END in the SPIM3
 * interrupt (INTENSET.END):
 *   RESULT[0..3] received bytes, RESULT[4] TXD.AMOUNT, RESULT[5] RXD.AMOUNT,
 *   RESULT[6] END interrupts taken, RESULT[7] = 1 done.
 */
#include "nrf52.h"

#ifndef CSNPOL
#define CSNPOL 0
#endif

void spim3_handler(void);

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 48] = {
    [0] = 0x20040000u,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
    [16 + IRQ_SPIM3] = (uintptr_t)spim3_handler,
};

void spim3_handler(void) {
    if (SPIM3_EVENTS_END) {
        SPIM3_EVENTS_END = 0u;
        RESULT[6] += 1u;
    }
}

int main(void) {
    /* The pads' GPIO state while the SPIM does not own them: CSN at its
     * inactive level, as the product spec asks. */
    if (CSNPOL) {
        P0_OUTCLR = 1u << 2;
    } else {
        P0_OUTSET = 1u << 2;
    }
    P0_OUTSET = 1u << 6;
    P0_PIN_CNF(2) = PIN_OUTPUT;
    P0_PIN_CNF(3) = PIN_OUTPUT;         /* SCK idles low (CPOL = 0) */
    P0_PIN_CNF(4) = PIN_OUTPUT;
    P0_PIN_CNF(5) = PIN_INPUT;
    P0_PIN_CNF(6) = PIN_OUTPUT;
    SPIM3_PSEL_SCK = 3u;
    SPIM3_PSEL_MOSI = 4u;
    SPIM3_PSEL_MISO = 5u;
    SPIM3_PSEL_CSN = 2u;
    SPIM3_PSELDCX = 6u;
    SPIM3_CSNPOL = CSNPOL;
    SPIM3_IFTIMING_CSNDUR = 64u;
    SPIM3_DCXCNT = 1u;
    SPIM3_FREQUENCY = 0x02000000u;      /* K125 */
    SPIM3_CONFIG = 0u;                  /* MSB first, CPHA 0, CPOL 0 */
    SPIM3_ORC = 0xFFu;
    static const uint8_t tx[4] = {0xA5, 0x3C, 0x5A, 0xC3};
    for (int i = 0; i < 4; ++i) {
        TXBUF[i] = tx[i];               /* EasyDMA reads RAM only */
    }
    SPIM3_TXD_PTR = (uint32_t)TXBUF;
    SPIM3_TXD_MAXCNT = 4u;
    SPIM3_RXD_PTR = (uint32_t)RXBUF;
    SPIM3_RXD_MAXCNT = 4u;
    SPIM3_INTENSET = 1u << 6;           /* END */
    NVIC_ISER1 = 1u << (IRQ_SPIM3 - 32u);
    __asm__ volatile("cpsie i" ::: "memory");
    SPIM3_ENABLE = 7u;                  /* SPIM: CSN inactive, D/CX high */

    delay(400);                         /* let the slave come up */
    SPIM3_TASKS_START = 1u;
    while (RESULT[6] == 0u) {
        __asm__ volatile("wfi");
    }
    for (int i = 0; i < 4; ++i) {
        RESULT[i] = RXBUF[i];
    }
    RESULT[4] = SPIM3_TXD_AMOUNT;
    RESULT[5] = SPIM3_RXD_AMOUNT;
    RESULT[7] = 1u;
    for (;;) {}
}

void reset(void) { (void)main(); for (;;) {} }
