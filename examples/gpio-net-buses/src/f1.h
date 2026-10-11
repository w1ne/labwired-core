/* STM32F103 registers the bus-over-nets firmware uses (RM0008). */
#ifndef F1_H
#define F1_H
#include <stdint.h>

#define REG(a) (*(volatile uint32_t *)(a))
#define RCC_APB2ENR  REG(0x40021018u)
#define RCC_APB1ENR  REG(0x4002101Cu)

#define GPIOB_CRL    REG(0x40010C00u)

#define I2C1_CR1   REG(0x40005400u)
#define I2C1_CR2   REG(0x40005404u)
#define I2C1_OAR1  REG(0x40005408u)
#define I2C1_DR    REG(0x40005410u)
#define I2C1_SR1   REG(0x40005414u)
#define I2C1_SR2   REG(0x40005418u)
#define I2C1_CCR   REG(0x4000541Cu)
#define I2C1_TRISE REG(0x40005420u)

#define NVIC_ISER0 REG(0xE000E100u)
#define NVIC_ISER1 REG(0xE000E104u)

/* Results the tests read, by address. */
#define RESULT ((volatile uint32_t *)0x20000100u)

static inline void delay(uint32_t loops) {
    for (volatile uint32_t i = 0; i < loops; ++i) {}
}

void reset(void);
void default_handler(void) { for (;;) {} }
#endif
