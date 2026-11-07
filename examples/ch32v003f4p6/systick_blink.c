#include <stdint.h>

/* Register addresses and bits: WCH ch32v00x.h and core_riscv.h (openwch/ch32v003). */
#define RCC_CFGR0     (*(volatile uint32_t *)0x40021004u)
#define RCC_APB2PCENR (*(volatile uint32_t *)0x40021018u)
#define GPIOC_CFGLR   (*(volatile uint32_t *)0x40011000u)
#define GPIOC_BSHR    (*(volatile uint32_t *)0x40011010u)
#define GPIOC_BCR     (*(volatile uint32_t *)0x40011014u)

typedef struct {
    volatile uint32_t CTLR;
    volatile uint32_t SR;
    volatile uint32_t CNT;
    uint32_t RESERVED0;
    volatile uint32_t CMP;
    uint32_t RESERVED1;
} SysTick_Type;
#define SysTick ((SysTick_Type *)0xE000F000u)

#define SYSTEM_CORE_CLOCK 24000000u /* HSI, HPRE = DIV1 */

static uint16_t p_ms;

/* WCH Debug/debug.c Delay_Init: STK runs at HCLK/8. */
static void Delay_Init(void) {
    uint8_t p_us = SYSTEM_CORE_CLOCK / 8000000u;
    p_ms = (uint16_t)p_us * 1000u;
}

/* WCH Debug/debug.c Delay_Ms, unchanged. */
static void Delay_Ms(uint32_t n) {
    uint32_t i;

    SysTick->SR &= ~(1u << 0);
    i = (uint32_t)n * p_ms;

    SysTick->CMP = i;
    SysTick->CNT = 0;
    SysTick->CTLR |= (1u << 0);

    while ((SysTick->SR & (1u << 0)) != (1u << 0)) {
    }
    SysTick->CTLR &= ~(1u << 0);
}

void main(void) {
    /* SystemInit's RCC reset step: SW = HSI, HPRE = DIV1 -> HCLK = 24 MHz. */
    RCC_CFGR0 &= 0xF8FF0000u;
    Delay_Init();

    RCC_APB2PCENR |= 1u << 4; /* enable GPIOC */
    GPIOC_CFGLR = 0x2u;       /* PC0: push-pull output, 2 MHz */

    for (;;) {
        GPIOC_BSHR = 1u; /* set PC0 */
        Delay_Ms(1);
        GPIOC_BCR = 1u;  /* reset PC0 */
        Delay_Ms(1);
    }
}
