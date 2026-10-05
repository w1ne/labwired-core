/* RP2040 side of the gpio-net-two-boards example, in place of the ATmega328P
 * (env-rp2040.yaml). Register level, no SDK. Same three wires to the STM32:
 *
 *   irq    GP2  output, push-pull                 -> STM32 PB0
 *   ready  GP3  input, IO_BANK0 EDGE_HIGH irq     <- STM32 PB1
 *   alert  GP4  open drain by OE (OUT stays 0), shared, pull-up on the net,
 *               IO_BANK0 EDGE_LOW + EDGE_HIGH irq <-> STM32 PB4
 *
 * Unlike the AVR this side counts with GPIO interrupts (IO_IRQ_BANK0, NVIC
 * 13), not by polling. Sequence:
 *   1. put 10 pulses on irq;
 *   2. count 7 rising edges on ready (interrupt);
 *   3. count 5 falling and 5 rising edges on alert (interrupt), then mask it;
 *   4. pull alert low 3 times (OE on = drive the 0, OE off = release);
 *   5. leave the counts in SRAM (the test reads them).
 *
 * SRAM 0x20000100: [0] ready rising  [1] alert falling  [2] alert rising
 *                  [3] interrupts taken  [4] 1 when done
 */
#include <stdint.h>

#define REG(a) (*(volatile uint32_t *)(a))
#define IO_CTRL(n)   REG(0x40014004u + 8u * (n))   /* GPIOn_CTRL */
#define IO_INTR0     REG(0x400140F0u)
#define IO_P0_INTE0  REG(0x40014100u)
#define IO_P0_INTS0  REG(0x40014120u)
#define SIO_OUT_SET  REG(0xD0000014u)
#define SIO_OUT_CLR  REG(0xD0000018u)
#define SIO_OE_SET   REG(0xD0000024u)
#define SIO_OE_CLR   REG(0xD0000028u)
#define NVIC_ISER    REG(0xE000E100u)
#define RESULT ((volatile uint32_t *)0x20000100u)

#define IRQ   2u
#define READY 3u
#define ALERT 4u
/* INTR / INTE nibble of pad n: LEVEL_LOW, LEVEL_HIGH, EDGE_LOW, EDGE_HIGH. */
#define EDGE_LOW(n)  (1u << (4u * (n) + 2u))
#define EDGE_HIGH(n) (1u << (4u * (n) + 3u))

/* About a microsecond at 125 MHz (a pass is a handful of instructions). */
static void delay_us(uint32_t us) {
    for (volatile uint32_t i = 0; i < us * 25u; ++i) {}
}

void reset(void);
void default_handler(void) { for (;;) {} }
void io_bank0_handler(void);

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 26] = {
    [0] = 0x20040000u,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
    [16 + 13] = (uintptr_t)io_bank0_handler,
};

void io_bank0_handler(void) {
    uint32_t s = IO_P0_INTS0;
    IO_INTR0 = s & (EDGE_HIGH(READY) | EDGE_LOW(ALERT) | EDGE_HIGH(ALERT));
    if (s & EDGE_HIGH(READY)) RESULT[0] += 1;
    if (s & EDGE_LOW(ALERT)) RESULT[1] += 1;
    if (s & EDGE_HIGH(ALERT)) RESULT[2] += 1;
    RESULT[3] += 1;
}

int main(void);
void reset(void) { (void)main(); for (;;) {} }

int main(void) {
    IO_CTRL(IRQ) = 5u;                   /* FUNCSEL = SIO */
    IO_CTRL(READY) = 5u;
    IO_CTRL(ALERT) = 5u;
    SIO_OUT_CLR = (1u << IRQ) | (1u << ALERT);
    SIO_OE_SET = 1u << IRQ;              /* alert: OE off = released */

    IO_INTR0 = 0xFFFFFFFFu;              /* drop edges latched at power-up */
    IO_P0_INTE0 = EDGE_HIGH(READY) | EDGE_LOW(ALERT) | EDGE_HIGH(ALERT);
    NVIC_ISER = 1u << 13;
    __asm__ volatile("cpsie i" ::: "memory");
    delay_us(1000);                      /* let the STM32 arm EXTI */

    for (int i = 0; i < 10; ++i) {
        SIO_OUT_SET = 1u << IRQ;
        delay_us(20);
        SIO_OUT_CLR = 1u << IRQ;
        delay_us(20);
    }

    while (RESULT[0] < 7u) {}
    while (RESULT[1] < 5u || RESULT[2] < 5u) {}
    IO_P0_INTE0 = EDGE_HIGH(READY);      /* our own pulls are not counted */

    delay_us(100);
    for (int i = 0; i < 3; ++i) {
        SIO_OE_SET = 1u << ALERT;        /* drive low */
        delay_us(30);
        SIO_OE_CLR = 1u << ALERT;        /* release */
        delay_us(30);
    }
    RESULT[4] = 1u;
    for (;;) {}
}
