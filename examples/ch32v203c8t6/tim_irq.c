/* TIM2 update and TIM1 capture/compare interrupt smoke for the CH32V203C8T6.
 *
 * ch32fun supplies the QingKe V4 startup and interrupt vector table.
 * SystemInit leaves HCLK at 144 MHz and APB1 at HCLK/2, so both timer
 * kernels run at 144 MHz (APB1 timers double a divided PCLK1).
 *
 * TIM2: PSC 143, ATRLR 999 gives a 1 kHz update on TIM2_IRQn 44.
 * TIM1: same period, CH1CVR 250, CC1IE only, so the compare event reaches
 * TIM1_CC_IRQn 43 and not the TIM1_UP_IRQn 41 line.
 *
 * The TIM2 handler stamps the 64-bit SysTick (HCLK/8 = 18 MHz) at update 1
 * and update 11; main prints the elapsed milliseconds between them.
 */
#include "ch32fun.h"
#include <stdint.h>

#define TIM2_UPDATES 11u

static volatile uint32_t tim2_updates;
static volatile uint32_t tim1_compares;
static volatile uint32_t stamp_first;
static volatile uint32_t stamp_last;

__attribute__((interrupt)) void TIM2_IRQHandler(void) {
    if ((TIM2->INTFR & TIM_UIF) != 0) {
        TIM2->INTFR = (uint16_t)~TIM_UIF;
        uint32_t count = tim2_updates + 1;
        uint32_t now = (uint32_t)SysTick->CNT;
        if (count == 1) {
            stamp_first = now;
        } else if (count == TIM2_UPDATES) {
            stamp_last = now;
            TIM2->CTLR1 &= (uint16_t)~TIM_CEN;
            TIM1->CTLR1 &= (uint16_t)~TIM_CEN;
        }
        tim2_updates = count;
    }
}

__attribute__((interrupt)) void TIM1_CC_IRQHandler(void) {
    if ((TIM1->INTFR & TIM_CC1IF) != 0) {
        TIM1->INTFR = (uint16_t)~TIM_CC1IF;
        tim1_compares++;
    }
}

static void uart_putc(char byte) {
    while ((USART1->STATR & USART_FLAG_TXE) == 0) {
    }
    USART1->DATAR = (uint8_t)byte;
}

static void uart_puts(const char *text) {
    while (*text != '\0') {
        uart_putc(*text++);
    }
}

static void uart_putu(uint32_t value) {
    char digits[10];
    uint32_t n = 0;
    do {
        digits[n++] = (char)('0' + value % 10u);
        value /= 10u;
    } while (value != 0 && n < sizeof(digits));
    while (n != 0) {
        uart_putc(digits[--n]);
    }
}

int main(void) {
    SystemInit();
    funGpioInitAll();

    funPinMode(PA9, GPIO_CFGLR_OUT_10Mhz_AF_PP);
    RCC->APB2PCENR |= RCC_APB2Periph_USART1 | RCC_APB2Periph_TIM1;
    RCC->APB1PCENR |= RCC_APB1Periph_TIM2;

    /* 144 MHz / 115200. */
    USART1->BRR = 0x04E2u;
    USART1->CTLR1 = USART_Mode_Tx | CTLR1_UE_Set;

    TIM1->PSC = 143;
    TIM1->ATRLR = 999;
    TIM1->CH1CVR = 250;
    TIM1->DMAINTENR = TIM_CC1IE;

    TIM2->PSC = 143;
    TIM2->ATRLR = 999;
    TIM2->DMAINTENR = TIM_UIE;

    NVIC_EnableIRQ(TIM1_CC_IRQn);
    NVIC_EnableIRQ(TIM2_IRQn);

    TIM1->CTLR1 = TIM_CEN;
    TIM2->CTLR1 = TIM_CEN;

    /* 20 ms of SysTick is ample for 11 updates at 1 kHz. */
    uint32_t start = (uint32_t)SysTick->CNT;
    while (tim2_updates < TIM2_UPDATES &&
           (uint32_t)SysTick->CNT - start < 20u * DELAY_MS_TIME) {
    }

    if (tim2_updates == TIM2_UPDATES) {
        uint32_t elapsed = stamp_last - stamp_first;
        uart_puts("TIM2_UP=");
        uart_putu(tim2_updates);
        uart_puts(" ms=");
        uart_putu((elapsed + DELAY_MS_TIME / 2u) / DELAY_MS_TIME);
        uart_puts(" TIM1_CC1=");
        uart_putu(tim1_compares);
        uart_puts("\r\n");
    } else {
        uart_puts("TIM_TIMEOUT up=");
        uart_putu(tim2_updates);
        uart_puts(" cc1=");
        uart_putu(tim1_compares);
        uart_puts("\r\n");
    }

    while ((USART1->STATR & USART_FLAG_TC) == 0) {
    }
    for (;;) {
    }
}
