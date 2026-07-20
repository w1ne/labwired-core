/*
 * Cortex-M33 startup for the nRF54L15 application core.
 *
 * Deliberately dependency-free: no CMSIS, no vendor SDK, no libc startup. The
 * point of the onboarding smoke firmware is to exercise the *chip profile*, so
 * the less code between reset and the first UARTE byte, the more precisely a
 * failure localises to the simulator rather than to a vendor HAL.
 *
 * The vector table is placed at RRAM base (0x0) by the linker script. On
 * nRF54L the NVM is RRAM rather than flash, but from the core's point of view
 * reset behaviour is identical: load SP from word 0, PC from word 1.
 */
#include <stdint.h>

extern int main(void);

/* Provided by the linker script. */
extern uint32_t _sidata, _sdata, _edata, _sbss, _ebss, _estack;

void Reset_Handler(void);

static void Default_Handler(void)
{
    /* Spin rather than reset: a simulator run that lands here stops with a
     * recognisable PC instead of looping through reset forever. */
    for (;;) {
    }
}

void NMI_Handler(void)        __attribute__((weak, alias("Default_Handler")));
void HardFault_Handler(void)  __attribute__((weak, alias("Default_Handler")));
void MemManage_Handler(void)  __attribute__((weak, alias("Default_Handler")));
void BusFault_Handler(void)   __attribute__((weak, alias("Default_Handler")));
void UsageFault_Handler(void) __attribute__((weak, alias("Default_Handler")));
void SVC_Handler(void)        __attribute__((weak, alias("Default_Handler")));
void DebugMon_Handler(void)   __attribute__((weak, alias("Default_Handler")));
void PendSV_Handler(void)     __attribute__((weak, alias("Default_Handler")));
void SysTick_Handler(void)    __attribute__((weak, alias("Default_Handler")));

/*
 * ARMv8-M exception table. Only the architectural entries are populated; the
 * nRF54L15 has ~270 external IRQs but the smoke firmware runs entirely
 * polled, so an empty external vector region is honest rather than a
 * placeholder table that pretends handlers exist.
 */
/*
 * Entry 0 is the initial stack pointer, not a function pointer, so the table is
 * typed as a union rather than cast through a function-pointer type (which ISO
 * C forbids for object pointers, and -Wpedantic correctly rejects).
 */
typedef union {
    void (*handler)(void);
    uint32_t *stack_top;
} vector_entry_t;

__attribute__((section(".isr_vector"), used))
const vector_entry_t g_vectors[] = {
    { .stack_top = &_estack },
    { .handler = Reset_Handler },
    { .handler = NMI_Handler },
    { .handler = HardFault_Handler },
    { .handler = MemManage_Handler },
    { .handler = BusFault_Handler },
    { .handler = UsageFault_Handler },
    { .handler = 0 }, { .handler = 0 }, { .handler = 0 }, { .handler = 0 },
    { .handler = SVC_Handler },
    { .handler = DebugMon_Handler },
    { .handler = 0 },
    { .handler = PendSV_Handler },
    { .handler = SysTick_Handler },
};

void Reset_Handler(void)
{
    uint32_t *src, *dst;

    /* .data: RRAM -> RAM */
    src = &_sidata;
    for (dst = &_sdata; dst < &_edata; ) {
        *dst++ = *src++;
    }

    /* .bss: zero */
    for (dst = &_sbss; dst < &_ebss; ) {
        *dst++ = 0;
    }

    (void)main();

    for (;;) {
    }
}
