/*
 * STM32F446RE raw-register firmware for NUCLEO-F446RE.
 * ST RM0390 and stm32f446xx.h register map; no LabWired-specific calls.
 * HSI reset 16 MHz, UART2 on PA2/PA3 (ST-LINK VCP), LD2 on PA5.
 */
#include <stdint.h>
#define REG32(addr) (*(volatile uint32_t *)(addr))
#define RCC_AHB1ENR REG32(0x40023830UL)
#define RCC_APB1ENR REG32(0x40023840UL)
#define GPIOA_MODER REG32(0x40020000UL)
#define GPIOA_AFRL  REG32(0x40020020UL)
#define GPIOA_BSRR  REG32(0x40020018UL)
#define USART2_SR   REG32(0x40004400UL)
#define USART2_DR   REG32(0x40004404UL)
#define USART2_BRR  REG32(0x40004408UL)
#define USART2_CR1  REG32(0x4000440CUL)

void Reset_Handler(void);
static void uart_write(char c) {
    uint32_t timeout = 40000;
    while (!(USART2_SR & (1UL << 7)) && timeout--) {}
    USART2_DR = (uint32_t)(unsigned char)c;
}
static void uart_puts(const char *p) {
    while (*p) uart_write(*p++);
}
static void spin(uint32_t n) {
    while (n--) __asm__ volatile ("nop");
}
__attribute__((used, section(".isr_vector")))
const uintptr_t vectors[] = { 0x20020000UL, (uintptr_t)Reset_Handler };

void Reset_Handler(void) {
    RCC_AHB1ENR |= 1UL << 0;
    RCC_APB1ENR |= 1UL << 17;
    GPIOA_MODER = (GPIOA_MODER & ~((3UL << 4) | (3UL << 6) | (3UL << 10)))
                | (2UL << 4) | (2UL << 6) | (1UL << 10);
    GPIOA_AFRL = (GPIOA_AFRL & ~((15UL << 8) | (15UL << 12)))
               | (7UL << 8) | (7UL << 12);
    USART2_BRR = 0x8B;
    USART2_CR1 = (1UL << 13) | (1UL << 3);
    uart_puts("STM32F446RE_OK\n");
    for (;;) {
        GPIOA_BSRR = 1UL << 5;
        spin(400);
        GPIOA_BSRR = 1UL << 21;
        spin(400);
    }
}
