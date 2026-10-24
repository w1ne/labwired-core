/* STM32G071 SPI1 slave on gpio nets, hardware NSS. Every byte the master
 * clocks in is taken in the SPI1 interrupt (RXNEIE), which also queues the
 * next answer.
 *
 *   PA4 NSS (AF0)   PA5 SCK (AF0)   PA6 MISO (AF0)   PA7 MOSI (AF0)
 *
 * Answers 0x81, 0x82, 0x83, 0x84. RESULT[0..3] bytes received,
 * RESULT[4] RXNE interrupts taken, RESULT[5] SR overrun flag seen (0/1),
 * RESULT[6] TXE interrupts taken.
 *
 * Variants (build.sh): -DCPOL/-DCPHA/-DLSBFIRST/-DDS match the master's
 * frame shape (with -DDS=n the answers are 0x8181.. masked to n bits);
 * -DTXE_IRQ=1 queues every answer from the TXE interrupt (TXEIE)
 * instead, and switches TXEIE off after the fourth; -DUNDERRUN=1 writes the
 * first answer only and never refills DR.
 */
#include "g0.h"

void spi1_handler(void);

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 32] = {
    [0] = 0x20009000u,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
    [16 + 25] = (uintptr_t)spi1_handler,
};

#ifndef CPOL
#define CPOL 0
#endif
#ifndef CPHA
#define CPHA 0
#endif
#ifndef LSBFIRST
#define LSBFIRST 0
#endif
#ifndef TXE_IRQ
#define TXE_IRQ 0
#endif
#ifndef UNDERRUN
#define UNDERRUN 0
#endif
#ifndef DS
#define DS 8
#endif
#define SHAPE ((CPOL << 1) | CPHA | (LSBFIRST << 7))
/* FIFO-type SPI (RM0444 §35): DS-1 in CR2.DS, FRXTH so RXNE rises per
 * frame of up to 8 bits; DR accessed at the frame's width. */
#define CR2_FRAME ((((DS) - 1u) << 8) | ((DS) <= 8 ? (1u << 12) : 0u))
#define MASK ((1u << (DS)) - 1u)
#define ANSWER(n) (((0x81u + (n)) * 0x0101u) & MASK)
#if DS <= 8
#define DR_WRITE(v) (*(volatile uint8_t *)&SPI1_DR = (uint8_t)(v))
#define DR_READ() (*(volatile uint8_t *)&SPI1_DR)
#else
#define DR_WRITE(v) (*(volatile uint16_t *)&SPI1_DR = (uint16_t)(v))
#define DR_READ() (*(volatile uint16_t *)&SPI1_DR)
#endif

static volatile uint32_t count;
#if TXE_IRQ
static volatile uint32_t answers;
#endif

void spi1_handler(void) {
    uint32_t sr = SPI1_SR;
    if (sr & (1u << 6)) {
        RESULT[5] = 1u;
    }
    if (sr & 1u) {
        uint32_t byte = DR_READ();
        if (count < 4u) {
            RESULT[count] = byte;
        }
        count = count + 1u;
        RESULT[4] = count;
#if !TXE_IRQ && !UNDERRUN
        DR_WRITE(ANSWER(count));
#endif
    }
#if TXE_IRQ
    if ((SPI1_CR2 & (1u << 7)) && (SPI1_SR & (1u << 1))) {
        DR_WRITE(ANSWER(answers));
        answers = answers + 1u;
        RESULT[6] = answers;
        if (answers == 4u) {
            SPI1_CR2 &= ~(1u << 7);             /* TXEIE off: all queued */
        }
    }
#endif
}

int main(void) {
    RCC_IOPENR |= 1u;
    RCC_APBENR2 |= 1u << 12;
    for (unsigned pin = 4; pin <= 7; ++pin) {
        set_af_low(&GPIOA_AFRL, pin, 0);
        set_mode(&GPIOA_MODER, pin, 2);
    }
    SPI1_CR1 = SHAPE;
    SPI1_CR1 = SHAPE | (1u << 6);               /* slave, hardware NSS, SPE */
    NVIC_ISER = 1u << 25;
    __asm__ volatile("cpsie i" ::: "memory");
#if TXE_IRQ
    /* RXNEIE + TXEIE: TXE is set, so the interrupt queues the first answer
     * at once. */
    SPI1_CR2 = CR2_FRAME | (1u << 6) | (1u << 7);
#else
    SPI1_CR2 = CR2_FRAME | (1u << 6);           /* RXNEIE */
    DR_WRITE(ANSWER(0));                        /* first answer */
#endif
    for (;;) {
        __asm__ volatile("wfi");
    }
}

void reset(void) { (void)main(); for (;;) {} }
