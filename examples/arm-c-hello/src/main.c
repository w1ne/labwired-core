#include <stdint.h>

extern uint32_t _estack; // Defined in linker script if we want, but let's use a constant for now

#define UART_BASE 0x4000C000
#define UART_DR   (*(volatile uint32_t *)(UART_BASE + 0x00))
#define UART_FR   (*(volatile uint32_t *)(UART_BASE + 0x18))

void _start(void);

// Vector table
__attribute__((section(".vectors")))
const void* vectors[] = {
    (void*)0x20010000, // Initial SP (End of RAM)
    (void*)_start,      // Reset Handler
};

void uart_putc(char c) {
    // Basic UART write for LabWired
    UART_DR = c;
}

void uart_puts(const char *s) {
    while (*s) {
        uart_putc(*s++);
    }
}

void main(void) {
    uart_puts("Hello from LabWired C Example!\n");
    uart_puts("This is running on a simulated ARM Cortex-M0.\n");
    
    while (1) {
        // Spin
        for (int i = 0; i < 100000; i++) {
            __asm__("nop");
        }
        uart_puts("Pulse...\n");
    }
}

// Minimal startup
void _start(void) {
    main();
}
