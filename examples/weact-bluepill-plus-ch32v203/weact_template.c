/*
 * WeAct BluePill Plus (CH32V203C8T6): clean-room bare-metal rewrite of the
 * behaviour of WeAct's own template, WeActStudio.BluePill-Plus-CH32
 * Examples/CH32V203/01-template (User/main.c, User/board.c; MIT). No WeAct or
 * WCH SDK source is copied. Behaviour kept from the template:
 *   - the same banner on USART1 TX (PA9), 115200 baud;
 *   - the blue LED on PB2 (push-pull, active high) toggles every
 *     `toggle_tick` passes of a 1 ms loop, starting at 250;
 *   - KEY on PA0 is an input with the internal pull-down; a press waits for
 *     the release in 50 ms steps, then swaps `toggle_tick` between 250 and
 *     100.
 * One difference: the template's SystemInit runs the 8 MHz HSE through the
 * PLL to 144 MHz. This build stays on the 8 MHz HSI reset clock, because the
 * engine's F1 RCC model does not decode the V20x PLLMUL x18 setting. The
 * delay is derived from the same SystemCoreClock / 8 tick as the WCH SDK.
 *
 * Register addresses and bits: WCH CH32FV2x_V3xRM (RCC, GPIO, USART, STK).
 */
#include <stdint.h>

#define RCC_APB2PCENR (*(volatile uint32_t *)0x40021018u)
#define GPIOA_CFGLR   (*(volatile uint32_t *)0x40010800u)
#define GPIOA_CFGHR   (*(volatile uint32_t *)0x40010804u)
#define GPIOA_INDR    (*(volatile uint32_t *)0x40010808u)
#define GPIOA_OUTDR   (*(volatile uint32_t *)0x4001080Cu)
#define GPIOB_CFGLR   (*(volatile uint32_t *)0x40010C00u)
#define GPIOB_OUTDR   (*(volatile uint32_t *)0x40010C0Cu)
#define GPIOB_BSHR    (*(volatile uint32_t *)0x40010C10u)
#define GPIOB_BCR     (*(volatile uint32_t *)0x40010C14u)
#define USART1_STATR  (*(volatile uint32_t *)0x40013800u)
#define USART1_DATAR  (*(volatile uint32_t *)0x40013804u)
#define USART1_BRR    (*(volatile uint32_t *)0x40013808u)
#define USART1_CTLR1  (*(volatile uint32_t *)0x4001380Cu)
#define STK_CTLR      (*(volatile uint32_t *)0xE000F000u)
#define STK_SR        (*(volatile uint32_t *)0xE000F004u)
#define STK_CMPL      (*(volatile uint32_t *)0xE000F010u)
#define STK_CMPH      (*(volatile uint32_t *)0xE000F014u)

#define RCC_IOPAEN      (1u << 2)
#define RCC_IOPBEN      (1u << 3)
#define RCC_USART1EN    (1u << 14)
#define USART_STATR_TC  (1u << 6)
#define USART_STATR_TXE (1u << 7)
#define USART_CTLR1_TE  (1u << 3)
#define USART_CTLR1_UE  (1u << 13)
#define STK_STE         (1u << 0)
#define STK_MODE_DOWN   (1u << 4)
#define STK_INIT        (1u << 5)
#define STK_CNTIF       (1u << 0)

#define LED_PIN 2u
#define KEY_PIN 0u

/* HSI reset clock (RM section 3.2.1). */
static const uint32_t system_core_clock = 8000000u;

/* The WCH SDK Delay_Ms: a down count of n * (HCLK / 8 / 1000) STK ticks,
 * STCLK = 0 (HCLK / 8), then poll CNTIF. */
static void delay_ms(uint32_t n) {
    uint32_t ticks = n * (system_core_clock / 8000u);
    STK_SR &= ~STK_CNTIF;
    STK_CMPL = ticks;
    STK_CMPH = 0;
    STK_CTLR |= STK_MODE_DOWN;
    STK_CTLR |= STK_INIT | STK_STE;
    while ((STK_SR & STK_CNTIF) == 0) {
    }
    STK_CTLR &= ~STK_STE;
}

static void uart_putc(char c) {
    while ((USART1_STATR & USART_STATR_TXE) == 0) {
    }
    USART1_DATAR = (uint8_t)c;
}

static void uart_puts(const char *s) {
    while (*s) {
        uart_putc(*s++);
    }
}

static void uart_putu(uint32_t v) {
    char buf[10];
    int n = 0;
    do {
        buf[n++] = (char)('0' + v % 10u);
        v /= 10u;
    } while (v != 0u);
    while (n > 0) {
        uart_putc(buf[--n]);
    }
}

static void usart1_init(void) {
    /* PA9 TX: alternate-function push-pull, 50 MHz (CFGHR bits 7:4). */
    RCC_APB2PCENR |= RCC_IOPAEN | RCC_USART1EN;
    GPIOA_CFGHR = (GPIOA_CFGHR & ~(0xFu << 4)) | (0xBu << 4);
    /* 8 MHz / 115200: mantissa 4, fraction 5 (USARTDIV = 4.34). */
    USART1_BRR = 0x0045u;
    USART1_CTLR1 = USART_CTLR1_UE | USART_CTLR1_TE;
}

static void board_button_init(void) {
    RCC_APB2PCENR |= RCC_IOPAEN;
    /* PA0: input with pull-up/pull-down (CNF = 10, MODE = 00); ODR 0 = down. */
    GPIOA_CFGLR = (GPIOA_CFGLR & ~(0xFu << (KEY_PIN * 4u))) | (0x8u << (KEY_PIN * 4u));
    GPIOA_OUTDR &= ~(1u << KEY_PIN);
}

static uint32_t board_button_getstate(void) {
    return (GPIOA_INDR >> KEY_PIN) & 1u;
}

static void board_led_init(void) {
    RCC_APB2PCENR |= RCC_IOPBEN;
    /* PB2: general-purpose push-pull output, 50 MHz (CNF = 00, MODE = 11). */
    GPIOB_CFGLR = (GPIOB_CFGLR & ~(0xFu << (LED_PIN * 4u))) | (0x3u << (LED_PIN * 4u));
}

static void board_led_toggle(void) {
    if (GPIOB_OUTDR & (1u << LED_PIN)) {
        GPIOB_BCR = 1u << LED_PIN;
    } else {
        GPIOB_BSHR = 1u << LED_PIN;
    }
}

void main(void) {
    usart1_init();
    board_button_init();
    board_led_init();

    uart_puts("\r\nWeAct Studio Core Board\r\n");
    uart_puts("weactstudio.taobao.com\r\n");
    uart_puts("weactstudio.aliexpress.com\r\n");
    uart_puts("blog.weactstudio.com\r\n\r\n");
    uart_puts("SystemClk:");
    uart_putu(system_core_clock);
    uart_puts("\r\n");
    uart_puts("Template\r\n");
    while ((USART1_STATR & USART_STATR_TC) == 0) {
    }

    uint32_t tick = 0;
    uint32_t toggle_tick = 250;
    for (;;) {
        tick++;
        if (tick % toggle_tick == 0) {
            board_led_toggle();
        }
        if (board_button_getstate()) {
            while (board_button_getstate()) {
                delay_ms(50);
            }
            toggle_tick = (toggle_tick == 250) ? 100 : 250;
        }
        delay_ms(1);
    }
}
