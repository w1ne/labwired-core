/* STM32G071 registers the bus-over-nets firmware uses (RM0444). */
#ifndef G0_H
#define G0_H
#include <stdint.h>

#define REG(a) (*(volatile uint32_t *)(a))
#define RCC_IOPENR   REG(0x40021034u)
#define RCC_APBENR1  REG(0x4002103Cu)
#define RCC_APBENR2  REG(0x40021040u)

#define GPIOA_MODER  REG(0x50000000u)
#define GPIOA_OTYPER REG(0x50000004u)
#define GPIOA_BSRR   REG(0x50000018u)
#define GPIOA_AFRL   REG(0x50000020u)
#define GPIOB_MODER  REG(0x50000400u)
#define GPIOB_OTYPER REG(0x50000404u)
#define GPIOB_AFRL   REG(0x50000420u)

#define SPI1_CR1 REG(0x40013000u)
#define SPI1_CR2 REG(0x40013004u)
#define SPI1_SR  REG(0x40013008u)
#define SPI1_DR  REG(0x4001300Cu)

#define I2C1_CR1     REG(0x40005400u)
#define I2C1_CR2     REG(0x40005404u)
#define I2C1_OAR1    REG(0x40005408u)
#define I2C1_TIMINGR REG(0x40005410u)
#define I2C1_ISR     REG(0x40005418u)
#define I2C1_ICR     REG(0x4000541Cu)
#define I2C1_RXDR    REG(0x40005424u)
#define I2C1_TXDR    REG(0x40005428u)

#define NVIC_ISER REG(0xE000E100u)

/* Results the tests read, by address. */
#define RESULT ((volatile uint32_t *)0x20000100u)

static inline void set_mode(volatile uint32_t *moder, unsigned pin, unsigned mode) {
    *moder = (*moder & ~(3u << (pin * 2))) | (mode << (pin * 2));
}

static inline void set_af_low(volatile uint32_t *afrl, unsigned pin, unsigned af) {
    *afrl = (*afrl & ~(0xFu << (pin * 4))) | (af << (pin * 4));
}

static inline void delay(uint32_t loops) {
    for (volatile uint32_t i = 0; i < loops; ++i) {}
}

void reset(void);
void default_handler(void) { for (;;) {} }
#endif
