/* nRF52840 SPIM0 master on gpio nets against the STM32G071 SPI1 slave
 * (spi_slave.c). Mode 0, MSB first, 125 kHz (FREQUENCY = K125); chip select
 * is a plain GPIO, as on SPIM0..2.
 *
 *   P0.02 CS (GPIO)   P0.03 SCK   P0.04 MOSI   P0.05 MISO
 *
 * Sends 0xA5 0x3C 0x5A 0xC3 in one EasyDMA transfer and waits for END in the
 * SPIM interrupt (INTENSET.END):
 *   RESULT[0..3] received bytes, RESULT[4] TXD.AMOUNT, RESULT[5] RXD.AMOUNT,
 *   RESULT[6] END interrupts taken, RESULT[7] = 1 done.
 */
#include "nrf52.h"

void serial0_handler(void);

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 48] = {
    [0] = 0x20040000u,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
    [16 + IRQ_SERIAL0] = (uintptr_t)serial0_handler,
};

void serial0_handler(void) {
    if (SPIM_EVENTS_END) {
        SPIM_EVENTS_END = 0u;
        RESULT[6] += 1u;
    }
}

int main(void) {
    P0_OUTSET = 1u << 2;                /* CS high before it becomes an output */
    P0_PIN_CNF(2) = PIN_OUTPUT;
    P0_PIN_CNF(3) = PIN_OUTPUT;         /* SCK idles low (CPOL = 0) */
    P0_PIN_CNF(4) = PIN_OUTPUT;
    P0_PIN_CNF(5) = PIN_INPUT;
    SPIM_PSEL_SCK = 3u;
    SPIM_PSEL_MOSI = 4u;
    SPIM_PSEL_MISO = 5u;
    SPIM_FREQUENCY = 0x02000000u;       /* K125 */
    SPIM_CONFIG = 0u;                   /* MSB first, CPHA 0, CPOL 0 */
    S0_ORC = 0xFFu;
    static const uint8_t tx[4] = {0xA5, 0x3C, 0x5A, 0xC3};
    for (int i = 0; i < 4; ++i) {
        TXBUF[i] = tx[i];               /* EasyDMA reads RAM only */
    }
    S0_TXD_PTR = (uint32_t)TXBUF;
    S0_TXD_MAXCNT = 4u;
    S0_RXD_PTR = (uint32_t)RXBUF;
    S0_RXD_MAXCNT = 4u;
    S0_INTENSET = 1u << 6;              /* END */
    NVIC_ISER = 1u << IRQ_SERIAL0;
    __asm__ volatile("cpsie i" ::: "memory");
    S0_ENABLE = 7u;                     /* SPIM */

    delay(400);                         /* let the slave come up */
    P0_OUTCLR = 1u << 2;                /* CS low */
    delay(20);
    SPIM_TASKS_START = 1u;
    while (RESULT[6] == 0u) {
        __asm__ volatile("wfi");
    }
    delay(20);
    P0_OUTSET = 1u << 2;                /* CS high */
    for (int i = 0; i < 4; ++i) {
        RESULT[i] = RXBUF[i];
    }
    RESULT[4] = S0_TXD_AMOUNT;
    RESULT[5] = S0_RXD_AMOUNT;
    RESULT[7] = 1u;
    for (;;) {}
}

void reset(void) { (void)main(); for (;;) {} }
