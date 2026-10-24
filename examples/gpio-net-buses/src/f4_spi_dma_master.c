/* STM32F401 DMA2: stream arbitration, then SPI1 master on gpio nets with
 * both directions through the stream FIFOs (RM0368 §9, the RM0090 DMA).
 *
 * 1. Priority. Streams 0 and 1 copy memory to memory, 64 bytes each.
 *    Stream 0 (PL = low) is enabled first, stream 1 (PL = very high) right
 *    after: the arbiter gives every slot to stream 1 until it is done, so
 *    stream 1 completes first although it started second.
 *      RESULT[0] the stream that completed first, RESULT[1] the second,
 *      RESULT[2] stream 0's NDTR when stream 1 completed, RESULT[3] = 1 if
 *      both copies are byte-exact.
 *
 * 2. FIFO + bursts. SPI1 master, mode 0, 8-bit frames, SCK = f/128, CS on
 *    PA4 (GPIO), against the G071 DMA slave (spi_dma_slave.c, answers
 *    81..88). DMA2 channel 3 (RM0368 Table 28): stream 2 SPI1_RX, stream 3
 *    SPI1_TX, both in FIFO mode (DMDIS), PSIZE = byte, MSIZE = half-word,
 *    MBURST = INCR4, FTH = 1/2 (RM0090 Table 48: one INCR4 half-word burst).
 *    The TX FIFO is filled from memory by one 4-beat half-word burst and
 *    unpacked a byte per SPI request; the RX FIFO packs the received bytes
 *    and writes them as one 4-beat half-word burst at the threshold.
 *      RESULT[4..11] the bytes received, RESULT[12] LISR at the end,
 *      RESULT[13] NDTR of stream 2 | stream 3 << 16, RESULT[14] SPI1_SR,
 *      RESULT[15] = 1 done.
 *
 *   PA4 CS (GPIO push-pull)   PA5 SCK (AF5)   PA6 MISO (AF5)   PA7 MOSI (AF5)
 */
#include "f4.h"

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 64] = {
    [0] = 0x20009000u,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
};

#define DMA2_BASE   0x40026400u
#define DMA2_LISR   REG(DMA2_BASE + 0x00u)
#define DMA2_LIFCR  REG(DMA2_BASE + 0x08u)
#define DMA_SCR(s)   REG(DMA2_BASE + 0x10u + 0x18u * (s))
#define DMA_SNDTR(s) REG(DMA2_BASE + 0x14u + 0x18u * (s))
#define DMA_SPAR(s)  REG(DMA2_BASE + 0x18u + 0x18u * (s))
#define DMA_SM0AR(s) REG(DMA2_BASE + 0x1Cu + 0x18u * (s))
#define DMA_SFCR(s)  REG(DMA2_BASE + 0x24u + 0x18u * (s))

/* SxCR (RM0090 §10.5.5). */
#define CR_EN        (1u << 0)
#define CR_DIR_M2P   (1u << 6)
#define CR_DIR_M2M   (2u << 6)
#define CR_PINC      (1u << 9)
#define CR_MINC      (1u << 10)
#define CR_MSIZE_16  (1u << 13)
#define CR_PL(p)     ((uint32_t)(p) << 16)
#define CR_MBURST_4  (1u << 23)
#define CR_CHSEL(c)  ((uint32_t)(c) << 25)
/* SxFCR: FTH = 1/2, DMDIS. */
#define FCR_HALF_FIFO ((1u << 2) | 1u)
/* LISR TCIF of streams 0..3 (bases 0, 6, 16, 22; TCIF = +5). */
#define TCIF0 (1u << 5)
#define TCIF1 (1u << 11)
#define TCIF2 (1u << 21)
#define TCIF3 (1u << 27)

#define M 64u
#define N 8u
static uint8_t src[2][M];
static uint8_t dst[2][M];
static const uint8_t tx[N] = {0xA5, 0x3C, 0x5A, 0xC3, 0x0F, 0xF0, 0x81, 0x7E};
static volatile uint8_t rx[N];

