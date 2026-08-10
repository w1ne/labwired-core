/* USART1 polled TX (debug console captured by the simulator). Driven through the
 * CMSIS register definitions.
 *
 * The simulator's byte-level UART model transmits on any TDR write and reports
 * TXE ready unconditionally, so a token CR1 (UE|TE) is all it takes to make the
 * console TEXT appear. It is not what it takes to make a WAVEFORM appear: a pad
 * left in its reset state is not connected to the USART at all, and BRR = 0 is
 * an invalid divisor rather than a slow one, so a logic probe on the TX pad
 * shows the idle GPIO latch while the console reads perfectly. Bring the pad
 * and the baud rate up the way silicon needs them and it is visible both ways. */
#include "stm32l476xx.h"
#include "debug_uart.h"
#include <stdint.h>

/* PA9 = USART1_TX at AF7 — STM32L476xx datasheet DS10198 Rev 11, Table 17
 * "Alternate function AF0 to AF7", p88, port A. Only TX is muxed: this console
 * never reads RDR, and routing a pad to a receiver nothing drives would claim
 * an idle-high line that is not ours to claim. */
#define DBG_TX_PIN 9u
#define DBG_TX_AF 7u

/* 115200 8N1, the rate the NUCLEO-L476RG's ST-LINK virtual COM port runs at.
 * This firmware never configures the PLL, so the core and APB2 stay on the MSI
 * reset clock — 4 MHz, the value master/system.yaml records as `cpu_hz`. Under
 * the default 16x oversampling BRR IS USARTDIV (RM0351 §38.5.4):
 *
 *   USARTDIV = f_ck / baud = 4 000 000 / 115 200 = 34.72 -> 35
 *   actual baud = 4 000 000 / 35 = 114 285.7 (-0.79%, inside 8N1 tolerance)
 */
#define DBG_BRR 35u

void dbg_uart_init(void) {
    const uint32_t shift = DBG_TX_PIN * 2u;
    GPIOA->MODER = (GPIOA->MODER & ~(3u << shift)) | (2u << shift); /* AF mode */
    GPIOA->OTYPER &= ~(1u << DBG_TX_PIN);                           /* push-pull */
    GPIOA->OSPEEDR |= 3u << shift;                                  /* very high */
    GPIOA->PUPDR &= ~(3u << shift);                                 /* no pull */
    GPIOA->AFR[1] = (GPIOA->AFR[1] & ~(0xFu << ((DBG_TX_PIN - 8u) * 4u))) |
                    (DBG_TX_AF << ((DBG_TX_PIN - 8u) * 4u));

    USART1->CR1 = 0u;
    USART1->CR2 = 0u;
    USART1->CR3 = 0u;
    USART1->BRR = DBG_BRR;
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
