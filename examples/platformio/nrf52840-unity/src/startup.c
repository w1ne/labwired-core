// LabWired - PlatformIO + LabWired integration example
// Minimal Cortex-M4 startup for a no-framework (bare-metal) nRF52840 build.
//
// Why hand-rolled: this example deliberately avoids a heavyweight framework so
// the firmware boots instantly and deterministically in the LabWired model.
// Reset_Handler sets the stack pointer explicitly (as vendor startups do),
// initialises .data / .bss, and calls main(). main() is provided either by the
// application (src/main.c, weak) or by the Unity test (test/.../test_main.c,
// strong) depending on what is being built.
#include <stdint.h>

extern uint32_t _sidata; // .data init values in FLASH (LMA)
extern uint32_t _sdata;  // .data start in RAM (VMA)
extern uint32_t _edata;
extern uint32_t _sbss;
extern uint32_t _ebss;
extern uint32_t _estack; // top of stack (provided by linker)

int main(void);
void _start_c(void);

// First instruction out of reset. Naked so the compiler emits no prologue that
// would touch the stack before SP is loaded. We set MSP from the linker symbol
// rather than relying on the loader to pre-load it from the vector table.
__attribute__((naked, noreturn)) void Reset_Handler(void) {
    __asm volatile(
        "ldr r0, =_estack \n"
        "mov sp, r0       \n"
        "bl  _start_c     \n"
        "b   .            \n");
}

void _start_c(void) {
    // Copy .data from FLASH to RAM.
    uint32_t *src = &_sidata;
    uint32_t *dst = &_sdata;
    while (dst < &_edata) {
        *dst++ = *src++;
    }
    // Zero .bss.
    for (dst = &_sbss; dst < &_ebss;) {
        *dst++ = 0;
    }

    main();

    while (1) {
        // Park here once the test runner returns. The LabWired runner stops on
        // its max_steps / wall_time budget after the test output has been
        // streamed to stdout.
    }
}

void Default_Handler(void) {
    while (1) {
    }
}

// Minimal vector table: initial SP + reset, then a few faults pointed at the
// default handler. That is all a polled, interrupt-free test needs.
__attribute__((section(".isr_vector"), used))
const void *const g_vector_table[] = {
    (const void *)&_estack,        // 0x00: Initial Stack Pointer
    (const void *)Reset_Handler,   // 0x04: Reset
    (const void *)Default_Handler, // 0x08: NMI
    (const void *)Default_Handler, // 0x0C: HardFault
    (const void *)Default_Handler, // 0x10: MemManage
    (const void *)Default_Handler, // 0x14: BusFault
    (const void *)Default_Handler, // 0x18: UsageFault
};
