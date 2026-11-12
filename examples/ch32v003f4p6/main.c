#include <stdint.h>

#define RCC_APB2PCENR (*(volatile uint32_t *)0x40021018u)
#define GPIOC_CFGLR   (*(volatile uint32_t *)0x40011000u)
#define GPIOC_BSHR    (*(volatile uint32_t *)0x40011010u)
#define GPIOC_BCR     (*(volatile uint32_t *)0x40011014u)

static void delay(void) {
    for (volatile uint32_t i = 0; i < 500; ++i) {
        __asm__ volatile("nop");
    }
}

void main(void) {
    RCC_APB2PCENR |= 1u << 4; /* enable GPIOC */
    GPIOC_CFGLR = 0x2u;       /* PC0: push-pull output, 2 MHz */

    for (;;) {
        GPIOC_BSHR = 1u; /* set PC0 */
        delay();
        GPIOC_BCR = 1u;  /* reset PC0 */
        delay();
    }
}
