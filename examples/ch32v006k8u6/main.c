#include <stdint.h>

/* Register addresses and bits: WCH ch32v00X.h (openwch/ch32v002_004_005_006_007). */
#define RCC_APB2PCENR (*(volatile uint32_t *)0x40021018u)
#define GPIOB_CFGLR   (*(volatile uint32_t *)0x40010C00u)
#define GPIOB_BSHR    (*(volatile uint32_t *)0x40010C10u)
#define GPIOB_BCR     (*(volatile uint32_t *)0x40010C14u)

static void delay(void) {
    for (volatile uint32_t i = 0; i < 500; ++i) {
        __asm__ volatile("nop");
    }
}

void main(void) {
    RCC_APB2PCENR |= 1u << 3; /* enable GPIOB, the port the CH32V003 lacks */
    GPIOB_CFGLR = (GPIOB_CFGLR & ~0xFu) | 0x2u; /* PB0: push-pull output, 2 MHz */

    for (;;) {
        GPIOB_BSHR = 1u; /* set PB0 */
        delay();
        GPIOB_BCR = 1u;  /* reset PB0 */
        delay();
    }
}
