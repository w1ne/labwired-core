/* Polled debug UART on USART1 (text to the simulator's stdout capture). */
#ifndef DEBUG_UART_H
#define DEBUG_UART_H

void dbg_uart_init(void);
void dbg_puts(const char *s);
void dbg_hex8(unsigned char b); /* print one byte as two hex chars */

#endif /* DEBUG_UART_H */
