// Reference-derived AT32F421G8U7 smoke; built against pinned Artery BSP 2.1.9.
// This is a peripheral contract test, not ESC firmware or a flight test.
#include "at32f421_conf.h"

extern unsigned _sidata, _sdata, _edata, _sbss, _ebss, _estack;
static volatile unsigned timer_irqs;
static const unsigned source[4] = {0x12345678, 0xa5a5aa55, 0x55aa5a5a, 0xfedcba98};
static volatile unsigned destination[4];
void Reset_Handler(void);
static void Fault_Handler(void) { for (;;) __asm volatile("nop"); }
static void TMR6_Handler(void) {
    tmr_flag_clear(TMR6, TMR_OVF_FLAG);
    timer_irqs++;
    gpio_bits_toggle(GPIOA, GPIO_PINS_5);
}
__attribute__((section(".isr_vector"), used))
void (*const vectors[96])(void) = {
    [0] = (void (*)(void))&_estack, [1] = Reset_Handler,
    [2] = Fault_Handler, [3] = Fault_Handler, [4] = Fault_Handler,
    [5] = Fault_Handler, [6] = Fault_Handler, [16 + 17] = TMR6_Handler,
};
static void puts_uart(const char *s) {
    while (*s) {
        while (usart_flag_get(USART2, USART_TDBE_FLAG) == RESET) {}
        usart_data_transmit(USART2, (unsigned char)*s++);
    }
}
static void fail(const char *s) { puts_uart(s); for (;;) __asm volatile("nop"); }
static void main_smoke(void) {
    flash_psr_set(FLASH_WAIT_CYCLE_3);
    crm_reset();
    crm_pll_config(CRM_PLL_SOURCE_HICK, CRM_PLL_MULT_30);
    crm_clock_source_enable(CRM_CLOCK_SOURCE_PLL, TRUE);
    while (crm_flag_get(CRM_PLL_STABLE_FLAG) == RESET) {}
    crm_sysclk_switch(CRM_SCLK_PLL);
    while (crm_sysclk_switch_status_get() != CRM_SCLK_PLL) {}
    system_core_clock_update();
    crm_periph_clock_enable(CRM_GPIOA_PERIPH_CLOCK, TRUE);
    crm_periph_clock_enable(CRM_GPIOB_PERIPH_CLOCK, TRUE);
    crm_periph_clock_enable(CRM_USART2_PERIPH_CLOCK, TRUE);
    gpio_init_type gpio;
    gpio_default_para_init(&gpio);
    gpio.gpio_mode = GPIO_MODE_MUX;
    gpio.gpio_pins = GPIO_PINS_2;
    gpio_init(GPIOA, &gpio);
    gpio_pin_mux_config(GPIOA, GPIO_PINS_SOURCE2, GPIO_MUX_1);
    gpio.gpio_mode = GPIO_MODE_OUTPUT;
    gpio.gpio_pins = GPIO_PINS_5;
    gpio_init(GPIOA, &gpio);
    usart_init(USART2, 115200, USART_DATA_8BITS, USART_STOP_1_BIT);
    usart_transmitter_enable(USART2, TRUE);
    usart_enable(USART2, TRUE);
    puts_uart("AT32 START\n");
    if (system_core_clock != 120000000) fail("FAIL CLOCK\n");

    flash_unlock();
    if (flash_sector_erase(0x0800fc00) != FLASH_OPERATE_DONE) fail("FAIL ERASE\n");
    if (*(volatile unsigned *)0x0800fc00 != 0xffffffff) fail("FAIL ERASED DATA\n");
    if (flash_word_program(0x0800fc00, 0x12345678) != FLASH_OPERATE_DONE) fail("FAIL PROGRAM\n");
    if (*(volatile unsigned *)0x0800fc00 != 0x12345678) fail("FAIL FLASH DATA\n");
    flash_lock();
    puts_uart("FLASH OK\n");

    crm_periph_clock_enable(CRM_DMA1_PERIPH_CLOCK, TRUE);
    DMA1_CHANNEL1->paddr = (unsigned)source;
    DMA1_CHANNEL1->maddr = (unsigned)destination;
    DMA1_CHANNEL1->dtcnt = 4;
    // M2M, word widths, increment both pointers, peripheral-to-memory.
    DMA1_CHANNEL1->ctrl = (1u << 14) | (2u << 10) | (2u << 8) | (1u << 7) | (1u << 6) | 1;
    while (!(DMA1->sts & DMA1_FDT1_FLAG)) {}
    for (unsigned i = 0; i < 4; i++) if (destination[i] != source[i]) fail("FAIL DMA\n");
    puts_uart("DMA OK\n");

    // TMR3 CH1 on bonded PB4/AF1. Enable the physical output route.
    gpio.gpio_mode = GPIO_MODE_MUX;
    gpio.gpio_pins = GPIO_PINS_4;
    gpio_init(GPIOB, &gpio);
    gpio_pin_mux_config(GPIOB, GPIO_PINS_SOURCE4, GPIO_MUX_1);
    crm_periph_clock_enable(CRM_TMR3_PERIPH_CLOCK, TRUE);
    TMR3->pr = 99; TMR3->div = 119; TMR3->c1dt = 50;
    TMR3->cm1 = 0x60; TMR3->cctrl = 1; TMR3->swevt = 1; TMR3->ctrl1 = 1;
    crm_periph_clock_enable(CRM_TMR6_PERIPH_CLOCK, TRUE);
    tmr_base_init(TMR6, 99, 119);
    tmr_interrupt_enable(TMR6, TMR_OVF_INT, TRUE);
    NVIC_EnableIRQ(TMR6_GLOBAL_IRQn);
    tmr_counter_enable(TMR6, TRUE);
    while (timer_irqs < 8) {}
    puts_uart("TIMER IRQ OK\nAT32F421G8U7 IO OK\n");
    for (;;) __asm volatile("nop");
}
void Reset_Handler(void) {
    unsigned *d = &_sdata, *s = &_sidata;
    while (d < &_edata) *d++ = *s++;
    for (d = &_sbss; d < &_ebss; d++) *d = 0;
    main_smoke();
}
