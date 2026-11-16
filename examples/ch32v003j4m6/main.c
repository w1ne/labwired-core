#include <stdint.h>

/* CH32V003J4M6 (SOP-8) blink on PC4, package pin 7. PC0, used by the
 * F4P6 blink, has no pad on the SOP-8 package. */
#define RCC_APB2PCENR (*(volatile uint32_t *)0x40021018u)
#define GPIOC_CFGLR   (*(volatile uint32_t *)0x40011000u)
#define GPIOC_BSHR    (*(volatile uint32_t *)0x40011010u)
#define GPIOC_BCR     (*(volatile uint32_t *)0x40011014u)

#define LED_BIT 4u

static void delay(void) {
    for (volatile uint32_t i = 0; i < 500; ++i) {
        __asm__ volatile("nop");
    }
}

void main(void) {
    RCC_APB2PCENR |= 1u << 4; /* enable GPIOC */
    /* PC4: push-pull output, 2 MHz (CFGLR nibble 4 = 0x2). */
    GPIOC_CFGLR = (GPIOC_CFGLR & ~(0xFu << (LED_BIT * 4u))) | (0x2u << (LED_BIT * 4u));

    for (;;) {
        GPIOC_BSHR = 1u << LED_BIT; /* set PC4 */
        delay();
        GPIOC_BCR = 1u << LED_BIT;  /* reset PC4 */
        delay();
    }
}
