/* Polled debug console on ESP32-C3 UART0, for the LabWired simulator.
 *
 * UART0 lives at 0x60000000 and is the real Espressif UART block (TRM §UART):
 * FIFO@0x00, CLKDIV@0x14, STATUS@0x1C with TXFIFO_CNT[25:16], and a 128-entry
 * (SOC_UART_FIFO_LEN) TX FIFO that shifts out one 10-bit frame per baud period.
 *
 * Two consequences drive this driver, and both are properties of the silicon,
 * not of the simulator:
 *
 *  1. A write to a FULL TX FIFO is DROPPED. The FIFO is a memory-mapped queue,
 *     not a handshake — the bus write always completes. A polled driver must
 *     therefore read STATUS.TXFIFO_CNT and wait for a free entry before every
 *     byte, which is what uart_putc() does. Without that wait, anything longer
 *     than 128 bytes is silently truncated on the wire.
 *
 *  2. Wire time is real. At the power-on CLKDIV (694 -> ~115200 baud) one byte
 *     costs ~87 us; this firmware prints ~24 kB per run (per-cycle telemetry
 *     plus the final 128x64 framebuffer dump), which is ~2.1 s of wire time —
 *     far longer than the demo's simulated window. uart_init() therefore
 *     programs the console to 2 Mbaud, the way a real bring-up firmware that
 *     dumps a framebuffer over serial would.
 *
 * The simulator's test runner captures each byte as it shifts OUT of the TX
 * FIFO, so `uart_contains` assertions see exactly what a logic analyzer on the
 * TXD pad would see. */
#ifndef C3_UART_H
#define C3_UART_H

#include <stdint.h>

/* Program the console baud rate. Call once before the first uart_putc(). */
void uart_init(void);

void uart_putc(char c);
void uart_puts(const char *s);
/* Print a signed integer in decimal. */
void uart_puti(int32_t v);
/* Print a fixed-point value given as value*100 (e.g. 4910 -> "49.10"). */
void uart_putfix2(int32_t v_x100);
/* Print `n` as exactly `width` hex chars (upper-case, zero-padded). */
void uart_puthex(uint32_t n, int width);

#endif /* C3_UART_H */
