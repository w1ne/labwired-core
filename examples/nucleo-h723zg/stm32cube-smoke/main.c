#include <stdint.h>
#define R(x) (*(volatile uint32_t *)(x))
#define RCC_AHB4ENR R(0x580244e0u)
#define RCC_APB1LENR R(0x580244e8u)
#define GPIOB_MODER R(0x58020400u)
#define GPIOB_BSRR R(0x58020418u)
#define GPIOD_MODER R(0x58020c00u)
#define GPIOD_AFRH R(0x58020c24u)
#define USART3_CR1 R(0x40004800u)
#define USART3_BRR R(0x4000480cu)
#define USART3_ISR R(0x4000481cu)
#define USART3_TDR R(0x40004828u)
int main(void) {
  RCC_AHB4ENR |= (1u<<1)|(1u<<3); RCC_APB1LENR |= 1u<<18;
  GPIOB_MODER = (GPIOB_MODER & ~3u) | 1u;
  GPIOD_MODER = (GPIOD_MODER & ~((3u<<16)|(3u<<18))) | (2u<<16) | (2u<<18);
  GPIOD_AFRH = (GPIOD_AFRH & ~0xffu) | 0x77u;
  USART3_BRR = 556; USART3_CR1 = (1u<<0)|(1u<<3);
  const char *s="H723_OK\n";
  while (*s) { unsigned timeout=20000; while (!(USART3_ISR & (1u<<7)) && timeout--) {} USART3_TDR = *s++; }
  for (;;) { GPIOB_BSRR=1u; for (volatile unsigned i=0;i<300;i++) {} GPIOB_BSRR=1u<<16; for (volatile unsigned i=0;i<300;i++) {} }
}
