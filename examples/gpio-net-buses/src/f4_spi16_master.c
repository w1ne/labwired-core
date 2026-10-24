/* STM32F401 SPI1 master on gpio nets, 16-bit frames (CR1.DFF), mode 0,
 * MSB first, SCK = f/128 (BR=6).
 *
 *   PA4 CS (GPIO push-pull)   PA5 SCK (AF5)   PA6 MISO (AF5)   PA7 MOSI (AF5)
 *
 * -DHW_NSS=1: PA4 is SPI1_NSS (AF5) driven by the SPI itself (SSM=0,
 * CR2.SSOE=1): low from SPE=1 until SPE=0 (RM0368 §20.3.1), so the master
 * enables the SPI to select the slave and disables it to let go.
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

#ifndef HW_NSS
#define HW_NSS 0
#endif

int main(void) {
    RCC_AHB1ENR |= 1u;          /* GPIOA */
    RCC_APB2ENR |= 1u << 12;    /* SPI1 */
#if HW_NSS
    for (unsigned pin = 4; pin <= 7; ++pin) {
        set_af_low(&GPIOA_AFRL, pin, 5);
        set_mode(&GPIOA_MODER, pin, 2);
    }
    /* SSOE: the SPI drives NSS. MSTR, BR=6, DFF; SPE (and NSS low) later. */
    SPI1_CR2 = 1u << 2;
    SPI1_CR1 = (1u << 2) | (6u << 3) | (1u << 11);
    delay(400);                 /* let the slave come up */
    SPI1_CR1 |= 1u << 6;        /* SPE: NSS low */
    delay(20);
#else
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
#endif
    for (int i = 0; i < 3; ++i) {
        while (!(SPI1_SR & (1u << 1))) {}
        SPI1_DR = tx[i];
        while (!(SPI1_SR & 1u)) {}
        RESULT[i] = SPI1_DR & 0xFFFFu;
    }
    while (SPI1_SR & (1u << 7)) {}
    delay(20);
#if HW_NSS
    SPI1_CR1 &= ~(1u << 6);     /* SPE=0: NSS released (the pull-up) */
#else
    GPIOA_BSRR = 1u << 4;       /* CS high */
#endif
    RESULT[4] = SPI1_SR;
    RESULT[7] = 1u;
    for (;;) {}
}

void reset(void) { (void)main(); for (;;) {} }
