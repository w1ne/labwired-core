#include <stdint.h>
extern uint32_t __stack_top, __bss_start__, __bss_end__, __data_start__, __data_end__, __data_load__;
int main(void);
void Default_Handler(void) { for (;;) { __asm volatile ("wfi"); } }
void Reset_Handler(void) {
    uint32_t *src = &__data_load__;
    for (uint32_t *dst = &__data_start__; dst < &__data_end__;) *dst++ = *src++;
    for (uint32_t *dst = &__bss_start__; dst < &__bss_end__;) *dst++ = 0;
    (void)main();
    for (;;) {}
}
/* Cortex-M3 vectors; no interrupts are used in this deterministic polling fixture. */
__attribute__((used, section(".isr_vector")))
const uintptr_t vectors[16] = {
    (uintptr_t)&__stack_top, (uintptr_t)Reset_Handler,
    (uintptr_t)Default_Handler, (uintptr_t)Default_Handler,
    (uintptr_t)Default_Handler, (uintptr_t)Default_Handler,
    (uintptr_t)Default_Handler, 0, 0, 0, 0,
    (uintptr_t)Default_Handler, (uintptr_t)Default_Handler,
    0, (uintptr_t)Default_Handler, (uintptr_t)Default_Handler
};
