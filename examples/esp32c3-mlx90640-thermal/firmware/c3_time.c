/* ESP32-C3 SYSTIMER UNIT0 + TARGET0, using the C3 register map.
 *
 * Polling a 2 Hz sensor need not consume the CPU between polls. TARGET0 wakes
 * WFI through matrix source 37 / CPU line 7. Global interrupts remain masked:
 * WFI can wake on a locally enabled interrupt without entering a trap handler.
 * The polling loop clears the level after wake. This bare-metal application
 * owns this timer and interrupt line; it does not run an RTOS.
 */
#include "c3_time.h"

#define SYSTIMER_BASE 0x60023000u
#define TIMER_REG(offset) (*(volatile uint32_t *)(SYSTIMER_BASE + (offset)))
#define INTC_BASE 0x600c2000u
#define INTC_REG(offset) (*(volatile uint32_t *)(INTC_BASE + (offset)))
#define TARGET0_ENABLE (1u << 24)
#define CPU_LINE 7u

static uint64_t timer_ticks(void) {
    TIMER_REG(0x04u) = 1u << 30; /* UNIT0_OP.UPDATE */
    while ((TIMER_REG(0x04u) & (1u << 29)) == 0u) {
    }
    uint32_t hi = TIMER_REG(0x40u);
    uint32_t lo = TIMER_REG(0x44u);
    return ((uint64_t)hi << 32) | lo;
}

void c3_time_init(void) {
    __asm__ volatile ("csrci mstatus, 8" ::: "memory");
    TIMER_REG(0x00u) |= (1u << 31) | (1u << 30); /* CLK_EN, UNIT0_WORK_EN */
    TIMER_REG(0x00u) &= ~TARGET0_ENABLE;
    TIMER_REG(0x6cu) = 1u; /* acknowledge any old TARGET0 level */
    INTC_REG(37u * 4u) = CPU_LINE;
    INTC_REG(0x108u) &= ~(1u << CPU_LINE); /* level-sensitive */
    INTC_REG(0x114u + CPU_LINE * 4u) = 1u; /* priority */
    INTC_REG(0x194u) = 1u; /* priority threshold */
    INTC_REG(0x104u) |= 1u << CPU_LINE;
}

uint64_t c3_time_us(void) {
    return timer_ticks() / 16u;
}

void c3_sleep_us(uint32_t delay_us) {
    uint64_t target = timer_ticks() + (uint64_t)delay_us * 16u;
    TIMER_REG(0x00u) &= ~TARGET0_ENABLE;
    TIMER_REG(0x6cu) = 1u;
    TIMER_REG(0x34u) = 0u; /* one-shot, UNIT0 */
    TIMER_REG(0x1cu) = (uint32_t)(target >> 32);
    TIMER_REG(0x20u) = (uint32_t)target;
    TIMER_REG(0x50u) = 1u; /* COMP0_LOAD commits the target */
    TIMER_REG(0x64u) |= 1u;
    TIMER_REG(0x00u) |= TARGET0_ENABLE;
    while ((TIMER_REG(0x68u) & 1u) == 0u) {
        __asm__ volatile ("wfi" ::: "memory");
    }
    TIMER_REG(0x00u) &= ~TARGET0_ENABLE;
    TIMER_REG(0x6cu) = 1u;
}
