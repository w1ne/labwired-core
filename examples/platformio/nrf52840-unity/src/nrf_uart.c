// LabWired - PlatformIO + LabWired integration example
// nRF52840 UART0 TX driver (register layout per the nRF52840 datasheet).
#include <stdint.h>
#include "nrf_uart.h"

#define UART0_BASE   0x40002000u
#define UART0_ENABLE (*(volatile uint32_t *)(UART0_BASE + 0x500))
#define UART0_TXD    (*(volatile uint32_t *)(UART0_BASE + 0x51C))

void uart_init(void) {
    UART0_ENABLE = 4; // ENABLE = 4 -> UART enabled
}

void uart_putc(char c) {
    UART0_TXD = (uint32_t)(unsigned char)c;
}

void uart_puts(const char *s) {
    while (*s) {
        uart_putc(*s++);
    }
}
