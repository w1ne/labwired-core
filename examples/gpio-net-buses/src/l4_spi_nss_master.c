/* STM32L476 / STM32G474 SPI1 master on gpio nets with the hardware slave
 * select: SSM=0, CR2.SSOE=1, so the SPI itself drives PA4 = SPI1_NSS (AF5)
 * low from SPE=1 until SPE=0 (RM0351 §42.4.5, RM0440 §39.4.5). Mode 0, MSB
 * first, 8-bit frames (CR2.DS=7, FRXTH), SCK = f/128 (BR=6).
 *
 *   PA4 NSS (AF5)   PA5 SCK (AF5)   PA6 MISO (AF5)   PA7 MOSI (AF5)
 *
 * -DNSSP=1 also sets CR2.NSSP, NSS pulse mode: NSS rises after every frame
 * that is not followed back to back by another, and falls again as the next
 * one starts. This master polls RXNE between frames, so every frame gets its
 * own NSS low period.
 *
 * Sends A5 3C 5A C3 and stores what came back on MISO:
 *   RESULT[0..3] received bytes, RESULT[4] SR at the end, RESULT[7] = 1 done.
 */
#include "l4.h"

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 64] = {
    [0] = 0x20009000u,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
};

#ifndef NSSP
#define NSSP 0
#endif

static const uint8_t tx[4] = {0xA5, 0x3C, 0x5A, 0xC3};

int main(void) {
    RCC_AHB2ENR |= 1u;          /* GPIOA */
    RCC_APB2ENR |= 1u << 12;    /* SPI1 */
    for (unsigned pin = 4; pin <= 7; ++pin) {
        set_af_low(&GPIOA_AFRL, pin, 5);
        set_mode(&GPIOA_MODER, pin, 2);
    }
    /* CR2: DS=7 (8-bit), FRXTH, SSOE, NSSP when asked. */
    SPI1_CR2 = (7u << 8) | (1u << 12) | (1u << 2) | (NSSP ? (1u << 3) : 0u);
    /* MSTR, BR=6, hardware NSS (SSM=0); SPE (and NSS low) later. */
    SPI1_CR1 = (1u << 2) | (6u << 3);
    delay(400);                 /* let the slave come up */
    SPI1_CR1 |= 1u << 6;        /* SPE: NSS low */
    delay(20);
    for (int i = 0; i < 4; ++i) {
        while (!(SPI1_SR & (1u << 1))) {}
        SPI1_DR8 = tx[i];
        while (!(SPI1_SR & 1u)) {}
        RESULT[i] = SPI1_DR8;
    }
    while (SPI1_SR & (1u << 7)) {}
    delay(20);
    SPI1_CR1 &= ~(1u << 6);     /* SPE=0: NSS released (the pull-up) */
    RESULT[4] = SPI1_SR;
    RESULT[7] = 1u;
    for (;;) {}
}

void reset(void) { (void)main(); for (;;) {} }
