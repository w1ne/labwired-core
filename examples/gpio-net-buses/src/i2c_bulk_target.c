/* STM32G071 I2C1 target on gpio nets: own 10-bit address 0x2A5 (OAR1
 * OA1MODE), OAR2 0x30 with OA2MSK = 1 (answers 0x30 and 0x31), general call
 * (GCEN). Interrupt driven; SCL is stretched while the handler catches up.
 * Bytes written over the 10-bit address are checked against 3i + 7; a read
 * gets 5i + 1.
 *
 * RESULT[0] bytes received at 0x2A5, RESULT[1] their sum, RESULT[2]
 * mismatches against 3i + 7, RESULT[3] bytes loaded into TXDR, RESULT[4]
 * STOPF count, RESULT[5] the general-call byte, RESULT[6] the byte written
 * to the OAR2 address, RESULT[8..] ADDCODE|DIR<<7 of each ADDR, in order.
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

static uint32_t addcode;
static uint32_t rx;
static uint32_t tx;
static uint32_t addrs;

void i2c1_handler(void) {
    uint32_t isr = I2C1_ISR;
    if (isr & (1u << 3)) {                  /* ADDR */
        addcode = (isr >> 17) & 0x7Fu;
        if (addrs < 8u) {
            RESULT[8 + addrs] = addcode | (((isr >> 16) & 1u) << 7);
        }
        addrs += 1u;
        if (isr & (1u << 16)) {             /* DIR: we transmit */
            I2C1_ISR = 1u;                  /* flush TXDR */
            tx = 0u;
        }
        I2C1_ICR = 1u << 3;
    }
    if (isr & (1u << 2)) {                  /* RXNE */
        uint8_t b = (uint8_t)I2C1_RXDR;
        if (addcode == 0x7Au) {             /* 11110 + A9 A8 = 10 */
            if (b != (uint8_t)(3u * rx + 7u)) {
                RESULT[2] += 1u;
            }
            rx += 1u;
            RESULT[0] = rx;
            RESULT[1] += b;
        } else if (addcode == 0u) {
            RESULT[5] = b;
        } else {
            RESULT[6] = b;
        }
    }
    if (I2C1_ISR & (1u << 1)) {             /* TXIS */
        I2C1_TXDR = (uint8_t)(5u * tx + 1u);
        tx += 1u;
        RESULT[3] = tx;
    }
    if (isr & (1u << 5)) {                  /* STOPF */
        I2C1_ICR = 1u << 5;
        RESULT[4] += 1u;
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
    I2C1_TIMINGR = (2u << 20) | (1u << 16) | (4u << 8) | 4u;
    I2C1_OAR1 = (1u << 15) | (1u << 10) | 0x2A5u;
    I2C1_OAR2 = (1u << 15) | (1u << 8) | (0x30u << 1);
    /* PE, TXIE, RXIE, ADDRIE, STOPIE, GCEN */
    I2C1_CR1 = 1u | (1u << 1) | (1u << 2) | (1u << 3) | (1u << 5) | (1u << 19);
    NVIC_ISER = 1u << 23;
    __asm__ volatile("cpsie i" ::: "memory");
    for (;;) {
        __asm__ volatile("wfi");
    }
}

void reset(void) { (void)main(); for (;;) {} }
