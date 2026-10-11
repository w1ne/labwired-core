/* Cortex-M3 start-up for the STM32F103C8 (Blue Pill): vector table with the
 * interrupts this example uses, .data copy, .bss clear, then main().
 * Every handler not defined by the firmware falls back to Default_Handler. */
#include <stdint.h>

extern uint32_t _sidata, _sdata, _edata, _sbss, _ebss, _estack;
extern int main(void);

void Default_Handler(void)
{
    for (;;) {
    }
}

#define WEAK_DEFAULT __attribute__((weak, alias("Default_Handler")))

void NMI_Handler(void) WEAK_DEFAULT;
void HardFault_Handler(void) WEAK_DEFAULT;
void USB_LP_CAN1_RX0_IRQHandler(void) WEAK_DEFAULT; /* IRQ 20 */
void TIM2_IRQHandler(void) WEAK_DEFAULT;            /* IRQ 28 */
void TIM3_IRQHandler(void) WEAK_DEFAULT;            /* IRQ 29 */

__attribute__((used, noreturn)) static void Reset_C(void)
{
    const uint32_t *src = &_sidata;
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

typedef void (*vector_t)(void);

/* (-Wno-override-init in the Makefile: the range fill is overridden on purpose.)
 * 16 core exceptions + 30 external IRQs (up to TIM3 = IRQ 29). */
__attribute__((section(".isr_vector"), used)) const vector_t g_vectors[16 + 30] = {
    [0 ... 16 + 29] = Default_Handler, /* then the named ones override it */
    [0] = (vector_t) &_estack,
    [1] = Reset,
    [2] = NMI_Handler,
    [3] = HardFault_Handler,
    [16 + 20] = USB_LP_CAN1_RX0_IRQHandler,
    [16 + 28] = TIM2_IRQHandler,
    [16 + 29] = TIM3_IRQHandler,
};
