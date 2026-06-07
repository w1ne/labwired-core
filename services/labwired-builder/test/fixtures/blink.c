/*
 * Minimal blink for STM32L476 — toggles GPIOA pin 5 (PA5, the LD2 LED on
 * NUCLEO-L476RG) by writing directly to the GPIO ODR register.
 *
 * Memory map (RM0351):
 *   GPIOA base  = 0x48000000 (confirmed in core/configs/chips/stm32l476.yaml)
 *   GPIOA_MODER = base + 0x00
 *   GPIOA_ODR   = base + 0x14
 *
 * This file is intentionally self-contained — no CMSIS headers, no HAL.
 * It only needs to COMPILE and produce a short run under the LabWired sim.
 */

#define GPIOA_BASE   0x48000000UL
#define GPIOA_MODER  (*(volatile unsigned int *)(GPIOA_BASE + 0x00U))
#define GPIOA_ODR    (*(volatile unsigned int *)(GPIOA_BASE + 0x14U))

#define PIN5_MASK    (1U << 5)

/* Configure PA5 as output (MODER bits [11:10] = 01) */
static void gpio_init(void) {
    GPIOA_MODER &= ~(3U << 10);  /* clear bits 11:10 */
    GPIOA_MODER |=  (1U << 10);  /* set output mode */
}

static void delay(volatile unsigned int n) {
    while (n--) {
        __asm__ volatile("nop");
    }
}

int main(void) {
    gpio_init();

    /* Toggle PA5 a fixed number of times so the sim terminates */
    for (int i = 0; i < 10; i++) {
        GPIOA_ODR |=  PIN5_MASK;  /* LED on  */
        delay(500);
        GPIOA_ODR &= ~PIN5_MASK;  /* LED off */
        delay(500);
    }

    return 0;
}
