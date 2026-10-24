/* STM32G071 SPI1 master on gpio nets. Mode 0, MSB first, 8-bit frames,
 * SCK = f/128 (BR=6). Built once per frame shape: -DCPOL=1 / -DCPHA=1 /
 * -DLSBFIRST=1 select SPI mode 1-3 and LSB-first, -DDS=n a frame of n bits
 * (CR2.DS, 4..16), -DFRXTH=0 leaves CR2.FRXTH at its reset 0 (build.sh). Chip select is a plain GPIO (PA4), as most
 * firmware does it; the slave's PA4 is its hardware NSS input.
 *
 * The G0 SPI is the FIFO type (RM0444 §35): frames up to 8 bits take one
 * byte of the 32-bit RX FIFO, so CR2.FRXTH is set to raise RXNE per frame
 * and DR is accessed 8 bits wide (a 16-bit access would pack two frames);
 * wider frames use 16-bit accesses.
 *
 *   PA4 CS (GPIO push-pull)   PA5 SCK (AF0)   PA6 MISO (AF0)   PA7 MOSI (AF0)
 *
 * Sends 4 frames in one select and stores what came back on MISO:
 *   RESULT[0..3] received frames, RESULT[4] SR at the end, RESULT[7] = 1 done.
 */
#include "g0.h"

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 32] = {
    [0] = 0x20009000u,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
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
#ifndef DS
#define DS 8
#endif
#define SHAPE ((CPOL << 1) | CPHA | (LSBFIRST << 7))
/* CR2: DS-1 in DS[3:0]; FRXTH (RXNE at 8 bits) for frames up to 8 bits. */
#ifndef FRXTH
#define FRXTH ((DS) <= 8)
#endif
#define CR2_FRAME ((((DS) - 1u) << 8) | (FRXTH ? (1u << 12) : 0u))
#define MASK ((1u << (DS)) - 1u)

#if DS <= 8
static const uint16_t tx[4] = {0xA5, 0x3C, 0x5A, 0xC3};
#define DR_WRITE(v) (*(volatile uint8_t *)&SPI1_DR = (uint8_t)(v))
#define DR_READ() (*(volatile uint8_t *)&SPI1_DR)
#else
static const uint16_t tx[4] = {0xA55A, 0x3CC3, 0x5AA5, 0xC33C};
#define DR_WRITE(v) (*(volatile uint16_t *)&SPI1_DR = (uint16_t)(v))
#define DR_READ() (*(volatile uint16_t *)&SPI1_DR)
#endif

int main(void) {
    RCC_IOPENR |= 1u;           /* GPIOA */
    RCC_APBENR2 |= 1u << 12;    /* SPI1 */
    GPIOA_BSRR = 1u << 4;       /* CS high before it becomes an output */
    set_mode(&GPIOA_MODER, 4, 1);
    for (unsigned pin = 5; pin <= 7; ++pin) {
        set_af_low(&GPIOA_AFRL, pin, 0);
        set_mode(&GPIOA_MODER, pin, 2);
    }
    /* MSTR, BR=6, SSM+SSI (the master's own NSS is not used), the frame
     * shape, then SPE. */
    SPI1_CR2 = CR2_FRAME;
    SPI1_CR1 = (1u << 2) | (6u << 3) | (1u << 9) | (1u << 8) | SHAPE;
    SPI1_CR1 |= 1u << 6;

    delay(400);                 /* let the slave come up */
    GPIOA_BSRR = 1u << 20;      /* CS low */
    delay(20);
    for (int i = 0; i < 4; ++i) {
        while (!(SPI1_SR & (1u << 1))) {}
        DR_WRITE(tx[i] & MASK);
        while (!(SPI1_SR & 1u)) {}
        RESULT[i] = DR_READ();
    }
    while (SPI1_SR & (1u << 7)) {}
    delay(20);
    GPIOA_BSRR = 1u << 4;       /* CS high */
    RESULT[4] = SPI1_SR;
    RESULT[7] = 1u;
    for (;;) {}
}

void reset(void) { (void)main(); for (;;) {} }
