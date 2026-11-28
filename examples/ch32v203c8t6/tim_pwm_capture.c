/* TIM3 CH1 PWM output and TIM2 PWM-input capture smoke for the CH32V203C8T6.
 *
 * ch32fun supplies the QingKe V4 startup and interrupt vector table.
 * SystemInit leaves HCLK at 144 MHz and APB1 at HCLK/2, so both timer
 * kernels run at 144 MHz (APB1 timers double a divided PCLK1).
 *
 * TIM3 CH1 (PA6, default map, CH32V203DS0 Table 3-1-1): PSC 143, ATRLR 999,
 * CH1CVR 250, PWM mode 1. That is a 1 kHz wave, high for 250 us.
 *
 * TIM2 (PA0 = TIM2_CH1, floating input): PSC 143 gives a 1 MHz count.
 * CH1 captures TI1 rising edges, CH2 captures the same TI1 input on falling
 * edges (CC2S = 10, CC2P). The TIM2 handler (PFIC interrupt 44) keeps the
 * first rise, the fall after it and the second rise; main prints the period
 * and the high time in units of 10 us.
 *
 * PA6 and PA0 are joined by a wire on the bench. The firmware does not
 * depend on that: any 3.3 V wave on PA0 is measured the same way.
 */
#include "ch32fun.h"
#include <stdint.h>

static volatile uint32_t rises;
static volatile uint16_t rise_first;
static volatile uint16_t rise_second;
static volatile uint16_t fall_first;
static volatile uint32_t have_fall;

__attribute__((interrupt)) void TIM2_IRQHandler(void) {
    uint16_t flags = TIM2->INTFR;
    /* Reading CHxCVR clears CCxIF. A rise is handled before a fall: the
     * wave is high for 250 us, so both flags are never new together unless
     * the handler was held off, and then the rise came first. */
    if ((flags & TIM_CC1IF) != 0) {
        uint16_t at = (uint16_t)TIM2->CH1CVR;
        if (rises == 0) {
            rise_first = at;
        } else if (rises == 1) {
            rise_second = at;
            TIM2->DMAINTENR = 0;
        }
        rises++;
    }
    if ((flags & TIM_CC2IF) != 0) {
        uint16_t at = (uint16_t)TIM2->CH2CVR;
        if (rises == 1 && have_fall == 0) {
            fall_first = at;
            have_fall = 1;
        }
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
    funPinMode(PA6, GPIO_CFGLR_OUT_10Mhz_AF_PP);
    funPinMode(PA0, GPIO_CFGLR_IN_FLOAT);
    RCC->APB2PCENR |= RCC_APB2Periph_USART1;
    RCC->APB1PCENR |= RCC_APB1Periph_TIM2 | RCC_APB1Periph_TIM3;

    /* 144 MHz / 115200. */
    USART1->BRR = 0x04E2u;
    USART1->CTLR1 = USART_Mode_Tx | CTLR1_UE_Set;

    /* TIM2 capture: CC1S = 01 (IC1 on TI1), CC2S = 10 (IC2 on TI1);
     * CC1E rising, CC2E with CC2P falling. */
    TIM2->PSC = 143;
    TIM2->ATRLR = 0xFFFF;
    TIM2->CHCTLR1 = 0x0201;
    TIM2->CCER = 0x0031;
    TIM2->SWEVGR = TIM_UG;
    TIM2->INTFR = 0;
    TIM2->DMAINTENR = TIM_CC1IE | TIM_CC2IE;
    NVIC_EnableIRQ(TIM2_IRQn);
    TIM2->CTLR1 = TIM_CEN;

    /* TIM3 CH1: OC1M = 110 (PWM mode 1) with OC1PE, output enabled. */
    TIM3->PSC = 143;
    TIM3->ATRLR = 999;
    TIM3->CH1CVR = 250;
    TIM3->CHCTLR1 = 0x0068;
    TIM3->CCER = 0x0001;
    TIM3->SWEVGR = TIM_UG;
    TIM3->CTLR1 = TIM_ARPE | TIM_CEN;

    /* 50 ms of SysTick is ample for two periods of a 1 kHz wave. */
    uint32_t start = (uint32_t)SysTick->CNT;
    while (rises < 2 && (uint32_t)SysTick->CNT - start < 50u * DELAY_MS_TIME) {
    }

    if (rises >= 2 && have_fall != 0) {
        uint32_t period = (uint16_t)(rise_second - rise_first);
        uint32_t high = (uint16_t)(fall_first - rise_first);
        uart_puts("PWM_IC period_10us=");
        uart_putu((period + 5u) / 10u);
        uart_puts(" high_10us=");
        uart_putu((high + 5u) / 10u);
        uart_puts("\r\n");
    } else {
        uart_puts("PWM_IC_TIMEOUT rises=");
        uart_putu(rises);
        uart_puts(" fall=");
        uart_putu(have_fall);
        uart_puts("\r\n");
    }

    while ((USART1->STATR & USART_FLAG_TC) == 0) {
    }
    for (;;) {
    }
}
