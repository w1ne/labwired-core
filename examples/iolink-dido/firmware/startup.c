/* Minimal STM32L476 (Cortex-M4) startup: vector table + Reset that
 * initialises .data/.bss and calls main(). Self-contained (no vendor SDK). */
#include <stdint.h>

extern uint32_t _sidata, _sdata, _edata, _sbss, _ebss, _estack;
extern int main(void);

void Default_Handler(void) { for (;;) {} }

void Reset(void) {
    /* Copy .data from flash to RAM. */
    uint32_t *src = &_sidata, *dst = &_sdata;
    while (dst < &_edata) {
        *dst++ = *src++;
    }
    /* Zero .bss. */
    for (dst = &_sbss; dst < &_ebss; ) {
        *dst++ = 0u;
    }
    main();
    for (;;) {}
}

/* Cortex-M vector table: [0]=initial SP, [1]=Reset, faults -> Default_Handler. */
__attribute__((section(".isr_vector"), used))
void (*const g_vectors[16])(void) = {
    (void (*)(void))&_estack, /* 0: Initial stack pointer */
    Reset,                    /* 1: Reset */
    Default_Handler,          /* 2: NMI */
    Default_Handler,          /* 3: HardFault */
    Default_Handler,          /* 4: MemManage */
    Default_Handler,          /* 5: BusFault */
    Default_Handler,          /* 6: UsageFault */
    0, 0, 0, 0,               /* 7-10: reserved */
    Default_Handler,          /* 11: SVCall */
    Default_Handler,          /* 12: Debug */
    0,                        /* 13: reserved */
    Default_Handler,          /* 14: PendSV */
    Default_Handler,          /* 15: SysTick */
};