static void priority(void) {
    for (unsigned s = 0; s < 2; ++s) {
        for (unsigned i = 0; i < M; ++i) {
            src[s][i] = (uint8_t)(i * 7u + s * 101u + 1u);
            dst[s][i] = 0u;
        }
        DMA_SPAR(s) = (uint32_t)src[s];   /* M2M: the peripheral port reads */
        DMA_SM0AR(s) = (uint32_t)dst[s];
        DMA_SNDTR(s) = M;
        DMA_SFCR(s) = FCR_HALF_FIFO;
    }
    DMA_SCR(0) = CR_DIR_M2M | CR_PINC | CR_MINC | CR_PL(0) | CR_EN;
    DMA_SCR(1) = CR_DIR_M2M | CR_PINC | CR_MINC | CR_PL(3) | CR_EN;
    /* Poll for the two completions, stream 0 checked first in each poll. */
    unsigned n = 0, seen = 0;
    while (n < 2u) {
        uint32_t isr = DMA2_LISR;
        if ((isr & TCIF0) && !(seen & 1u)) {
            seen |= 1u;
            RESULT[n++] = 0u;
        }
        if ((isr & TCIF1) && !(seen & 2u)) {
            seen |= 2u;
            if (n == 0u) {
                RESULT[2] = DMA_SNDTR(0);
            }
            RESULT[n++] = 1u;
        }
    }
    uint32_t same = 1u;
    for (unsigned s = 0; s < 2; ++s) {
        for (unsigned i = 0; i < M; ++i) {
            same &= dst[s][i] == src[s][i];
        }
    }
    RESULT[3] = same;
    DMA2_LIFCR = 0x0F7D0F7Du;
}

int main(void) {
    RCC_AHB1ENR |= 1u | (1u << 22);   /* GPIOA, DMA2 */
    RCC_APB2ENR |= 1u << 12;          /* SPI1 */
    priority();

    GPIOA_BSRR = 1u << 4;             /* CS high before it becomes an output */
    set_mode(&GPIOA_MODER, 4, 1);
    for (unsigned pin = 5; pin <= 7; ++pin) {
        set_af_low(&GPIOA_AFRL, pin, 5);
        set_mode(&GPIOA_MODER, pin, 2);
    }
    /* RX: stream 2, SPI1_DR -> rx[]; TX: stream 3, tx[] -> SPI1_DR. */
    DMA_SPAR(2) = (uint32_t)&SPI1_DR;
    DMA_SM0AR(2) = (uint32_t)rx;
    DMA_SNDTR(2) = N;
    DMA_SFCR(2) = FCR_HALF_FIFO;
    DMA_SPAR(3) = (uint32_t)&SPI1_DR;
    DMA_SM0AR(3) = (uint32_t)tx;
    DMA_SNDTR(3) = N;
    DMA_SFCR(3) = FCR_HALF_FIFO;

    /* MSTR, BR=6, SSM+SSI, 8-bit frames, RXDMAEN. */
    SPI1_CR1 = (1u << 2) | (6u << 3) | (1u << 9) | (1u << 8);
    SPI1_CR2 = 1u << 0;

    delay(400);                       /* let the slave come up */
    GPIOA_BSRR = 1u << 20;            /* CS low */
    delay(20);
    DMA_SCR(2) = CR_CHSEL(3) | CR_MINC | CR_MSIZE_16 | CR_MBURST_4 | CR_EN;
    DMA_SCR(3) = CR_CHSEL(3) | CR_DIR_M2P | CR_MINC | CR_MSIZE_16 | CR_MBURST_4 | CR_EN;
    SPI1_CR1 |= 1u << 6;              /* SPE */
    SPI1_CR2 |= 1u << 1;              /* TXDMAEN: the transfer starts */

    while (!(DMA2_LISR & TCIF2)) {}
    while (SPI1_SR & (1u << 7)) {}
    delay(20);
    GPIOA_BSRR = 1u << 4;             /* CS high */
    for (unsigned i = 0; i < N; ++i) {
        RESULT[4 + i] = rx[i];
    }
    RESULT[12] = DMA2_LISR;
    RESULT[13] = DMA_SNDTR(2) | (DMA_SNDTR(3) << 16);
    RESULT[14] = SPI1_SR;
    RESULT[15] = 1u;
    for (;;) {}
}

void reset(void) { (void)main(); for (;;) {} }
