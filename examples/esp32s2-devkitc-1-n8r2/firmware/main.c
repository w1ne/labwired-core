#include <stdint.h>

#define UART0_FIFO (*(volatile uint32_t *)0x3f400000u)

__attribute__((noreturn)) void main(void) {
    static const char marker[] = "ESP32S2_DEVKITC_N8R2_UART_OK\r\n";
    for (const char *p = marker; *p; ++p) {
        UART0_FIFO = (uint8_t)*p;
    }
    for (;;) {
        __asm__ volatile("nop");
    }
}
