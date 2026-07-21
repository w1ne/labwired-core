/* Polled debug UART for the ESP32-C3 in the LabWired simulator.
 *
 * The simulator maps a generic UART model at 0x60000000 (UART0). Its Stm32F1
 * register layout treats a byte write at offset 0 as a TX-data write, so a
 * single `*(volatile uint8_t*)0x60000000 = c` transmits one character — the
 * same path the firmware-esp32c3-demo crate uses. The simulator's test runner
 * captures every transmitted byte into its UART log for `uart_contains`
 * assertions. */
#ifndef C3_UART_H
#define C3_UART_H

#include <stdint.h>

void uart_putc(char c);
void uart_puts(const char *s);
/* Print a signed integer in decimal. */
void uart_puti(int32_t v);
/* Print a fixed-point value given as value*100 (e.g. 4910 -> "49.10"). */
void uart_putfix2(int32_t v_x100);
/* Print `n` as exactly `width` hex chars (upper-case, zero-padded). */
void uart_puthex(uint32_t n, int width);

#endif /* C3_UART_H */
