/* ADC1 end-of-conversion interrupt and ADC2 polled smoke for the CH32V203C8T6.
 *
 * ch32fun supplies the QingKe V4 startup and interrupt vector table.
 * The system file holds PA0 (ADC1 IN0) at 1650 mV and PA1 (ADC2 IN1) at
 * 825 mV against the 3.3 V VDDA.
 *
 * ADC1: power on, reset and run calibration, then a software-started
 * single conversion of IN0 with EOCIE set. The ADC_IRQn 34 handler reads
 * RDATAR (which clears EOC). A second conversion reads IN17 (VREFINT,
 * CTLR2.TSVREFE set) the same way.
 * ADC2: a software-started single conversion of IN1, polled on STATR.EOC.
 *
 * Expected codes at 12 bits: 1650 mV -> 2047, 825 mV -> 1023,
 * VREFINT 1.2 V -> 1489.
 */
#include "ch32fun.h"
#include <stdint.h>

static volatile uint32_t adc_irqs;
static volatile uint32_t adc1_last;

__attribute__((interrupt)) void ADC1_2_IRQHandler(void) {
    if ((ADC1->STATR & ADC_EOC) != 0) {
        adc1_last = ADC1->RDATAR;
        adc_irqs++;
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

static void adc_calibrate(ADC_TypeDef *adc) {
    adc->CTLR2 |= ADC_ADON;
    adc->CTLR2 |= ADC_RSTCAL;
    while ((adc->CTLR2 & ADC_RSTCAL) != 0) {
    }
    adc->CTLR2 |= ADC_CAL;
    while ((adc->CTLR2 & ADC_CAL) != 0) {
    }
}

/* Start one ADC1 conversion of `channel` and wait for the interrupt. */
static int adc1_convert_irq(uint32_t channel, uint32_t *code) {
    uint32_t before = adc_irqs;
    ADC1->RSQR3 = channel;
    ADC1->CTLR2 |= ADC_SWSTART;
    uint32_t start = (uint32_t)SysTick->CNT;
    while (adc_irqs == before) {
        if ((uint32_t)SysTick->CNT - start > 5u * DELAY_MS_TIME) {
            return 0;
        }
    }
    *code = adc1_last;
    return 1;
}

int main(void) {
    SystemInit();
    funGpioInitAll();

    funPinMode(PA9, GPIO_CFGLR_OUT_10Mhz_AF_PP);
    funPinMode(PA0, GPIO_CFGLR_IN_ANALOG);
    funPinMode(PA1, GPIO_CFGLR_IN_ANALOG);

    /* ADCCLK = PCLK2 / 8, the largest divider. The model takes a fixed
     * conversion time, so the divider is not checked against the 14 MHz limit. */
    RCC->CFGR0 = (RCC->CFGR0 & ~RCC_ADCPRE) | RCC_ADCPRE_DIV8;
    RCC->APB2PCENR |= RCC_APB2Periph_USART1 | RCC_APB2Periph_ADC1 | RCC_APB2Periph_ADC2;

    /* 144 MHz / 115200. */
    USART1->BRR = 0x04E2u;
    USART1->CTLR1 = USART_Mode_Tx | CTLR1_UE_Set;

    adc_calibrate(ADC1);
    adc_calibrate(ADC2);

    /* Software trigger (EXTSEL = 111, EXTTRIG) and VREFINT enable. */
    ADC1->CTLR2 |= ADC_EXTSEL | ADC_EXTTRIG | ADC_TSVREFE;
    ADC2->CTLR2 |= ADC_EXTSEL | ADC_EXTTRIG;
    ADC1->CTLR1 = ADC_EOCIE;
    NVIC_EnableIRQ(ADC_IRQn);

    uint32_t in0 = 0;
    uint32_t vref = 0;
    int ok = adc1_convert_irq(0, &in0);
    ok = ok && adc1_convert_irq(17, &vref);

    ADC2->RSQR3 = 1;
    ADC2->CTLR2 |= ADC_SWSTART;
    uint32_t start = (uint32_t)SysTick->CNT;
    while ((ADC2->STATR & ADC_EOC) == 0 &&
           (uint32_t)SysTick->CNT - start < 5u * DELAY_MS_TIME) {
    }
    uint32_t in1 = ADC2->RDATAR;

    uart_puts(ok ? "ADC1_IN0=" : "ADC_TIMEOUT in0=");
    uart_putu(in0);
    uart_puts(" VREF=");
    uart_putu(vref);
    uart_puts(" ADC2_IN1=");
    uart_putu(in1);
    uart_puts(" irqs=");
    uart_putu(adc_irqs);
    uart_puts("\r\n");

    while ((USART1->STATR & USART_FLAG_TC) == 0) {
    }
    for (;;) {
    }
}
