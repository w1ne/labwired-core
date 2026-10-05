/* ESP32-C6 side of the gpio-net-two-boards example, in place of the
 * ATmega328P (env-esp32c6.yaml). Register level, no IDF, bare ELF. Same three
 * wires to the STM32:
 *
 *   irq    GPIO4  output, push-pull                         -> STM32 PB0
 *   ready  GPIO5  input, GPIO interrupt on the rising edge  <- STM32 PB1
 *   alert  GPIO6  open drain (GPIO_PIN6.PAD_DRIVER), shared, pull-up on the
 *                 net, GPIO interrupt on any edge           <-> STM32 PB4
 *
 * Counts with the GPIO interrupt: ETS_GPIO_INTR_SOURCE (30 on the C6) is
 * mapped to CPU line 9 in the interrupt matrix, line 9 enabled at priority 1
 * in INTPRI, and the RISC-V trap handler reads GPIO_STATUS and acknowledges
 * it in GPIO_STATUS_W1TC. Sequence as the AVR's:
 *   1. put 10 pulses on irq;
 *   2. count 7 rising edges on ready;
 *   3. count 5 falling and 5 rising edges on alert, then mask its interrupt;
 *   4. pull alert low 3 times (OUT 0 drives, OUT 1 releases);
 *   5. leave the counts in SRAM (the test reads them).
 *
 * SRAM 0x40800100: [0] ready rising  [1] alert falling  [2] alert rising
 *                  [3] interrupts taken  [4] 1 when done
 */
#include <stdint.h>

#define REG(a) (*(volatile uint32_t *)(a))
#define GPIO(o)          REG(0x60091000u + (o))
#define GPIO_OUT_W1TS    GPIO(0x08)
#define GPIO_OUT_W1TC    GPIO(0x0C)
#define GPIO_ENABLE_W1TS GPIO(0x24)
#define GPIO_IN          GPIO(0x3C)
#define GPIO_STATUS      GPIO(0x44)
#define GPIO_STATUS_W1TC GPIO(0x4C)
#define GPIO_PIN(n)      GPIO(0x74 + 4 * (n))
#define INTMTX_MAP(src)  REG(0x60010000u + 4 * (src))
#define INTPRI_ENABLE    REG(0x600C5000u)
#define INTPRI_PRI(n)    REG(0x600C500Cu + 4 * (n))
#define INTPRI_THRESH    REG(0x600C508Cu)
#define RESULT ((volatile uint32_t *)0x40800100u)

#define IRQ   4u
#define READY 5u
#define ALERT 6u
#define GPIO_SOURCE 30u
#define LINE 9u
#define PAD_DRIVER (1u << 2)
#define INT_TYPE(t) ((uint32_t)(t) << 7)  /* 1 rising, 3 any edge */
#define INT_ENA (1u << 13)

/* About a microsecond at 160 MHz. */
static void delay_us(uint32_t us) {
    for (volatile uint32_t i = 0; i < us * 32u; ++i) {}
}

void gpio_isr(void) {
    uint32_t st = GPIO_STATUS;
    GPIO_STATUS_W1TC = st;
    if (st & (1u << READY)) RESULT[0] += 1;
    if (st & (1u << ALERT)) {
        if (GPIO_IN & (1u << ALERT)) RESULT[2] += 1;
        else RESULT[1] += 1;
    }
    RESULT[3] += 1;
}

/* Direct-mode trap entry: save the caller-saved registers, count, MRET. */
__attribute__((naked, aligned(4))) void trap_entry(void) {
    __asm__ volatile(
        "addi sp, sp, -64\n"
        "sw ra, 0(sp)\n  sw t0, 4(sp)\n  sw t1, 8(sp)\n  sw t2, 12(sp)\n"
        "sw a0, 16(sp)\n sw a1, 20(sp)\n sw a2, 24(sp)\n sw a3, 28(sp)\n"
        "sw a4, 32(sp)\n sw a5, 36(sp)\n sw a6, 40(sp)\n sw a7, 44(sp)\n"
        "sw t3, 48(sp)\n sw t4, 52(sp)\n sw t5, 56(sp)\n sw t6, 60(sp)\n"
        "call gpio_isr\n"
        "lw ra, 0(sp)\n  lw t0, 4(sp)\n  lw t1, 8(sp)\n  lw t2, 12(sp)\n"
        "lw a0, 16(sp)\n lw a1, 20(sp)\n lw a2, 24(sp)\n lw a3, 28(sp)\n"
        "lw a4, 32(sp)\n lw a5, 36(sp)\n lw a6, 40(sp)\n lw a7, 44(sp)\n"
        "lw t3, 48(sp)\n lw t4, 52(sp)\n lw t5, 56(sp)\n lw t6, 60(sp)\n"
        "addi sp, sp, 64\n"
        "mret\n");
}

int main(void) {
    /* irq: push-pull output, low. */
    GPIO_OUT_W1TC = 1u << IRQ;
    GPIO_ENABLE_W1TS = 1u << IRQ;
    /* alert: open drain, latch 1 = released, output stage on. */
    GPIO_PIN(ALERT) = PAD_DRIVER;
    GPIO_OUT_W1TS = 1u << ALERT;
    GPIO_ENABLE_W1TS = 1u << ALERT;

    /* GPIO interrupt -> CPU line 9, priority 1 >= threshold 1. */
    INTMTX_MAP(GPIO_SOURCE) = LINE;
    INTPRI_PRI(LINE) = 1u;
    INTPRI_THRESH = 1u;
    INTPRI_ENABLE |= 1u << LINE;
    __asm__ volatile("csrw mtvec, %0" ::"r"((uint32_t)trap_entry));
    GPIO_PIN(READY) = INT_TYPE(1) | INT_ENA;
    GPIO_PIN(ALERT) = PAD_DRIVER | INT_TYPE(3) | INT_ENA;
    GPIO_STATUS_W1TC = 0xFFFFFFFFu;
    __asm__ volatile("csrsi mstatus, 8" ::: "memory");
    delay_us(1000);                      /* let the STM32 arm EXTI */

    for (int i = 0; i < 10; ++i) {
        GPIO_OUT_W1TS = 1u << IRQ;
        delay_us(20);
        GPIO_OUT_W1TC = 1u << IRQ;
        delay_us(20);
    }

    while (RESULT[0] < 7u) {}
    while (RESULT[1] < 5u || RESULT[2] < 5u) {}
    GPIO_PIN(ALERT) = PAD_DRIVER;        /* our own pulls are not counted */

    delay_us(100);
    for (int i = 0; i < 3; ++i) {
        GPIO_OUT_W1TC = 1u << ALERT;     /* drive low */
        delay_us(30);
        GPIO_OUT_W1TS = 1u << ALERT;     /* release */
        delay_us(30);
    }
    RESULT[4] = 1u;
    for (;;) {}
}

__attribute__((naked, section(".text.start"))) void _start(void) {
    __asm__ volatile(
        "li sp, 0x40880000\n"
        "call main\n"
        "1: j 1b\n");
}
