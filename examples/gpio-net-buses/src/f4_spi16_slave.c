/* STM32F401 SPI1 slave on gpio nets, 16-bit frames (CR1.DFF), mode 0. Its
 * PA4 is a plain input here: the F401's hardware NSS pad is not routed by
 * the model, so the slave selects itself in software (SSM=1, SSI=0) and
 * the master frames by the words it sends.
 *
 *   PA5 SCK (AF5)   PA6 MISO (AF5)   PA7 MOSI (AF5)
 *
 * Answers 0x1234, 0xBEEF, 0x00FF from the RXNE interrupt. RESULT[0..2]
 * words received, RESULT[4] interrupts taken, RESULT[5] OVR seen.
 */
#include "f4.h"

void spi1_handler(void);

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 64] = {
    [0] = 0x20009000u,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
    [16 + 35] = (uintptr_t)spi1_handler,
};

static const uint16_t answers[4] = {0x1234, 0xBEEF, 0x00FF, 0x7E7E};
static volatile uint32_t count;

void spi1_handler(void) {
    uint32_t sr = SPI1_SR;
    if (sr & (1u << 6)) {
        RESULT[5] = 1u;
    }
    if (sr & 1u) {
        uint32_t word = SPI1_DR & 0xFFFFu;
        if (count < 3u) {
            RESULT[count] = word;
        }
        count = count + 1u;
        RESULT[4] = count;
        SPI1_DR = answers[count < 3u ? count : 3u];
    }
}

int main(void) {
    RCC_AHB1ENR |= 1u;
    RCC_APB2ENR |= 1u << 12;
    for (unsigned pin = 5; pin <= 7; ++pin) {
        set_af_low(&GPIOA_AFRL, pin, 5);
        set_mode(&GPIOA_MODER, pin, 2);
    }
    SPI1_CR2 = 1u << 6;                          /* RXNEIE */
    SPI1_CR1 = (1u << 11) | (1u << 9);           /* DFF, SSM, SSI = 0 */
    SPI1_DR = answers[0];                        /* first answer, SPE=0 */
    SPI1_CR1 |= 1u << 6;                         /* SPE */
    NVIC_ISER1 = 1u << (35 - 32);
    __asm__ volatile("cpsie i" ::: "memory");
    for (;;) {
        __asm__ volatile("wfi");
    }
}

void reset(void) { (void)main(); for (;;) {} }
