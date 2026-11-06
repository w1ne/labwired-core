/* CH32V003 PFIC interrupt smoke, built against WCH's EVT SDK (startup,
 * SystemInit, debug.c USART printf) by build-pfic-firmware.sh.
 *
 * Every marker prints only when the interrupt it names actually ran:
 *   CH32V003_SYSTICK_IRQ_OK  SysTick interrupt at 1 kHz, counted in the ISR
 *   CH32V003_IRQ_MASK_OK     NVIC_DisableIRQ stops it, re-enable resumes it
 *   CH32V003_SW_IPSR_OK      software interrupt pended through PFIC IPSR
 *   CH32V003_SW_SWIE_OK      software interrupt raised by STK_CTLR.SWIE
 * The SysTick ISR toggles PD0 every 10 ms, so the LED edges come only from
 * interrupt context.
 *
 * Handlers use the standard RISC-V `interrupt` attribute (ch32v003fun's
 * style), which saves registers in software and returns with mret.
 */
#include "debug.h"

static volatile uint32_t ms_ticks;
static volatile uint32_t sw_hits;

void SysTick_Handler(void) __attribute__((interrupt));
void SysTick_Handler(void)
{
    SysTick->SR = 0;
    ms_ticks++;
    if (ms_ticks % 10 == 0)
        GPIOD->OUTDR ^= GPIO_Pin_0;
}

void SW_Handler(void) __attribute__((interrupt));
void SW_Handler(void)
{
    /* SWIE keeps requesting until cleared; an IPSR pend is consumed on entry. */
    SysTick->CTLR &= ~(1u << 31);
    sw_hits++;
}

/* Spin until `*v` reaches `target`, at most `limit` iterations. */
static int wait_for(volatile uint32_t *v, uint32_t target, uint32_t limit)
{
    while (*v < target && limit--)
        ;
    return *v >= target;
}

int main(void)
{
    GPIO_InitTypeDef gpio = {0};

    SystemCoreClockUpdate();
    USART_Printf_Init(115200);
    printf("SystemClk:%d\r\n", SystemCoreClock);

    RCC_APB2PeriphClockCmd(RCC_APB2Periph_GPIOD, ENABLE);
    gpio.GPIO_Pin = GPIO_Pin_0;
    gpio.GPIO_Mode = GPIO_Mode_Out_PP;
    gpio.GPIO_Speed = GPIO_Speed_30MHz;
    GPIO_Init(GPIOD, &gpio);

    /* 1 kHz SysTick interrupt: HCLK source, auto-reload (CTLR = 0xF). */
    NVIC_EnableIRQ(SysTick_IRQn);
    SysTick->SR = 0;
    SysTick->CMP = SystemCoreClock / 1000 - 1;
    SysTick->CNT = 0;
    SysTick->CTLR = 0xF;

    if (wait_for(&ms_ticks, 5, 2000000))
        printf("CH32V003_SYSTICK_IRQ_OK ticks=%lu\r\n", (unsigned long)ms_ticks);

    /* Masked in the PFIC: CNTIF keeps latching, the ISR must not run. */
    NVIC_DisableIRQ(SysTick_IRQn);
    uint32_t frozen = ms_ticks;
    for (volatile uint32_t i = 0; i < 20000; i++)
        ;
    int masked = ms_ticks == frozen && (SysTick->SR & 1);
    NVIC_EnableIRQ(SysTick_IRQn);
    if (masked && wait_for(&ms_ticks, frozen + 2, 2000000))
        printf("CH32V003_IRQ_MASK_OK\r\n");

    NVIC_EnableIRQ(Software_IRQn);
    NVIC_SetPendingIRQ(Software_IRQn);
    if (wait_for(&sw_hits, 1, 1000))
        printf("CH32V003_SW_IPSR_OK\r\n");

    SysTick->CTLR |= 1u << 31;
    if (wait_for(&sw_hits, 2, 1000) && !(SysTick->CTLR & (1u << 31)))
        printf("CH32V003_SW_SWIE_OK hits=%lu\r\n", (unsigned long)sw_hits);

    while (1)
        ;
}
