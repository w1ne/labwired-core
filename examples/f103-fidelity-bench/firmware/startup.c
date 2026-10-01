#include <stdint.h>

extern uint32_t _sidata, _sdata, _edata, _sbss, _ebss, _estack;
extern int main(void);

/* HardFault and the unused vectors land here. */
void Default_Handler(void)
{
    for (;;) {
    }
}

__attribute__((used, noreturn)) static void Reset_C(void)
{
    uint32_t *src = &_sidata;
    uint32_t *dst = &_sdata;
    while (dst < &_edata) {
        *dst++ = *src++;
    }
    for (dst = &_sbss; dst < &_ebss;) {
        *dst++ = 0u;
    }
    /* The image is linked at 0x08000000. Point VTOR there so a taken IRQ
     * reads this table, not the unset boot alias at 0. */
    *(volatile uint32_t *) 0xE000ED08u = 0x08000000u;
    (void) main();
    for (;;) {
    }
}

__attribute__((naked, used, noreturn)) void Reset(void)
{
    __asm volatile(
        "ldr r0, =_estack\n"
        "mov sp, r0\n"
        "bl  Reset_C\n"
        "b   .\n");
}

void Bench_IRQ0(void);
void Bench_HardFault(void);

__attribute__((section(".isr_vector"), used)) void (*const g_vectors[])(void) = {
    (void (*)(void)) &_estack,
    Reset,
    Default_Handler, /* NMI */
    Bench_HardFault, /* HardFault */
    Default_Handler, /* MemManage */
    Bench_HardFault, /* BusFault  */
    Default_Handler, /* UsageFault */
    0,
    0,
    0,
    0,
    Default_Handler, /* SVCall */
    Default_Handler, /* DebugMon */
    0,
    Default_Handler, /* PendSV */
    Default_Handler, /* SysTick */
    Bench_IRQ0,      /* IRQ0 (exception 16) */
};
