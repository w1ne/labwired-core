/**
 * Arduino main() wrapper for LabWired simulator.
 * Links with user's setup()/loop() sketch.
 */
#include "Arduino.h"

/* Forward-declare user's setup() and loop() */
extern void setup(void);
extern void loop(void);

/* Forward-declare _start before vector table reference */
void _start(void) __attribute__((noreturn));

/* Vector table */
__attribute__((section(".vectors")))
const void* _vector_table[] = {
    (void*)0x20020000,  /* Initial SP (top of 128KB RAM) */
    (void*)_start,      /* Reset handler */
};

void _start(void) {
    /* Enable GPIO and peripheral clocks */
    RCC_APB2ENR |= (1 << 2) | (1 << 3) | (1 << 4) | (1 << 14); /* GPIOA,B,C + USART1 */

    setup();
    for (;;) {
        loop();
    }
}
