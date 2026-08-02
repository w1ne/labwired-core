/* ESP32-C3 polled debug UART — see c3_uart.h. */
#include "c3_uart.h"

#define UART0_TX (*(volatile uint8_t *)0x60000000u)

void uart_putc(char c) { UART0_TX = (uint8_t)c; }

void uart_puts(const char *s) {
    while (*s) {
        uart_putc(*s++);
    }
}

void uart_puti(int32_t v) {
    char buf[12];
    int i = 0;
    uint32_t u;
    if (v < 0) {
        uart_putc('-');
        u = (uint32_t)(-(int64_t)v);
    } else {
        u = (uint32_t)v;
    }
    if (u == 0) {
        uart_putc('0');
        return;
    }
    while (u > 0) {
        buf[i++] = (char)('0' + (u % 10u));
        u /= 10u;
    }
    while (i > 0) {
        uart_putc(buf[--i]);
    }
}

void uart_putfix2(int32_t v_x100) {
    int32_t whole;
    int32_t frac;
    if (v_x100 < 0) {
        uart_putc('-');
        v_x100 = -v_x100;
    }
    whole = v_x100 / 100;
    frac = v_x100 % 100;
    uart_puti(whole);
    uart_putc('.');
    uart_putc((char)('0' + (frac / 10)));
    uart_putc((char)('0' + (frac % 10)));
}

void uart_puthex(uint32_t n, int width) {
    static const char hexd[] = "0123456789ABCDEF";
    int i;
    for (i = (width - 1) * 4; i >= 0; i -= 4) {
        uart_putc(hexd[(n >> i) & 0xFu]);
    }
}
