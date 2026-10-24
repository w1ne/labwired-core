/* STM32L476 / STM32G474 SPI1 slave on gpio nets with the hardware slave
 * select (SSM=0): it is selected by the level on its own PA4 = SPI1_NSS
 * (AF5). Mode 0, 8-bit frames; every byte is taken in the SPI1 interrupt
 * (RXNEIE), which also queues the next answer.
 *
 *   PA4 NSS (AF5)   PA5 SCK (AF5)   PA6 MISO (AF5)   PA7 MOSI (AF5)
 *
 * Answers 81 82 83 84. RESULT[0..3] bytes received, RESULT[4] RXNE
 * interrupts taken, RESULT[5] SR overrun flag seen (0/1).
 */
#include "l4.h"

void spi1_handler(void);

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 64] = {
    [0] = 0x20009000u,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
    [16 + IRQ_SPI1] = (uintptr_t)spi1_handler,
};

static volatile uint32_t count;

void spi1_handler(void) {
    uint32_t sr = SPI1_SR;
    if (sr & (1u << 6)) {
        RESULT[5] = 1u;
    }
    if (sr & 1u) {
        uint32_t byte = SPI1_DR8;
        if (count < 4u) {
            RESULT[count] = byte;
        }
        count = count + 1u;
        RESULT[4] = count;
        SPI1_DR8 = (uint8_t)(0x81u + count);
    }
}

int main(void) {
    RCC_AHB2ENR |= 1u;
    RCC_APB2ENR |= 1u << 12;
    for (unsigned pin = 4; pin <= 7; ++pin) {
        set_af_low(&GPIOA_AFRL, pin, 5);
        set_mode(&GPIOA_MODER, pin, 2);
    }
    SPI1_CR1 = 0u;
    SPI1_CR1 = 1u << 6;                           /* slave, hardware NSS, SPE */
    NVIC_ISER1 = 1u << (IRQ_SPI1 - 32u);
    __asm__ volatile("cpsie i" ::: "memory");
    SPI1_CR2 = (7u << 8) | (1u << 12) | (1u << 6); /* DS=7, FRXTH, RXNEIE */
    SPI1_DR8 = 0x81u;                             /* first answer */
    for (;;) {
        __asm__ volatile("wfi");
    }
}

void reset(void) { (void)main(); for (;;) {} }
