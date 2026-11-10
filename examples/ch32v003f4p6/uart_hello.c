#include <stdint.h>

/* Register addresses and bits: WCH ch32v00x.h (openwch/ch32v003). */
#define RCC_CFGR0     (*(volatile uint32_t *)0x40021004u)
#define RCC_APB2PCENR (*(volatile uint32_t *)0x40021018u)
#define GPIOD_CFGLR   (*(volatile uint32_t *)0x40011400u)
#define USART1_STATR  (*(volatile uint32_t *)0x40013800u)
#define USART1_DATAR  (*(volatile uint32_t *)0x40013804u)
#define USART1_BRR    (*(volatile uint32_t *)0x40013808u)
#define USART1_CTLR1  (*(volatile uint32_t *)0x4001380Cu)

#define RCC_IOPDEN    (1u << 5)
#define RCC_USART1EN  (1u << 14)
#define USART_STATR_TC  (1u << 6)
#define USART_STATR_TXE (1u << 7)
#define USART_CTLR1_TE  (1u << 3)
#define USART_CTLR1_UE  (1u << 13)

static void uart_putc(char c) {
    while ((USART1_STATR & USART_STATR_TXE) == 0) {
    }
    USART1_DATAR = (uint8_t)c;
}

static void uart_puts(const char *s) {
    while (*s) {
        uart_putc(*s++);
    }
}

void main(void) {
    /* SystemInit's RCC reset step: SW = HSI, HPRE = DIV1 -> HCLK = 24 MHz. */
    RCC_CFGR0 &= 0xF8FF0000u;

    /* USART1 default mapping (WCH Debug/debug.c): TX on PD5, AF push-pull. */
    RCC_APB2PCENR |= RCC_IOPDEN | RCC_USART1EN;
    GPIOD_CFGLR = (GPIOD_CFGLR & ~(0xFu << 20)) | (0xBu << 20);

    /* USART_Init's divider at 24 MHz / 115200: mantissa 13, fraction 0. */
    USART1_BRR = 0x00D0u;
    USART1_CTLR1 = USART_CTLR1_UE | USART_CTLR1_TE;

    uart_puts("Hello from CH32V003\r\n");
    while ((USART1_STATR & USART_STATR_TC) == 0) {
    }

    for (;;) {
    }
}
