/* STM32G071 I2C1 controller on gpio nets: PB6 SCL, PB7 SDA (AF6, open
 * drain; the pull-up is the net's). TIMINGR: SCLL = SCLH = 16, SDADEL = 2,
 * SCLDEL = 4 kernel periods.
 *
 *   1. write 0x10 0xDE 0xAD to target 0x42 (AUTOEND)
 *   2. write 0x10, repeated START, read 2 bytes from 0x42 (AUTOEND)
 *   3. write one byte to 0x50, where nobody answers: NACK
 *
 * RESULT[0..1] the two bytes read (0xEE when step 2 was NACKed), RESULT[2] NACKF seen in step 1 (0),
 * RESULT[3] NACKF seen in step 3 (1), RESULT[4] STOPF count, RESULT[7] done.
 */
#include "g0.h"

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 32] = {
    [0] = 0x20009000u,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
};

#define TXIS  (1u << 1)
#define RXNE  (1u << 2)
#define NACKF (1u << 4)
#define STOPF (1u << 5)
#define TC    (1u << 6)
#define START (1u << 13)
#define AUTOEND (1u << 25)
#define RD_WRN (1u << 10)
#define NBYTES(n) ((uint32_t)(n) << 16)
#define SADD(a) ((uint32_t)(a) << 1)

static uint32_t stops;

static void wait_stop(void) {
    while (!(I2C1_ISR & STOPF)) {}
    I2C1_ICR = STOPF;
    stops += 1u;
}

/* Returns 1 when the transfer was NACKed. */
static uint32_t write(uint8_t addr, const uint8_t *data, uint32_t n, uint32_t autoend) {
    I2C1_CR2 = SADD(addr) | NBYTES(n) | (autoend ? AUTOEND : 0u) | START;
    for (uint32_t i = 0; i < n; ++i) {
        uint32_t isr;
        do {
            isr = I2C1_ISR;
        } while (!(isr & (TXIS | NACKF)));
        if (isr & NACKF) {
            I2C1_ICR = NACKF;
            wait_stop();
            return 1u;
        }
        I2C1_TXDR = data[i];
    }
    if (autoend) {
        wait_stop();
    } else {
        while (!(I2C1_ISR & TC)) {}
    }
    return 0u;
}

int main(void) {
    RCC_IOPENR |= 2u;            /* GPIOB */
    RCC_APBENR1 |= 1u << 21;     /* I2C1 */
    GPIOB_OTYPER |= (1u << 6) | (1u << 7);
    for (unsigned pin = 6; pin <= 7; ++pin) {
        set_af_low(&GPIOB_AFRL, pin, 6);
        set_mode(&GPIOB_MODER, pin, 2);
    }
    I2C1_TIMINGR = (4u << 20) | (2u << 16) | (15u << 8) | 15u;
    I2C1_CR1 = 1u;               /* PE */

    delay(400);                  /* let the target come up */
    static const uint8_t first[3] = {0x10, 0xDE, 0xAD};
    RESULT[2] = write(0x42, first, 3, 1);

    static const uint8_t ptr[1] = {0x10};
    if (write(0x42, ptr, 1, 0) == 0u) {
        I2C1_CR2 = SADD(0x42) | RD_WRN | NBYTES(2) | AUTOEND | START;
        for (int i = 0; i < 2; ++i) {
            while (!(I2C1_ISR & RXNE)) {}
            RESULT[i] = I2C1_RXDR;
        }
        wait_stop();
    } else {
        RESULT[0] = RESULT[1] = 0xEEu;   /* nobody to read from */
    }

    static const uint8_t lost[1] = {0x55};
    RESULT[3] = write(0x50, lost, 1, 1);
    RESULT[4] = stops;
    RESULT[7] = 1u;
    for (;;) {}
}

void reset(void) { (void)main(); for (;;) {} }
