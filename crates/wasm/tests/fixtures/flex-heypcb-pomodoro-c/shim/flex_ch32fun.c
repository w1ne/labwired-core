/* SPDX-License-Identifier: MIT
 *
 * Runs firmware written for the ch32fun API as portable LabWired Flex
 * firmware. The firmware source is compiled unchanged; this file stands in
 * for the CH32V003 and its ch32fun support code.
 *
 * What this file assumes:
 *  - Code takes no virtual time, with three exceptions. Delay_Us and Delay_Ms
 *    take the time they name. Each use of `SysTick` outside the SysTick
 *    handler takes FLEX_SYSTICK_ACCESS_US. A touch reading takes none.
 *  - SysTick counts 48 MHz (STCLK set) or 6 MHz from the write that sets STE.
 *    A compare match sets SR bit 0 and, with STIE set and the interrupt
 *    enabled, calls SysTick_Handler between two firmware statements. The
 *    match is seen at the end of a time step of at most 1 ms.
 *  - ReadTouchPin returns FLEX_TOUCH_OPEN counts for each iteration with the
 *    bound Flex port low and FLEX_TOUCH_CONTACT counts with it high. The
 *    charge-time measurement of the ADC is not simulated.
 *  - A pin that the test bench did not bind is not connected: a write to it
 *    does nothing. Pin modes, pull resistors and clock gates do nothing.
 */
#include "ch32fun.h"
#include "ch32v00x_touch.h"

#define FLEX_SYSTICK_ACCESS_US 5
#define FLEX_TOUCH_OPEN 100
#define FLEX_TOUCH_CONTACT 200

__attribute__((import_module("labwired"), import_name("read"))) double labwired_read(uint32_t port);
__attribute__((import_module("labwired"), import_name("write"))) void labwired_write(uint32_t port, double value);
__attribute__((import_module("labwired"), import_name("advance_ns"))) void labwired_advance_ns(uint32_t delta);

void SysTick_Handler(void);

GPIO_TypeDef flex_gpio[4] = { { 0 }, { 1 }, { 2 }, { 3 } };
RCC_TypeDef flex_rcc;

/* Flex port id + 1 of each pin; 0 is a pin that is not bound. */
static uint32_t output_port[FLEX_PIN_COUNT];
static uint32_t touch_port[FLEX_PIN_COUNT];
/* Last level written + 1: a write of the level a pin already has is no edge. */
static uint8_t output_level[FLEX_PIN_COUNT];
static uint32_t bound;

static SysTick_Type systick;
static uint8_t systick_irq_enabled;
static uint8_t in_systick_handler;

/* The test bench connects MCU pin `pin` to the Flex write port `port`.
 * Returns the number of pins bound so far. */
__attribute__((export_name("flex_bind_output"))) uint32_t flex_bind_output(uint32_t pin, uint32_t port)
{
    if (pin >= FLEX_PIN_COUNT)
        __builtin_trap();
    output_port[pin] = port + 1;
    return ++bound;
}

/* The test bench connects the touch pad on MCU pin `pin` to the Boolean Flex
 * read port `port`. Returns the number of pins bound so far. */
__attribute__((export_name("flex_bind_touch"))) uint32_t flex_bind_touch(uint32_t pin, uint32_t port)
{
    if (pin >= FLEX_PIN_COUNT)
        __builtin_trap();
    touch_port[pin] = port + 1;
    return ++bound;
}

static void run_for_us(uint32_t us)
{
    while (us) {
        uint32_t step = us > 1000 ? 1000 : us;
        labwired_advance_ns(step * 1000);
        us -= step;
        if (systick.CTLR & SYSTICK_CTLR_STE) {
            uint32_t counts = step * ((systick.CTLR & SYSTICK_CTLR_STCLK) ? 48 : 6);
            uint32_t to_match = systick.CMP - systick.CNT;
            systick.CNT += counts;
            if (to_match - 1 < counts)
                systick.SR |= 1;
        }
        if ((systick.SR & 1) && (systick.CTLR & SYSTICK_CTLR_STIE) && systick_irq_enabled
            && !in_systick_handler) {
            in_systick_handler = 1;
            SysTick_Handler();
            in_systick_handler = 0;
        }
    }
}

SysTick_Type *flex_systick(void)
{
    if (!in_systick_handler)
        run_for_us(FLEX_SYSTICK_ACCESS_US);
    return &systick;
}

void NVIC_EnableIRQ(int irq)
{
    if (irq == SysTick_IRQn)
        systick_irq_enabled = 1;
}

void SystemInit(void) {}
void funGpioInitAll(void) {}
void funPinMode(int pin, int mode)
{
    (void)pin;
    (void)mode;
}

void funDigitalWrite(int pin, int level)
{
    uint8_t stored = level ? 2 : 1;
    if ((unsigned)pin >= FLEX_PIN_COUNT || !output_port[pin] || output_level[pin] == stored)
        return;
    output_level[pin] = stored;
    labwired_write(output_port[pin] - 1, level ? 1.0 : 0.0);
}

void Delay_Us(uint32_t us) { run_for_us(us); }

void InitTouchADC(void) {}

uint32_t ReadTouchPin(GPIO_TypeDef *io, int portpin, int adcno, int iterations)
{
    (void)adcno;
    uint32_t pin = FLEX_PIN(io->index, portpin);
    if (pin >= FLEX_PIN_COUNT || !touch_port[pin])
        return 0;
    uint32_t counts = labwired_read(touch_port[pin] - 1) != 0.0 ? FLEX_TOUCH_CONTACT : FLEX_TOUCH_OPEN;
    return counts * (uint32_t)iterations;
}
