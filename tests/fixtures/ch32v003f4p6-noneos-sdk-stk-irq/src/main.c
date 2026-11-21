#include "debug.h"

/* Stock noneos-sdk style: the handler relies on the QingKe hardware
 * prologue/epilogue (HPE) to save caller-saved registers. */
void SysTick_Handler(void) __attribute__((interrupt("WCH-Interrupt-fast")));

static volatile uint32_t ticks;

void SysTick_Handler(void)
{
    SysTick->SR = 0;
    ticks++;
}

int main(void)
{
    SystemCoreClockUpdate();
    Delay_Init();
    USART_Printf_Init(115200);
    printf("STK_IRQ_BOOT\r\n");
    /* 1 kHz tick: HCLK/8 = 6 MHz, 6000 counts per ms. */
    SysTick->SR = 0;
    SysTick->CNT = 0;
    SysTick->CMP = SystemCoreClock / 8 / 1000 - 1;
    SysTick->CTLR = (1 << 0) | (1 << 1) | (1 << 3); /* STE | STIE | STRE */
    NVIC_EnableIRQ(SysTicK_IRQn);
    /* Busy work in caller-saved registers while the tick fires. */
    uint32_t a = 1, b = 0;
    while (ticks < 5) {
        b += a;
        a = a * 3 + 1;
    }
    printf("STK_IRQ_TICKS %lu\r\n", (unsigned long)ticks);
    NVIC_DisableIRQ(SysTicK_IRQn);
    uint32_t frozen = ticks;
    Delay_Ms(3);
    printf("STK_IRQ_MASKED %s\r\n", ticks == frozen ? "ok" : "leak");
    printf("STK_IRQ_DONE %lu\r\n", (unsigned long)(b != 0));
    while (1) {}
}
