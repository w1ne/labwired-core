#include <stdint.h>

extern uint32_t _sidata, _sdata, _edata, _sbss, _ebss, _estack;
extern int main(void);

void Default_Handler(void)
{
    for (;;) {
    }
}

void Reset(void)
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

