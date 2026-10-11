/* STM32F401 SPI1 master on gpio nets, 16-bit frames (CR1.DFF), mode 0,
 * MSB first, SCK = f/128 (BR=6).
 *
 *   PA4 CS (GPIO push-pull)   PA5 SCK (AF5)   PA6 MISO (AF5)   PA7 MOSI (AF5)
 *
 * Sends 3 words in one select and stores what came back on MISO:
 *   RESULT[0..2] received words, RESULT[4] SR at the end, RESULT[7] = 1 done.
 */
#include "f4.h"

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 64] = {
    [0] = 0x20009000u,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
};

static const uint16_t tx[3] = {0xA55A, 0x0FF0, 0x8001};

int main(void) {
    RCC_AHB1ENR |= 1u;          /* GPIOA */
    RCC_APB2ENR |= 1u << 12;    /* SPI1 */
    GPIOA_BSRR = 1u << 4;       /* CS high before it becomes an output */
    set_mode(&GPIOA_MODER, 4, 1);
    for (unsigned pin = 5; pin <= 7; ++pin) {
        set_af_low(&GPIOA_AFRL, pin, 5);
        set_mode(&GPIOA_MODER, pin, 2);
    }
    /* MSTR, BR=6, SSM+SSI, DFF (16-bit), then SPE. */
    SPI1_CR1 = (1u << 2) | (6u << 3) | (1u << 9) | (1u << 8) | (1u << 11);
    SPI1_CR1 |= 1u << 6;

    delay(400);                 /* let the slave come up */
    GPIOA_BSRR = 1u << 20;      /* CS low */
    delay(20);
    for (int i = 0; i < 3; ++i) {
        while (!(SPI1_SR & (1u << 1))) {}
        SPI1_DR = tx[i];
        while (!(SPI1_SR & 1u)) {}
        RESULT[i] = SPI1_DR & 0xFFFFu;
    }
    while (SPI1_SR & (1u << 7)) {}
    delay(20);
    GPIOA_BSRR = 1u << 4;       /* CS high */
    RESULT[4] = SPI1_SR;
    RESULT[7] = 1u;
    for (;;) {}
}

void reset(void) { (void)main(); for (;;) {} }
