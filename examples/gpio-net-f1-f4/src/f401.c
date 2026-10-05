/* STM32F401 side of the gpio-net-f1-f4 example. Register level, no libraries.
 *
 *   irq    PC1  input, EXTI line 1 via SYSCFG_EXTICR1 = port C, both edges
 *   alert  PA8  input with the INTERNAL pull-up (PUPDR = 01), EXTI line 8 via
 *               SYSCFG_EXTICR3 = port A, both edges; the net has no pull of its
 *               own, so this pull-up is what holds the wire high
 *
 * SRAM 0x20000100 (the test reads it):
 *   [0] irq rising   [1] irq falling   [2] alert rising   [3] alert falling
 */
#include <stdint.h>

#define REG(a) (*(volatile uint32_t *)(a))
#define RCC_AHB1ENR    REG(0x40023830u)
#define RCC_APB2ENR    REG(0x40023844u)
#define GPIOA_MODER    REG(0x40020000u)
#define GPIOA_PUPDR    REG(0x4002000Cu)
#define GPIOA_IDR      REG(0x40020010u)
#define GPIOC_MODER    REG(0x40020800u)
#define GPIOC_IDR      REG(0x40020810u)
#define SYSCFG_EXTICR1 REG(0x40013808u)
#define SYSCFG_EXTICR3 REG(0x40013810u)
#define EXTI_IMR       REG(0x40013C00u)
#define EXTI_RTSR      REG(0x40013C08u)
#define EXTI_FTSR      REG(0x40013C0Cu)
#define EXTI_PR        REG(0x40013C14u)
#define NVIC_ISER0     REG(0xE000E100u)
#define RESULT ((volatile uint32_t *)0x20000100u)

static void delay(uint32_t n) {
    for (volatile uint32_t i = 0; i < n; ++i) {}
}

void reset(void);
void default_handler(void) { for (;;) {} }
void exti1_handler(void);
void exti9_5_handler(void);

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 24] = {
    [0] = 0x20010000u,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
    [16 + 7] = (uintptr_t)exti1_handler,    /* EXTI1 */
    [16 + 23] = (uintptr_t)exti9_5_handler, /* EXTI9_5 */
};

void exti1_handler(void) {
    if (!(EXTI_PR & (1u << 1))) return;
    EXTI_PR = 1u << 1;
    if (GPIOC_IDR & (1u << 1)) RESULT[0] += 1; else RESULT[1] += 1;
}

void exti9_5_handler(void) {
    if (!(EXTI_PR & (1u << 8))) return;
    EXTI_PR = 1u << 8;
    if (GPIOA_IDR & (1u << 8)) RESULT[2] += 1; else RESULT[3] += 1;
}

int main(void);
void reset(void) { (void)main(); for (;;) {} }

int main(void) {
    RCC_AHB1ENR |= (1u << 0) | (1u << 2);   /* GPIOA, GPIOC */
    RCC_APB2ENR |= 1u << 14;                /* SYSCFG */

    /* alert: PA8 input with the internal pull-up, then let the wire settle
     * before the edge detector is armed (the pull-up raising it is not one of
     * the AVR's pulses). */
    GPIOA_MODER &= ~(3u << 16);
    GPIOA_PUPDR = (GPIOA_PUPDR & ~(3u << 16)) | (1u << 16);
    /* irq: PC1 input, no pull (the net has a pull-down). */
    GPIOC_MODER &= ~(3u << 2);
    delay(2000);

    SYSCFG_EXTICR1 = 2u << 4;               /* line 1 <- port C */
    SYSCFG_EXTICR3 = 0u;                    /* line 8 <- port A */
    EXTI_RTSR = (1u << 1) | (1u << 8);
    EXTI_FTSR = (1u << 1) | (1u << 8);
    EXTI_IMR = (1u << 1) | (1u << 8);
    NVIC_ISER0 = (1u << 7) | (1u << 23);
    __asm__ volatile("cpsie i" ::: "memory");
    for (;;) {}
}
