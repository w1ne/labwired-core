/*
 * LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 * MIT License. See the LICENSE file in the project root.
 *
 * Fault-verdict fixtures: firmware that faults ON PURPOSE, one way per build.
 * Built for the NUCLEO-L476RG (Cortex-M4, flash 0x0800_0000, SRAM1
 * 0x2000_0000). See build.sh. FAULT_KIND selects the fault:
 *
 *   1 hardfault_forced  bad data read, BusFault NOT enabled  -> HardFault (FORCED)
 *   2 busfault          bad data read, BusFault enabled      -> precise BusFault
 *   3 undef             UDF (__builtin_trap), UsageFault off -> HardFault (FORCED)
 *   4 div0              SDIV by zero, CCR.DIV_0_TRP + USGFAULTENA -> UsageFault
 *   5 lockup            bad read, then a bad read inside HardFault -> LOCKUP
 *
 * The fault is always inside `sensor_read`, called from `main`, so the verdict
 * can name both.
 */
#include <stdint.h>

#define SCB_CCR   (*(volatile uint32_t *)0xE000ED14u)
#define SCB_SHCSR (*(volatile uint32_t *)0xE000ED24u)
#define CCR_DIV_0_TRP      (1u << 4)
#define SHCSR_BUSFAULTENA  (1u << 17)
#define SHCSR_USGFAULTENA  (1u << 18)

/* No memory and no peripheral is mapped here on the STM32L476. */
#define UNMAPPED_ADDR 0x30000004u

extern uint32_t _estack;
void Reset_Handler(void);
void HardFault_Handler(void);
void MemManage_Handler(void);
void BusFault_Handler(void);
void UsageFault_Handler(void);

volatile uint32_t g_divisor = 0;
volatile uint32_t g_sink;
volatile uint32_t g_last;
volatile uint32_t g_handler_hits;

__attribute__((noinline)) uint32_t sensor_read(uint32_t channel)
{
#if FAULT_KIND == 3
    if (channel == 2u) {
        __builtin_trap();
    }
    return channel;
#elif FAULT_KIND == 4
    int32_t scaled = (int32_t)(1000 + channel) / (int32_t)g_divisor; /* faults */
    g_last = (uint32_t)scaled;
    return g_last;
#elif FAULT_KIND != 6
    volatile uint32_t *reg = (volatile uint32_t *)(UNMAPPED_ADDR + channel * 0u);
    uint32_t raw = *reg; /* faults */
    g_last = raw;
    return g_last;
#else
    /* 0x0800_1000 is inside flash; the fixture chip sets cpu_writes: fault. */
    (void)channel;
    *(volatile uint32_t *)0x08001000u = 0xDEADBEEFu; /* faults */
    return 0;
#endif
/* FAULT_KIND 6 is appended after the old branches so the old kinds keep
 * their exact source lines: the committed old-kind ELFs and their golden
 * sentences pin them (e.g. the faulting load in `sensor_read` is line 55).
 * The #line restores the numbering the new branch pushed down, which keeps
 * rebuilding this file from churning those ELFs. */
#line 59 "fault_fixture.c"
}

int main(void)
{
#if FAULT_KIND == 2 || FAULT_KIND == 6
    SCB_SHCSR |= SHCSR_BUSFAULTENA;
#elif FAULT_KIND == 4
    SCB_CCR |= CCR_DIV_0_TRP;
    SCB_SHCSR |= SHCSR_USGFAULTENA;
#endif
    g_sink = sensor_read(2u);
    for (;;) {
    }
}

static void spin(void)
{
    g_handler_hits++;
    for (;;) {
    }
}

void HardFault_Handler(void)
{
#if FAULT_KIND == 5
    /* A fault while the HardFault handler runs cannot be taken: LOCKUP. */
    g_sink = *(volatile uint32_t *)UNMAPPED_ADDR;
#endif
    spin();
}
void MemManage_Handler(void) { spin(); }
void BusFault_Handler(void) { spin(); }
void UsageFault_Handler(void) { spin(); }

void Reset_Handler(void)
{
    main();
}

__attribute__((section(".isr_vector"), used)) void (*const g_vectors[16])(void) = {
    (void (*)(void))(&_estack),
    Reset_Handler,
    spin,               /* NMI */
    HardFault_Handler,
    MemManage_Handler,
    BusFault_Handler,
    UsageFault_Handler,
};
