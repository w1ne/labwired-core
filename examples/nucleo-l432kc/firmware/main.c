/*
 * STM32L432KC raw-register firmware for NUCLEO-L432KC.
 * ST RM0394 and stm32l432xx.h register map; no LabWired-specific calls.
 * MSI reset 4 MHz, USART2 TX on PA2 (AF7, ST-LINK VCP), LD3 on PB3.
 */
#include <stdint.h>
#define REG32(addr) (*(volatile uint32_t *)(addr))
#define RCC_AHB2ENR  REG32(0x4002104CUL) /* GPIOAEN bit 0, GPIOBEN bit 1 */
#define RCC_APB1ENR1 REG32(0x40021058UL) /* USART2EN bit 17 */
#define GPIOA_MODER  REG32(0x48000000UL)
#define GPIOA_AFRL   REG32(0x48000020UL)
#define GPIOB_MODER  REG32(0x48000400UL)
#define GPIOB_BSRR   REG32(0x48000418UL)
#define USART2_CR1   REG32(0x40004400UL)
#define USART2_BRR   REG32(0x4000440CUL)
#define USART2_ISR   REG32(0x4000441CUL)
#define USART2_TDR   REG32(0x40004428UL)

void Reset_Handler(void);
static void uart_write(char c) {
    uint32_t timeout = 40000;
    while (!(USART2_ISR & (1UL << 7)) && timeout--) {}
    USART2_TDR = (uint32_t)(unsigned char)c;
}
static void uart_puts(const char *p) {
    while (*p) uart_write(*p++);
}
static void spin(uint32_t n) {
    while (n--) __asm__ volatile ("nop");
}
__attribute__((used, section(".isr_vector")))
const uintptr_t vectors[] = { 0x20010000UL, (uintptr_t)Reset_Handler };

void Reset_Handler(void) {
    RCC_AHB2ENR |= (1UL << 0) | (1UL << 1);
    RCC_APB1ENR1 |= 1UL << 17;
    /* PA2 alternate function 7 = USART2_TX. */
    GPIOA_MODER = (GPIOA_MODER & ~(3UL << 4)) | (2UL << 4);
    GPIOA_AFRL = (GPIOA_AFRL & ~(15UL << 8)) | (7UL << 8);
    /* PB3 general-purpose output = LD3. */
    GPIOB_MODER = (GPIOB_MODER & ~(3UL << 6)) | (1UL << 6);
    USART2_BRR = 35; /* 4 MHz / 115200 */
    USART2_CR1 = (1UL << 3) | (1UL << 0); /* TE | UE */
    uart_puts("STM32L432KC_OK\n");
    for (;;) {
        GPIOB_BSRR = 1UL << 3;
        spin(400);
        GPIOB_BSRR = 1UL << 19;
        spin(400);
    }
}
