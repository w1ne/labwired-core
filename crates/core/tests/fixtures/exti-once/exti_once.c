/* EXTI handler-entry count fixture: one firmware, one build per chip family.
 *
 * PA0 is a plain input; EXTI line 0 is armed on both edges and unmasked. The
 * handler clears the pending bit WITHOUT checking it first and counts. On
 * silicon writing 1 to EXTI_PR (RPR1/FPR1 on G0/U5) drops the NVIC line, and
 * the NVIC pending bit was cleared on exception entry, so the handler runs
 * exactly once per edge: COUNT == the number of edges the harness drove.
 *
 * SRAM 0x20000100 (the test reads it):
 *   [0] handler entries   [1] 0x600DF00D once EXTI and the NVIC are armed
 *
 * Built by build.sh with one of -DCHIP_F1 / -DCHIP_F4 / -DCHIP_G0 / -DCHIP_L0 /
 * -DCHIP_U5.
 */
#include <stdint.h>

#define REG(a) (*(volatile uint32_t *)(a))
#define RESULT ((volatile uint32_t *)0x20000100u)
#define NVIC_ISER0 REG(0xE000E100u)

#if defined(CHIP_F1)
/* RM0008: AFIO_EXTICR1 reset = port A; EXTI0 is IRQ 6. */
#define EXTI 0x40010400u
#define EXTI_IRQ 6
static void clocks(void) { REG(0x40021018u) |= (1u << 0) | (1u << 2); } /* AFIO, IOPA */
#elif defined(CHIP_F4)
/* RM0368: SYSCFG_EXTICR1 reset = port A; EXTI0 is IRQ 6. */
#define EXTI 0x40013C00u
#define EXTI_IRQ 6
static void clocks(void) {
    REG(0x40023830u) |= 1u << 0;  /* AHB1ENR.GPIOAEN */
    REG(0x40023844u) |= 1u << 14; /* APB2ENR.SYSCFGEN */
}
#elif defined(CHIP_G0)
/* RM0444: EXTI_EXTICR1 reset = port A; EXTI0_1 is IRQ 5. */
#define EXTI 0x40021800u
#define EXTI_IRQ 5
#define SPLIT_PENDING 1
static void clocks(void) { REG(0x40021034u) |= 1u << 0; } /* IOPENR.GPIOAEN */
#elif defined(CHIP_L0)
/* RM0367: SYSCFG_EXTICR1 reset = port A; EXTI0_1 is IRQ 5. The register
 * file is the F1 one (PR at 0x14), the vectors the Cortex-M0+ groups. */
#define EXTI 0x40010400u
#define EXTI_IRQ 5
static void clocks(void) {
    REG(0x4002102Cu) |= 1u << 0; /* IOPENR.IOPAEN */
    REG(0x40021034u) |= 1u << 0; /* APB2ENR.SYSCFGEN */
}
#elif defined(CHIP_U5)
/* RM0456: EXTI_EXTICR1 reset = port A; EXTI0 is IRQ 11. */
#define EXTI 0x46022000u
#define EXTI_IRQ 11
#define SPLIT_PENDING 1
static void clocks(void) { REG(0x46020C8Cu) |= 1u << 0; } /* AHB2ENR1.GPIOAEN */
#else
#error "build with -DCHIP_F1, -DCHIP_F4, -DCHIP_G0, -DCHIP_L0 or -DCHIP_U5"
#endif

#ifdef SPLIT_PENDING
#define EXTI_RTSR REG(EXTI + 0x00u)
#define EXTI_FTSR REG(EXTI + 0x04u)
#define EXTI_RPR  REG(EXTI + 0x0Cu)
#define EXTI_FPR  REG(EXTI + 0x10u)
#define EXTI_IMR  REG(EXTI + 0x80u)
static void clear_pending(void) { EXTI_RPR = 1u; EXTI_FPR = 1u; }
#else
#define EXTI_IMR  REG(EXTI + 0x00u)
#define EXTI_RTSR REG(EXTI + 0x08u)
#define EXTI_FTSR REG(EXTI + 0x0Cu)
#define EXTI_PR   REG(EXTI + 0x14u)
static void clear_pending(void) { EXTI_PR = 1u; }
#endif

void reset(void);
void default_handler(void) { for (;;) {} }
void exti_handler(void) {
    clear_pending(); /* no check of the pending register first */
    RESULT[0] += 1;
}

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 16] = {
    [0] = 0x20004000u,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
    [16 + EXTI_IRQ] = (uintptr_t)exti_handler,
};

void reset(void) {
    clocks();
    RESULT[0] = 0;
    EXTI_RTSR = 1u;
    EXTI_FTSR = 1u;
    EXTI_IMR = 1u;
    NVIC_ISER0 = 1u << EXTI_IRQ;
    __asm__ volatile("cpsie i" ::: "memory");
    RESULT[1] = 0x600DF00Du;
    for (;;) {}
}
