#include <stdint.h>

/* Register addresses and bits: WCH CH32X035 RM V1.9 (RCC, GPIO, USART). */
#define RCC_APB2PCENR (*(volatile uint32_t *)0x40021018u)
#define GPIOB_CFGHR   (*(volatile uint32_t *)0x40010C04u)
#define USART1_STATR  (*(volatile uint32_t *)0x40013800u)
#define USART1_DATAR  (*(volatile uint32_t *)0x40013804u)
#define USART1_BRR    (*(volatile uint32_t *)0x40013808u)
#define USART1_CTLR1  (*(volatile uint32_t *)0x4001380Cu)

#define RCC_IOPBEN      (1u << 3)
#define RCC_USART1EN    (1u << 14)
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

/* Exercise the M and A extensions of the QingKe V4C. */
static volatile uint32_t counter;
static volatile uint32_t seed;
static volatile uint32_t divisor;

static uint32_t checksum(void) {
    uint32_t d = divisor;
    uint32_t product = seed * 2654435761u;
    uint32_t quotient = product / d;
    uint32_t old;
    __asm__ volatile("amoadd.w %0, %2, (%1)"
                     : "=r"(old)
                     : "r"(&counter), "r"(quotient & 0xFFu)
                     : "memory");
    return old + (product % d);
}

void main(void) {
    /* Reset clock: HSI 48 MHz, HPRE = DIV6 (CFGR0 = 0x50) -> HCLK = 8 MHz. */
    /* USART1 default mapping: TX1 on PB10, AF push-pull (CFGHR bits 11:8, GPIO_Init nibble 0x9). */
    RCC_APB2PCENR |= RCC_IOPBEN | RCC_USART1EN;
    GPIOB_CFGHR = (GPIOB_CFGHR & ~(0xFu << 8)) | (0x9u << 8);

    /* 8 MHz / 115200: mantissa 4, fraction 5 (USARTDIV = 4.34). */
    USART1_BRR = 0x0045u;
    USART1_CTLR1 = USART_CTLR1_UE | USART_CTLR1_TE;

    seed = 3u;
    divisor = 7u;
    counter = 40u;
    (void)checksum();
    if (counter == 40u + ((3u * 2654435761u / 7u) & 0xFFu)) {
        uart_puts("Hello from CH32X035\r\n");
    } else {
        uart_puts("RV32IMAC check failed\r\n");
    }
    while ((USART1_STATR & USART_STATR_TC) == 0) {
    }

    for (;;) {
    }
}
