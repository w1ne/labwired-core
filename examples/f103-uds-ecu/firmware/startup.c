#include <stdint.h>

extern uint32_t _sidata, _sdata, _edata, _sbss, _ebss, _estack;
extern int main(void);

void Default_Handler(void)
{
    for (;;) {
    }
}

/* C body of the reset handler — runs with a valid stack guaranteed by Reset(). */
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

/* Hardware normally loads MSP from vector[0] on reset; set it explicitly so the
 * example is robust regardless of how the loader seeds the initial SP. */
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
    Default_Handler,
    Default_Handler,
    Default_Handler,
    Default_Handler,
    Default_Handler,
    0,
    0,
    0,
    0,
    Default_Handler,
    Default_Handler,
    0,
    Default_Handler,
    Default_Handler,
};
