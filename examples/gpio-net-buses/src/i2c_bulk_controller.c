/* STM32G071 I2C1 controller on gpio nets: 10-bit addressing, transfers
 * above 255 bytes (RELOAD), general call and a masked OAR2 address. PB6
 * SCL, PB7 SDA (AF6, open drain). TIMINGR: SCLL = SCLH = 5, SDADEL = 1,
 * SCLDEL = 2 kernel periods.
 *
 *   1. write 300 bytes (3i + 7) to 10-bit 0x2A5: NBYTES 255 + RELOAD, then
 *      45 + AUTOEND
 *   2. read 300 bytes from 10-bit 0x2A5 (write header, low byte, repeated
 *      START, read header), RELOAD the same way
 *   3. write to 10-bit 0x1A5, whose header nobody answers: NACK
 *   4. general call: one byte (0x06) to address 0
 *   5. one byte (0x5C) to 7-bit 0x31, the target's OAR2 0x30 with one bit
 *      masked
 *
 * RESULT[0] NACKF in step 1 (0), RESULT[1] sum of the bytes read,
 * RESULT[2] bytes read, RESULT[3] NACKF in step 3 (1), RESULT[4] STOPF
 * count, RESULT[5] NACKF in step 4 (0), RESULT[6] NACKF in step 5 (0),
 * RESULT[7] done.
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
#define TCR   (1u << 7)
#define RD_WRN  (1u << 10)
#define ADD10   (1u << 11)
#define START   (1u << 13)
#define RELOAD  (1u << 24)
#define AUTOEND (1u << 25)
#define NBYTES(n) ((uint32_t)(n) << 16)

static uint32_t stops;

static void wait_stop(void) {
    while (!(I2C1_ISR & STOPF)) {}
    I2C1_ICR = STOPF;
    stops += 1u;
}

/* The next chunk of a transfer: at most 255 bytes, RELOAD while more
 * follow, AUTOEND on the last. */
static uint32_t chunk(uint32_t left) {
    return left > 255u ? NBYTES(255) | RELOAD : NBYTES(left) | AUTOEND;
}

static uint32_t chunk_len(uint32_t left) {
    return left > 255u ? 255u : left;
}

/* Write n bytes of 3i + 7 to `sadd` (`mode` = ADD10 or 0). Returns 1 when
 * NACKed. */
static uint32_t write(uint32_t sadd, uint32_t mode, uint32_t n, const uint8_t *fixed) {
    uint32_t base = sadd | mode;
    I2C1_CR2 = base | chunk(n) | START;
    uint32_t in_chunk = chunk_len(n);
    for (uint32_t i = 0; i < n; ++i) {
        if (in_chunk == 0u) {
            while (!(I2C1_ISR & TCR)) {}
            I2C1_CR2 = base | chunk(n - i);
            in_chunk = chunk_len(n - i);
        }
        in_chunk -= 1u;
        uint32_t isr;
        do {
            isr = I2C1_ISR;
        } while (!(isr & (TXIS | NACKF)));
        if (isr & NACKF) {
            I2C1_ICR = NACKF;
            wait_stop();
            return 1u;
        }
        I2C1_TXDR = fixed ? fixed[0] : (uint8_t)(3u * i + 7u);
    }
    wait_stop();
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
    I2C1_TIMINGR = (2u << 20) | (1u << 16) | (4u << 8) | 4u;
    I2C1_CR1 = 1u;               /* PE */

    delay(400);                  /* let the target come up */
    RESULT[0] = write(0x2A5u, ADD10, 300u, 0);

    /* 300 bytes back, 10-bit, the full write-header/low/read-header turn. */
    uint32_t base = 0x2A5u | ADD10 | RD_WRN;
    uint32_t sum = 0u, got = 0u;
    I2C1_CR2 = base | chunk(300u) | START;
    uint32_t in_chunk = chunk_len(300u);
    for (uint32_t i = 0; i < 300u; ++i) {
        if (in_chunk == 0u) {
            while (!(I2C1_ISR & TCR)) {}
            I2C1_CR2 = base | chunk(300u - i);
            in_chunk = chunk_len(300u - i);
        }
        in_chunk -= 1u;
        uint32_t isr;
        do {
            isr = I2C1_ISR;
        } while (!(isr & (RXNE | NACKF)));
        if (isr & NACKF) {
            I2C1_ICR = NACKF;
            break;
        }
        sum += I2C1_RXDR;
        got += 1u;
    }
    wait_stop();
    RESULT[1] = sum;
    RESULT[2] = got;

    RESULT[3] = write(0x1A5u, ADD10, 1u, 0);
    static const uint8_t gc[1] = {0x06};
    RESULT[5] = write(0x00u, 0u, 1u, gc);
    static const uint8_t two[1] = {0x5C};
    RESULT[6] = write(0x31u << 1, 0u, 1u, two);
    RESULT[4] = stops;
    RESULT[7] = 1u;
    for (;;) {}
}

void reset(void) { (void)main(); for (;;) {} }
