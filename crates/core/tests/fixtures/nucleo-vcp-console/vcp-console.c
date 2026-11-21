/* NUCLEO-64 console tap fixture, register-level. The ST-LINK virtual COM
 * port is wired to USART2 (PA2/PA3) on NUCLEO-F103RB / -F401RE / -F411RE /
 * -F446RE (UM1724 §6.8). The firmware also drives USART1 (an Arduino
 * `Serial1` link), one byte on each port in turn, so a console that taps
 * every UART splices the two streams together.
 *
 *   USART2 (console): "VCP_BOOT\r\n" then "VCP_DONE\r\n"
 *   USART1 (link):    "AUX_LINK_DATA\r\n" (sent between the two lines)
 *
 * Build: -DFAMILY_F1 for STM32F1 (RCC at 0x40021000), else STM32F4.
 * Both families keep USART SR/DR/BRR/CR1 at 0x00/0x04/0x08/0x0C and put
 * USART1 at APB2ENR bit 14 (F1) / bit 4 (F4), USART2 at APB1ENR bit 17.
 */
#include <stdint.h>

#define REG(a) (*(volatile uint32_t *)(a))
#ifdef FAMILY_F1
#define RCC_APB1ENR REG(0x4002101Cu)
#define RCC_APB2ENR REG(0x40021018u)
#define USART1_BASE 0x40013800u
#define USART1_EN   (1u << 14)
#define GPIOA_CRL   REG(0x40010800u)
#define GPIOA_CRH   REG(0x40010804u)
#define IOPA_EN     (1u << 2)
#else
#define RCC_APB1ENR REG(0x40023840u)
#define RCC_APB2ENR REG(0x40023844u)
#define USART1_BASE 0x40011000u
#define USART1_EN   (1u << 4)
#endif
#define USART2_BASE 0x40004400u
#define USART2_EN   (1u << 17)
#define U_SR(b)  REG((b) + 0x00u)
#define U_DR(b)  REG((b) + 0x04u)
#define U_BRR(b) REG((b) + 0x08u)
#define U_CR1(b) REG((b) + 0x0Cu)
#define SR_TXE  (1u << 7)
#define SR_TC   (1u << 6)
#define CR1_TE  (1u << 3)
#define CR1_UE  (1u << 13)

static void uart_init(uint32_t b) {
    U_BRR(b) = 0x45u; /* 115200 at 8 MHz; the value does not matter here */
    U_CR1(b) = CR1_UE | CR1_TE;
}

static void put(uint32_t b, char c) {
    while (!(U_SR(b) & SR_TXE)) {
    }
    U_DR(b) = (uint8_t)c;
}

static void puts_on(uint32_t b, const char *s) {
    while (*s) {
        put(b, *s++);
    }
}

static void flush(uint32_t b) {
    while (!(U_SR(b) & SR_TC)) {
    }
}

void reset(void) {
#ifdef FAMILY_F1
    /* F1 has no AF mux: TX pins PA2 (USART2) and PA9 (USART1) must be
     * alternate-function push-pull (CNF=10, MODE=11 -> 0xB). */
    RCC_APB2ENR |= IOPA_EN;
    GPIOA_CRL = (GPIOA_CRL & ~(0xFu << 8)) | (0xBu << 8);
    GPIOA_CRH = (GPIOA_CRH & ~(0xFu << 4)) | (0xBu << 4);
#endif
    RCC_APB2ENR |= USART1_EN;
    RCC_APB1ENR |= USART2_EN;
    uart_init(USART1_BASE);
    uart_init(USART2_BASE);
    puts_on(USART2_BASE, "VCP_BOOT\r\n");
    flush(USART2_BASE);
    /* One byte on each port in turn: Serial1 traffic while the console runs. */
    const char *aux = "AUX_LINK_DATA\r\n";
    const char *vcp = "VCP_DONE\r\n";
    while (*aux || *vcp) {
        if (*aux) {
            put(USART1_BASE, *aux++);
        }
        if (*vcp) {
            put(USART2_BASE, *vcp++);
        }
    }
    flush(USART1_BASE);
    flush(USART2_BASE);
    for (;;) {
    }
}

__attribute__((section(".vectors"), used))
static const uint32_t vectors[2] = {0x20002000u, (uint32_t)reset};
