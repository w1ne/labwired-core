/* STM32F103 side of the gpio-net-f1-f4 example. Register level, no libraries.
 *
 *   irq   PB0  floating input, EXTI line 0 via AFIO_EXTICR1 = port B, both edges
 *   wake  PA0  open-drain output, released; the only pull on the wire is the
 *              ATmega328P's internal pull-up on PD5
 *
 * Sequence: count the AVR's 10 irq pulses with the EXTI0 interrupt (rising and
 * falling separately), then pull wake low 4 times (the AVR counts them).
 *
 * SRAM 0x20000100 (the test reads it):
 *   [0] irq rising   [1] irq falling   [2] 1 when the wake pulses are done
 */
#include <stdint.h>

#define REG(a) (*(volatile uint32_t *)(a))
#define RCC_APB2ENR  REG(0x40021018u)
#define GPIOA_CRL    REG(0x40010800u)
#define GPIOA_BSRR   REG(0x40010810u)
#define GPIOA_ODR    REG(0x4001080Cu)
#define GPIOB_CRL    REG(0x40010C00u)
#define AFIO_EXTICR1 REG(0x40010008u)
#define EXTI_IMR     REG(0x40010400u)
#define EXTI_RTSR    REG(0x40010408u)
#define EXTI_FTSR    REG(0x4001040Cu)
#define EXTI_PR      REG(0x40010414u)
#define NVIC_ISER0   REG(0xE000E100u)
#define RESULT ((volatile uint32_t *)0x20000100u)

static void delay(uint32_t n) {
    for (volatile uint32_t i = 0; i < n; ++i) {}
}

void reset(void);
void default_handler(void) { for (;;) {} }
void exti0_handler(void);

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 7] = {
    [0] = 0x20004000u,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
    [16 + 6] = (uintptr_t)exti0_handler, /* EXTI0 */
};

static uint32_t level_pb0(void) { return REG(0x40010C08u) & 1u; }

void exti0_handler(void) {
    if (!(EXTI_PR & 1u)) return;          /* not ours (as HAL checks) */
    EXTI_PR = 1u;                         /* rc_w1 */
    if (level_pb0()) RESULT[0] += 1; else RESULT[1] += 1;
}

int main(void);
void reset(void) { (void)main(); for (;;) {} }

int main(void) {
    RCC_APB2ENR |= (1u << 0) | (1u << 2) | (1u << 3); /* AFIO, GPIOA, GPIOB */

    /* wake: PA0 open-drain output, released before it becomes an output. */
    GPIOA_ODR = 1u;
    GPIOA_CRL = (GPIOA_CRL & ~0xFu) | 0x6u;  /* MODE 10 (2 MHz), CNF 01 (OD) */

    /* irq: PB0 floating input (reset value), EXTI0 on port B, both edges. */
    GPIOB_CRL = (GPIOB_CRL & ~0xFu) | 0x4u;
    AFIO_EXTICR1 = 1u;
    EXTI_RTSR = 1u;
    EXTI_FTSR = 1u;
    EXTI_IMR = 1u;
    NVIC_ISER0 = 1u << 6;
    __asm__ volatile("cpsie i" ::: "memory");

    while (RESULT[0] < 10u || RESULT[1] < 10u) {}

    delay(60000);                         /* the AVR is polling wake by then */
    for (int i = 0; i < 4; ++i) {
        GPIOA_BSRR = 1u << 16;            /* pull low */
        delay(1500);
        GPIOA_BSRR = 1u;                  /* release */
        delay(1500);
    }
    RESULT[2] = 1u;
    for (;;) {}
}
