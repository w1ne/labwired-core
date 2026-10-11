/* ENGINE node: throttle potentiometer on PA0 (ADC1 channel 0) -> RPM frame.
 *
 * CAN ID 0x100, DLC 2, every 10 ms: RPM big-endian,
 *   rpm = 800 + adc * 6200 / 4095      (800 rpm idle .. 7000 rpm full throttle)
 *
 * TIM2 fires every 10 ms. Its handler collects the conversion started on the
 * previous tick, starts the next one and queues the frame, so nothing waits
 * on the ADC. The main loop only sleeps (WFI).
 */
#include "common.h"

#define CAN_ID_RPM 0x100u
#define PERIOD_MS 10u

#define ADC1_BASE 0x40012400u
#define ADC1_SR REG32(ADC1_BASE + 0x00u)
#define ADC1_CR2 REG32(ADC1_BASE + 0x08u)
#define ADC1_SMPR2 REG32(ADC1_BASE + 0x10u)
#define ADC1_SQR3 REG32(ADC1_BASE + 0x34u)
#define ADC1_DR REG32(ADC1_BASE + 0x4Cu)
#define ADC_CR2_ADON (1u << 0)
#define ADC_CR2_RSTCAL (1u << 3)
#define ADC_CR2_CAL (1u << 2)
#define ADC_CR2_EXTSEL_SWSTART (7u << 17)
#define ADC_CR2_EXTTRIG (1u << 20)
#define ADC_CR2_SWSTART (1u << 22)
#define ADC_SR_EOC (1u << 1)

static volatile uint32_t g_adc; /* last completed conversion, 0..4095 */

static void adc_init(void)
{
    RCC_APB2ENR |= RCC_APB2ENR_IOPAEN | RCC_APB2ENR_ADC1EN;
    gpio_crl_set(GPIOA_BASE, 0u, CNF_ANALOG); /* PA0 = ADC1_IN0 */
    ADC1_SQR3 = 0u;                            /* one regular conversion: channel 0 */
    ADC1_SMPR2 = 7u;                           /* 239.5 cycles: a potentiometer is a high-Z source */
    ADC1_CR2 = ADC_CR2_ADON;                   /* power on */
    for (volatile unsigned i = 0; i < 100u; i++) { /* tSTAB, once at boot */
    }
    ADC1_CR2 |= ADC_CR2_RSTCAL;
    while ((ADC1_CR2 & ADC_CR2_RSTCAL) != 0u) { /* calibration, once at boot */
    }
    ADC1_CR2 |= ADC_CR2_CAL;
    while ((ADC1_CR2 & ADC_CR2_CAL) != 0u) {
    }
    ADC1_CR2 = ADC_CR2_ADON | ADC_CR2_EXTSEL_SWSTART | ADC_CR2_EXTTRIG;
}

static void adc_start(void) { ADC1_CR2 |= ADC_CR2_SWSTART; }

static void send_rpm(void)
{
    uint32_t rpm = 800u + g_adc * 6200u / 4095u;
    /* Big-endian on the wire: byte 0 = high byte. TDLR byte 0 is bits 7:0. */
    can_send(CAN_ID_RPM, ((rpm >> 8) & 0xFFu) | ((rpm & 0xFFu) << 8), 0u, 2u);
}

void TIM2_IRQHandler(void)
{
    TIM_SR(TIM2_BASE) = 0u; /* clear UIF */
    if ((ADC1_SR & ADC_SR_EOC) != 0u) {
        g_adc = ADC1_DR & 0xFFFu; /* reading DR clears EOC */
    }
    adc_start();
    send_rpm();
}

int main(void)
{
    clock_init_72mhz();
    adc_init();
    can_init(CAN_BTR_FOR(500000u), 0xFFFFFFFFu, 0xFFFFFFFFu);
    adc_start();
    while ((ADC1_SR & ADC_SR_EOC) == 0u) { /* first sample before the first frame (once) */
    }
    g_adc = ADC1_DR & 0xFFFu;

    RCC_APB1ENR |= RCC_APB1ENR_TIM2EN;
    timer_start_100us(TIM2_BASE, PERIOD_MS * 10u);
    nvic_enable(IRQ_TIM2);

    for (;;) {
        __WFI();
    }
}
