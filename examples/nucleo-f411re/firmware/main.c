/*
 * STM32F411RE raw-register firmware for NUCLEO-F411RE.
 * ST RM0383 and stm32f411xe.h register map; no LabWired-specific calls.
 * HSI reset 16 MHz, USART2 TX on PA2 (AF7, ST-LINK VCP), LD2 on PA5.
 */
#include <stdint.h>
#define REG32(addr) (*(volatile uint32_t *)(addr))
#define RCC_AHB1ENR  REG32(0x40023830UL) /* GPIOAEN bit 0 */
#define RCC_APB1ENR  REG32(0x40023840UL) /* USART2EN bit 17 */
#define GPIOA_MODER  REG32(0x40020000UL)
#define GPIOA_BSRR   REG32(0x40020018UL)
#define GPIOA_AFRL   REG32(0x40020020UL)
#define USART2_SR    REG32(0x40004400UL)
#define USART2_DR    REG32(0x40004404UL)
#define USART2_BRR   REG32(0x40004408UL)
#define USART2_CR1   REG32(0x4000440CUL)

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
    /* PA2 alternate function 7 = USART2_TX. */
    GPIOA_MODER = (GPIOA_MODER & ~(3UL << 4)) | (2UL << 4);
    GPIOA_AFRL = (GPIOA_AFRL & ~(15UL << 8)) | (7UL << 8);
    /* PA5 general-purpose output = LD2. */
    GPIOA_MODER = (GPIOA_MODER & ~(3UL << 10)) | (1UL << 10);
    USART2_BRR = 139; /* 16 MHz / 115200 */
    USART2_CR1 = (1UL << 13) | (1UL << 3); /* UE | TE */
    uart_puts("STM32F411RE_OK\n");
    for (;;) {
        GPIOA_BSRR = 1UL << 5;
        spin(400);
        GPIOA_BSRR = 1UL << 21;
        spin(400);
    }
}
