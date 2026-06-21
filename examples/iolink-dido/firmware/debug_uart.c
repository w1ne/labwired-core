/* USART1 polled TX. The simulator's UART model transmits on any TDR write and
 * reports TXE ready unconditionally, so only a token CR1 (UE|TE) is needed. */
#include <stdint.h>
#include "debug_uart.h"

#define USART1_BASE 0x40013800u
#define REG(a) (*(volatile uint32_t *)(a))
#define U1_CR1 REG(USART1_BASE + 0x00u)
#define U1_ISR REG(USART1_BASE + 0x1Cu)
#define U1_TDR REG(USART1_BASE + 0x28u)
#define ISR_TXE (1u << 7)
#define CR1_UE (1u << 0)
#define CR1_TE (1u << 3)

void dbg_uart_init(void) {
    U1_CR1 = CR1_UE | CR1_TE;
}

static void dbg_putc(char c) {
    while ((U1_ISR & ISR_TXE) == 0u) {
    }
    U1_TDR = (uint32_t)(unsigned char)c;
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
