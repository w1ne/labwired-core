#include <Arduino.h>
#include <stdint.h>

#define REG32(a) (*(volatile uint32_t *)(a))
#define RCC_IOPENR REG32(0x4002102C)
#define RCC_APB1ENR REG32(0x40021038)
#define GPIOA_MODER REG32(0x50000000)
#define GPIOA_BSRR REG32(0x50000018)
#define GPIOA_AFRL REG32(0x50000020)
#define USART2_CR1 REG32(0x40004400)
#define USART2_BRR REG32(0x4000440C)
#define USART2_ISR REG32(0x4000441C)
#define USART2_TDR REG32(0x40004428)

static void putc_uart(char c) {
  while ((USART2_ISR & (1u << 7)) == 0) {}
  USART2_TDR = (uint32_t)c;
}

static void puts_uart(const char *s) {
  while (*s) putc_uart(*s++);
}

void setup() {
  RCC_IOPENR |= 1u;
  RCC_APB1ENR |= 1u << 17;
  GPIOA_MODER = (GPIOA_MODER & ~((3u << 4) | (3u << 6) | (3u << 10))) |
                (2u << 4) | (2u << 6) | (1u << 10);
  GPIOA_AFRL = (GPIOA_AFRL & ~((15u << 8) | (15u << 12))) |
               (4u << 8) | (4u << 12);
  GPIOA_BSRR = 1u << 5;
  USART2_BRR = 0x683u;
  USART2_CR1 = (1u << 3) | 1u;
  puts_uart("L073_ONBOARDING_OK\n");
}

void loop() {}
