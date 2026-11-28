#include <stdint.h>

/* Register addresses: WCH CH32X035 RM V1.9 (RCC Table 3-1, GPIO port B). */
#define RCC_APB2PCENR (*(volatile uint32_t *)0x40021018u)
#define GPIOB_CFGHR   (*(volatile uint32_t *)0x40010C04u)
#define GPIOB_BSHR    (*(volatile uint32_t *)0x40010C10u)
#define GPIOB_BCR     (*(volatile uint32_t *)0x40010C14u)

#define LED_BIT 12u

static void delay(void) {
    for (volatile uint32_t i = 0; i < 500; ++i) {
        __asm__ volatile("nop");
    }
}

void main(void) {
    RCC_APB2PCENR |= 1u << 3; /* enable GPIOB */
    /* PB12 is in CFGHR (pins 8..15): push-pull output (GPIO_Init nibble 0x1). */
    GPIOB_CFGHR = (GPIOB_CFGHR & ~(0xFu << ((LED_BIT - 8u) * 4u)))
                | (0x1u << ((LED_BIT - 8u) * 4u));

    for (;;) {
        GPIOB_BSHR = 1u << LED_BIT; /* set PB12 */
        delay();
        GPIOB_BCR = 1u << LED_BIT;  /* reset PB12 */
        delay();
    }
}
