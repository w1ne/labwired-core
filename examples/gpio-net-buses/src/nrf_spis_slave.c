/* nRF52840 SPIS0 slave on gpio nets against the STM32G071 SPI1 master
 * (spi_master.c). Mode 0, MSB first.
 *
 *   P0.02 CSN   P0.03 SCK   P0.04 MOSI   P0.05 MISO
 *
 * Answers 0x81 0x82 0x83 0x84 from its TX buffer. The CPU hands the buffers
 * to the slave with TASKS_RELEASE and gets them back after the transaction
 * (SHORTS.END_ACQUIRE):
 *   RESULT[0..3] bytes received, RESULT[4] RXD.AMOUNT, RESULT[5] TXD.AMOUNT,
 *   RESULT[6] SEMSTAT after ACQUIRED (1: the CPU), RESULT[7] = 1 done.
 */
#include "nrf52.h"

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 48] = {
    [0] = 0x20040000u,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
};

int main(void) {
    for (unsigned pin = 2; pin <= 5; ++pin) {
        P0_PIN_CNF(pin) = PIN_INPUT;    /* the slave drives MISO itself */
    }
    SPIS_PSEL_CSN = 2u;
    SPIS_PSEL_SCK = 3u;
    SPIS_PSEL_MOSI = 4u;
    SPIS_PSEL_MISO = 5u;
    SPIS_CONFIG = 0u;
    SPIS_DEF = 0xEEu;
    S0_ORC = 0xFFu;
    for (int i = 0; i < 4; ++i) {
        TXBUF[i] = (uint8_t)(0x81 + i);
    }
    S0_TXD_PTR = (uint32_t)TXBUF;
    S0_TXD_MAXCNT = 4u;
    S0_RXD_PTR = (uint32_t)RXBUF;
    S0_RXD_MAXCNT = 8u;
    S0_SHORTS = 1u << 2;                /* END_ACQUIRE */
    S0_ENABLE = 2u;                     /* SPIS */
    SPIS_TASKS_RELEASE = 1u;            /* the buffers are the slave's */

    while (!SPIS_EVENTS_END) {}
    SPIS_EVENTS_END = 0u;
    while (!SPIS_EVENTS_ACQUIRED) {}
    SPIS_EVENTS_ACQUIRED = 0u;
    for (int i = 0; i < 4; ++i) {
        RESULT[i] = RXBUF[i];
    }
    RESULT[4] = S0_RXD_AMOUNT;
    RESULT[5] = S0_TXD_AMOUNT;
    RESULT[6] = SPIS_SEMSTAT;
    RESULT[7] = 1u;
    for (;;) {}
}

void reset(void) { (void)main(); for (;;) {} }
