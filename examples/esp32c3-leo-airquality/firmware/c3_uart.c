/* ESP32-C3 polled debug UART — see c3_uart.h. */
#include "c3_uart.h"

/* UART0 register map (ESP32-C3 TRM §UART, soc/uart_reg.h). */
#define UART0_BASE 0x60000000u
#define UART_REG(o) (*(volatile uint32_t *)(UART0_BASE + (o)))

#define UART_FIFO   UART_REG(0x00u)
#define UART_CLKDIV UART_REG(0x14u)
#define UART_STATUS UART_REG(0x1Cu)

/* STATUS.TXFIFO_CNT[25:16] — entries currently queued in the TX FIFO. */
#define UART_TXFIFO_CNT_SHIFT 16
#define UART_TXFIFO_CNT_MASK  0x3FFu
/* SOC_UART_FIFO_LEN: the TX FIFO holds 128 bytes and drops writes past that. */
#define UART_TXFIFO_LEN 128u

/* Console baud. The UART divides its APB source clock (80 MHz) by CLKDIV's
 * integer part (bits [11:0]); 80 MHz / 40 = 2 Mbaud, a standard high-rate
 * console setting well inside the block's 5 Mbaud ceiling. The power-on value
 * (694 -> ~115200 baud) is ~17x slower, and this firmware prints ~24 kB per
 * run — ~2.1 s of wire time — so the console is raised here exactly as
 * a real bring-up firmware that streams a framebuffer dump would. */
#define UART_SCLK_HZ   80000000u
#define UART_BAUD      2000000u
#define UART_CLKDIV_VAL (UART_SCLK_HZ / UART_BAUD)

void uart_init(void) { UART_CLKDIV = UART_CLKDIV_VAL; }

void uart_putc(char c) {
    /* Real TX flow control. A write to a full FIFO is dropped by the hardware
     * (the bus write still completes — there is no back-pressure), so wait for
     * a free entry first. This is the whole reason output longer than the
     * 128-byte FIFO survives. */
    while (((UART_STATUS >> UART_TXFIFO_CNT_SHIFT) & UART_TXFIFO_CNT_MASK) >=
           UART_TXFIFO_LEN) {
    }
    UART_FIFO = (uint32_t)(uint8_t)c;
}

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
