/* ESP32-C3 Super Mini blinky.
 *
 * Toggles GPIO8 — the Super Mini's user LED — via the plain R/W GPIO_OUT and
 * GPIO_ENABLE registers, logging each transition over UART0. Bare-metal
 * rv32imc; the LabWired simulator boots the ELF directly (no boot ROM). The
 * C3's gpio block is the declarative register file, so the demo drives the
 * full-value registers (silicon-valid) rather than the W1TS/W1TC aliases.
 */
#include <stdint.h>

#include "c3_uart.h"

#define GPIO_BASE 0x60004000u
#define GPIO_OUT (*(volatile uint32_t *)(GPIO_BASE + 0x04u))
#define GPIO_ENABLE (*(volatile uint32_t *)(GPIO_BASE + 0x20u))

#define LED_PIN 8u

static void delay(void) {
    for (volatile uint32_t i = 0; i < 20000u; i++) {
    }
}

int main(void) {
    uart_puts("C3 BLINKY BOOT\n");
    GPIO_ENABLE |= 1u << LED_PIN;
    for (;;) {
        GPIO_OUT |= 1u << LED_PIN;
        uart_puts("LED ON\n");
        delay();
        GPIO_OUT &= ~(1u << LED_PIN);
        uart_puts("LED OFF\n");
        delay();
    }
}
