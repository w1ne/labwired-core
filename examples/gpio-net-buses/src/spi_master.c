/* STM32G071 SPI1 master on gpio nets. Mode 0, MSB first, 8-bit frames,
 * SCK = f/128 (BR=6). Chip select is a plain GPIO (PA4), as most firmware
 * does it; the slave's PA4 is its hardware NSS input.
 *
 *   PA4 CS (GPIO push-pull)   PA5 SCK (AF0)   PA6 MISO (AF0)   PA7 MOSI (AF0)
 *
 * Sends 4 bytes in one select and stores what came back on MISO:
 *   RESULT[0..3] received bytes, RESULT[4] SR at the end, RESULT[7] = 1 done.
 */
#include "g0.h"

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 32] = {
    [0] = 0x20009000u,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
};

static const uint8_t tx[4] = {0xA5, 0x3C, 0x5A, 0xC3};

int main(void) {
    RCC_IOPENR |= 1u;           /* GPIOA */
    RCC_APBENR2 |= 1u << 12;    /* SPI1 */
    GPIOA_BSRR = 1u << 4;       /* CS high before it becomes an output */
    set_mode(&GPIOA_MODER, 4, 1);
    for (unsigned pin = 5; pin <= 7; ++pin) {
        set_af_low(&GPIOA_AFRL, pin, 0);
        set_mode(&GPIOA_MODER, pin, 2);
    }
    /* MSTR, BR=6, SSM+SSI (the master's own NSS is not used), SPE. */
    SPI1_CR1 = (1u << 2) | (6u << 3) | (1u << 9) | (1u << 8);
    SPI1_CR1 |= 1u << 6;

    delay(400);                 /* let the slave come up */
    GPIOA_BSRR = 1u << 20;      /* CS low */
    delay(20);
    for (int i = 0; i < 4; ++i) {
        while (!(SPI1_SR & (1u << 1))) {}
        *(volatile uint8_t *)&SPI1_DR = tx[i];
        while (!(SPI1_SR & 1u)) {}
        RESULT[i] = *(volatile uint8_t *)&SPI1_DR;
    }
    while (SPI1_SR & (1u << 7)) {}
    delay(20);
    GPIOA_BSRR = 1u << 4;       /* CS high */
    RESULT[4] = SPI1_SR;
    RESULT[7] = 1u;
    for (;;) {}
}

void reset(void) { (void)main(); for (;;) {} }
