/* STM32G071 I2C1 target at 0x42 on gpio nets: PB6 SCL, PB7 SDA (AF6, open
 * drain). A 256-byte register file: the first byte of a write sets the
 * pointer, later bytes store and reads return from it, auto-incrementing.
 * Interrupt driven (ADDR, RXNE, TXIS, STOPF); the controller's SCL is
 * stretched while the handler runs.
 *
 * RESULT[0] transfers ended by STOP, RESULT[1..2] regs[0x10..0x11],
 * RESULT[3] bytes received, RESULT[4] bytes sent.
 */
#include "g0.h"

void i2c1_handler(void);

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 32] = {
    [0] = 0x20009000u,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
    [16 + 23] = (uintptr_t)i2c1_handler,
};

static uint8_t regs[256];
static uint8_t ptr;
static uint32_t have_ptr;

void i2c1_handler(void) {
    uint32_t isr = I2C1_ISR;
    if (isr & (1u << 3)) {                  /* ADDR */
        if (isr & (1u << 16)) {             /* DIR: we transmit */
            I2C1_ISR = 1u;                  /* flush TXDR */
        } else {
            have_ptr = 0u;
        }
        I2C1_ICR = 1u << 3;
    }
    if (isr & (1u << 2)) {                  /* RXNE */
        uint8_t b = (uint8_t)I2C1_RXDR;
        if (!have_ptr) {
            ptr = b;
            have_ptr = 1u;
        } else {
            regs[ptr++] = b;
        }
        RESULT[3] += 1u;
    }
    if (I2C1_ISR & (1u << 1)) {             /* TXIS */
        I2C1_TXDR = regs[ptr++];
        RESULT[4] += 1u;
    }
    if (isr & (1u << 5)) {                  /* STOPF */
        I2C1_ICR = 1u << 5;
        RESULT[0] += 1u;
        RESULT[1] = regs[0x10];
        RESULT[2] = regs[0x11];
    }
}

int main(void) {
    RCC_IOPENR |= 2u;
    RCC_APBENR1 |= 1u << 21;
    GPIOB_OTYPER |= (1u << 6) | (1u << 7);
    for (unsigned pin = 6; pin <= 7; ++pin) {
        set_af_low(&GPIOB_AFRL, pin, 6);
        set_mode(&GPIOB_MODER, pin, 2);
    }
    I2C1_TIMINGR = (4u << 20) | (2u << 16) | (15u << 8) | 15u;
    I2C1_OAR1 = (1u << 15) | (0x42u << 1);
    /* PE, TXIE, RXIE, ADDRIE, STOPIE */
    I2C1_CR1 = 1u | (1u << 1) | (1u << 2) | (1u << 3) | (1u << 5);
    NVIC_ISER = 1u << 23;
    __asm__ volatile("cpsie i" ::: "memory");
    for (;;) {
        __asm__ volatile("wfi");
    }
}

void reset(void) { (void)main(); for (;;) {} }
