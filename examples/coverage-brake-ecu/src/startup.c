// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

// Minimal startup for Cortex-M (vector table) and RISC-V (_start).

#include <stdint.h>

int main(void);
extern uint32_t _stack_top;

#if defined(__arm__)
void Reset_Handler(void) {
    main();
    for (;;) {
    }
}

void Default_Handler(void) {
    for (;;) {
    }
}

__attribute__((section(".vectors"), used)) const void *const vectors[] = {
    &_stack_top,     // initial SP
    Reset_Handler,   // reset
    Default_Handler, // NMI
    Default_Handler, // HardFault
};
#elif defined(__riscv)
__attribute__((naked, section(".text.start"))) void _start(void) {
    __asm__ volatile("la sp, _stack_top\n"
                     "j main\n");
}
#endif
