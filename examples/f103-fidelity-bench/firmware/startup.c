#include <stdint.h>

extern uint32_t _sidata, _sdata, _edata, _sbss, _ebss, _estack;
extern int main(void);

/* HardFault and the unused vectors land here. On real silicon the rambug store
 * past the end of SRAM escalates to HardFault, which traps here forever — so the
 * BENCH_RAM_OK marker never prints. */
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

__attribute__((section(".isr_vector"), used)) void (*const g_vectors[16])(void) = {
    (void (*)(void)) &_estack,
    Reset,
    Default_Handler, /* NMI */
    Default_Handler, /* HardFault */
    Default_Handler, /* MemManage */
    Default_Handler, /* BusFault  */
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
};
