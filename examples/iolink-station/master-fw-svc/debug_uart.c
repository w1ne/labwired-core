/* USART1 polled TX (debug console captured by the simulator). Driven through the
 * CMSIS register definitions. The simulator's UART model transmits on any TDR
 * write and reports TXE ready unconditionally, so only a token CR1 (UE|TE) is
 * needed. */
#include "stm32l476xx.h"
#include "debug_uart.h"
#include <stdint.h>

void dbg_uart_init(void) {
    USART1->CR1 = USART_CR1_UE | USART_CR1_TE;
}

static void dbg_putc(char c) {
    while ((USART1->ISR & USART_ISR_TXE) == 0u) {
    }
    USART1->TDR = (uint32_t)(unsigned char)c;
}

void dbg_puts(const char *s) {
    while (*s) {
        dbg_putc(*s++);
    }
}

void dbg_hex8(unsigned char b) {
    static const char hex[] = "0123456789ABCDEF";
    dbg_putc(hex[(b >> 4) & 0xFu]);
    dbg_putc(hex[b & 0xFu]);
}
