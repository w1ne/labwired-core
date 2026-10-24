/* STM32L476 and STM32G474 registers the bus-over-nets firmware uses (RM0351,
 * RM0440). Both parts put RCC, GPIOA and SPI1 at the same addresses with the
 * same enable bits, and SPI1's interrupt on IRQ 35, so one header serves both
 * (build.sh builds one image for both). */
#ifndef L4_H
#define L4_H
#include <stdint.h>

#define REG(a) (*(volatile uint32_t *)(a))
#define RCC_AHB2ENR  REG(0x4002104Cu)   /* GPIOAEN bit 0 */
#define RCC_APB2ENR  REG(0x40021060u)   /* SPI1EN bit 12 */

#define GPIOA_MODER  REG(0x48000000u)
#define GPIOA_BSRR   REG(0x48000018u)
#define GPIOA_AFRL   REG(0x48000020u)

#define SPI1_CR1 REG(0x40013000u)
#define SPI1_CR2 REG(0x40013004u)
#define SPI1_SR  REG(0x40013008u)
#define SPI1_DR  REG(0x4001300Cu)
#define SPI1_DR8 (*(volatile uint8_t *)0x4001300Cu)

#define NVIC_ISER1 REG(0xE000E104u)
#define IRQ_SPI1 35u

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
