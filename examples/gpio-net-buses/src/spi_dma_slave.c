/* STM32G071 SPI1 slave on gpio nets, hardware NSS, both directions by DMA:
 * the answers stream out of memory into the TX FIFO and every received frame
 * is moved to memory, with no interrupt and no DR access by the CPU. The
 * sequence is HAL_SPI_TransmitReceive_DMA's (RM0444 §35.5.10): FRXTH,
 * RXDMAEN, the RX channel, the TX channel, SPE, TXDMAEN last — which
 * pre-fills the TX FIFO before the master's first edge.
 *
 *   DMAMUX channel 0 <- SPI1_RX (16): DMA1 channel 1, SPI1_DR -> rx[]
 *   DMAMUX channel 1 <- SPI1_TX (17): DMA1 channel 2, tx[] -> SPI1_DR
 *
 *   PA4 NSS (AF0)   PA5 SCK (AF0)   PA6 MISO (AF0)   PA7 MOSI (AF0)
 *
 * Answers 81 82 .. 88. RESULT[0..7] the bytes received, RESULT[8] DMA1_ISR,
 * RESULT[9] / [10] CNDTR of the RX / TX channel, RESULT[11] SPI1_SR,
 * RESULT[15] = 1 done (the RX channel's transfer-complete flag).
 */
#include "g0.h"

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 32] = {
    [0] = 0x20009000u,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
};

#define N 8u
static const uint8_t tx[N] = {0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88};
static volatile uint8_t rx[N];

int main(void) {
    RCC_IOPENR |= 1u;
    RCC_APBENR2 |= 1u << 12;
    RCC_AHBENR |= 1u;
    for (unsigned pin = 4; pin <= 7; ++pin) {
        set_af_low(&GPIOA_AFRL, pin, 0);
        set_mode(&GPIOA_MODER, pin, 2);
    }
    DMAMUX_CCR(0) = DMAREQ_SPI1_RX;
    DMAMUX_CCR(1) = DMAREQ_SPI1_TX;
    DMA1_CPAR(1) = (uint32_t)&SPI1_DR;
    DMA1_CMAR(1) = (uint32_t)rx;
    DMA1_CNDTR(1) = N;
    DMA1_CPAR(2) = (uint32_t)&SPI1_DR;
    DMA1_CMAR(2) = (uint32_t)tx;
    DMA1_CNDTR(2) = N;

    SPI1_CR1 = 0u;                              /* slave, hardware NSS, mode 0 */
    SPI1_CR2 = (7u << 8) | (1u << 12) | (1u << 0); /* 8 bit, FRXTH, RXDMAEN */
    DMA1_CCR(1) = DMA_CCR_MINC | DMA_CCR_EN;
    DMA1_CCR(2) = DMA_CCR_MINC | DMA_CCR_DIR | DMA_CCR_EN;
    SPI1_CR1 = 1u << 6;                         /* SPE */
    SPI1_CR2 |= 1u << 1;                        /* TXDMAEN */

    while (!(DMA1_ISR & DMA_ISR_TCIF(1))) {}
    for (unsigned i = 0; i < N; ++i) {
        RESULT[i] = rx[i];
    }
    RESULT[8] = DMA1_ISR;
    RESULT[9] = DMA1_CNDTR(1);
    RESULT[10] = DMA1_CNDTR(2);
    RESULT[11] = SPI1_SR;
    RESULT[15] = 1u;
    for (;;) {}
}

void reset(void) { (void)main(); for (;;) {} }
