/* STM32G071 SPI1 slave on gpio nets, hardware NSS. Every byte the master
 * clocks in is taken in the SPI1 interrupt (RXNEIE), which also queues the
 * next answer.
 *
 *   PA4 NSS (AF0)   PA5 SCK (AF0)   PA6 MISO (AF0)   PA7 MOSI (AF0)
 *
 * Answers 0x81, 0x82, 0x83, 0x84. RESULT[0..3] bytes received,
 * RESULT[4] interrupts taken, RESULT[5] SR overrun flag seen (0/1).
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

static volatile uint32_t count;

void spi1_handler(void) {
    uint32_t sr = SPI1_SR;
    if (sr & (1u << 6)) {
        RESULT[5] = 1u;
    }
    if (sr & 1u) {
        uint32_t byte = *(volatile uint8_t *)&SPI1_DR;
        if (count < 4u) {
            RESULT[count] = byte;
        }
        count = count + 1u;
        RESULT[4] = count;
        *(volatile uint8_t *)&SPI1_DR = (uint8_t)(0x81u + count);
    }
}

int main(void) {
    RCC_IOPENR |= 1u;
    RCC_APBENR2 |= 1u << 12;
    for (unsigned pin = 4; pin <= 7; ++pin) {
        set_af_low(&GPIOA_AFRL, pin, 0);
        set_mode(&GPIOA_MODER, pin, 2);
    }
    SPI1_CR2 = 1u << 6;                         /* RXNEIE */
    SPI1_CR1 = 1u << 6;                         /* slave, hardware NSS, SPE */
    *(volatile uint8_t *)&SPI1_DR = 0x81u;      /* first answer */
    NVIC_ISER = 1u << 25;
    __asm__ volatile("cpsie i" ::: "memory");
    for (;;) {
        __asm__ volatile("wfi");
    }
}

void reset(void) { (void)main(); for (;;) {} }
