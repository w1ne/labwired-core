/* STM32G071 SPI1 master on gpio nets, both directions by DMA. Mode 0, MSB
 * first, 8-bit frames, SCK = f/128 (BR=6), chip select on a GPIO (PA4) as in
 * spi_master.c. The sequence is HAL_SPI_TransmitReceive_DMA's on the FIFO
 * SPI (RM0444 §35.5.10): FRXTH for 8-bit frames, RXDMAEN, the RX channel,
 * the TX channel, SPE, and TXDMAEN last.
 *
 *   DMAMUX channel 0 <- SPI1_RX (16): DMA1 channel 1, SPI1_DR -> rx[]
 *   DMAMUX channel 1 <- SPI1_TX (17): DMA1 channel 2, tx[] -> SPI1_DR
 *
 *   PA4 CS (GPIO push-pull)   PA5 SCK (AF0)   PA6 MISO (AF0)   PA7 MOSI (AF0)
 *
 * Sends 8 bytes in one select: RESULT[0..7] the bytes received, RESULT[8]
 * DMA1_ISR at the end, RESULT[9] / [10] CNDTR of the RX / TX channel,
 * RESULT[11] SPI1_SR, RESULT[15] = 1 done.
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
static const uint8_t tx[N] = {0xA5, 0x3C, 0x5A, 0xC3, 0x0F, 0xF0, 0x81, 0x7E};
static volatile uint8_t rx[N];

int main(void) {
    RCC_IOPENR |= 1u;           /* GPIOA */
    RCC_APBENR2 |= 1u << 12;    /* SPI1 */
    RCC_AHBENR |= 1u;           /* DMA1 + DMAMUX */
    GPIOA_BSRR = 1u << 4;       /* CS high before it becomes an output */
    set_mode(&GPIOA_MODER, 4, 1);
    for (unsigned pin = 5; pin <= 7; ++pin) {
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

    /* MSTR, BR=6, SSM+SSI. 8-bit frames, FRXTH, RXDMAEN. */
    SPI1_CR1 = (1u << 2) | (6u << 3) | (1u << 9) | (1u << 8);
    SPI1_CR2 = (7u << 8) | (1u << 12) | (1u << 0);

    delay(400);                 /* let the slave come up */
    GPIOA_BSRR = 1u << 20;      /* CS low */
    delay(20);
    DMA1_CCR(1) = DMA_CCR_MINC | DMA_CCR_EN;
    DMA1_CCR(2) = DMA_CCR_MINC | DMA_CCR_DIR | DMA_CCR_EN;
    SPI1_CR1 |= 1u << 6;        /* SPE */
    SPI1_CR2 |= 1u << 1;        /* TXDMAEN: the transfer starts */

    while (!(DMA1_ISR & DMA_ISR_TCIF(1))) {}
    while (SPI1_SR & (1u << 7)) {}
    delay(20);
    GPIOA_BSRR = 1u << 4;       /* CS high */
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
