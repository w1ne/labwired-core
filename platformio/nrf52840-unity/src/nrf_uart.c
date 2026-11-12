// LabWired - PlatformIO + LabWired integration example
// nRF52840 legacy UART0 TX driver. Wait for TXDRDY before the next byte.
#include <stdint.h>
#include "nrf_uart.h"

#define UART0_BASE 0x40002000u
#define UART0_TASKS_STARTTX  (*(volatile uint32_t *)(UART0_BASE + 0x008))
#define UART0_EVENTS_TXDRDY (*(volatile uint32_t *)(UART0_BASE + 0x11C))
#define UART0_ENABLE         (*(volatile uint32_t *)(UART0_BASE + 0x500))
#define UART0_PSELTXD        (*(volatile uint32_t *)(UART0_BASE + 0x50C))
#define UART0_TXD            (*(volatile uint32_t *)(UART0_BASE + 0x51C))
#define UART0_BAUDRATE       (*(volatile uint32_t *)(UART0_BASE + 0x524))

void uart_init(void) {
    UART0_PSELTXD = 6;          // nRF52840-DK virtual COM TX pin P0.06
    UART0_BAUDRATE = 0x01D7E000u; // 115200 baud
    UART0_ENABLE = 4;          // Legacy UART (not EasyDMA UARTE)
    UART0_TASKS_STARTTX = 1;   // Required to start transmission
}

void uart_putc(char c) {
    UART0_EVENTS_TXDRDY = 0;
    UART0_TXD = (uint32_t)(uint8_t)c;
    while (UART0_EVENTS_TXDRDY == 0) {}
}

void uart_puts(const char *s) {
    while (*s) {
        uart_putc(*s++);
    }
}
