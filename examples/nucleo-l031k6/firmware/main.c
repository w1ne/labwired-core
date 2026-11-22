/*
 * STM32L031K6 raw-register firmware for NUCLEO-L031K6.
 * ST RM0377 and stm32l031xx.h register map; no LabWired-specific calls.
 * MSI reset ~2.097 MHz, USART2 TX on PA2 (AF4, ST-LINK VCP), LD3 on PB3.
 */
#include <stdint.h>
#define REG32(addr) (*(volatile uint32_t *)(addr))
#define RCC_IOPENR   REG32(0x4002102CUL) /* IOPAEN bit 0, IOPBEN bit 1 */
#define RCC_APB1ENR  REG32(0x40021038UL) /* USART2EN bit 17 */
#define GPIOA_MODER  REG32(0x50000000UL)
#define GPIOA_AFRL   REG32(0x50000020UL)
#define GPIOB_MODER  REG32(0x50000400UL)
#define GPIOB_BSRR   REG32(0x50000418UL)
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
/* Initial SP is the top of the 8 KB SRAM (CMSIS SRAM_SIZE_MAX). */
__attribute__((used, section(".isr_vector")))
const uintptr_t vectors[] = { 0x20002000UL, (uintptr_t)Reset_Handler };

void Reset_Handler(void) {
    RCC_IOPENR |= (1UL << 0) | (1UL << 1);
    RCC_APB1ENR |= 1UL << 17;
    /* PA2 alternate function 4 = USART2_TX (RM0377 / stm32l031xx.h GPIO_AF4_USART2). */
    GPIOA_MODER = (GPIOA_MODER & ~(3UL << 4)) | (2UL << 4);
    GPIOA_AFRL = (GPIOA_AFRL & ~(15UL << 8)) | (4UL << 8);
    /* PB3 general-purpose output = LD3 (UM1956 Table 8 / Table 14, SB15 default ON). */
    GPIOB_MODER = (GPIOB_MODER & ~(3UL << 6)) | (1UL << 6);
    USART2_BRR = 18; /* ~2.097 MHz MSI / 115200 */
    USART2_CR1 = (1UL << 3) | (1UL << 0); /* TE | UE */
    uart_puts("STM32L031K6_OK\n");
    for (;;) {
        GPIOB_BSRR = 1UL << 3;
        spin(400);
        GPIOB_BSRR = 1UL << 19;
        spin(400);
    }
}
