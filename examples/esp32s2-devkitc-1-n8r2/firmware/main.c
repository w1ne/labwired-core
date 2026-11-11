// Bare-metal ESP32-S2 UART0 smoke. The native twin fast-starts this ELF into
// the S2 IROM/DROM windows; it does not run the mask ROM or flash bootloader.
#include <stdint.h>

#define UART0_FIFO (*(volatile uint32_t *)0x60000000u)
#define UART0_CLKDIV (*(volatile uint32_t *)0x60000014u)

__attribute__((noreturn)) void _start(void) {
    // 80 MHz UART source / 694 = approximately 115200 baud.
    UART0_CLKDIV = 694u;
    static const char marker[] = "S2_UART_OK\n";
    for (unsigned i = 0; i < sizeof(marker) - 1; ++i) {
        UART0_FIFO = (uint32_t)(uint8_t)marker[i];
    }
    for (;;) {
        __asm__ volatile("nop");
    }
}
