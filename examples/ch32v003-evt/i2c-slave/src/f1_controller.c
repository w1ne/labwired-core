/* STM32F103 I2C1 controller (PB6 SCL, PB7 SDA, 100 kHz, polled, RM0008
 * §26.3.3 master sequences) for ch32fun's examples/i2c_slave running
 * unmodified on a CH32V003 at 0x09. That example is a 3-register EEPROM-style
 * slave: the first byte of a write sets the offset, later bytes store from it
 * (bytes past the end are dropped), reads return from the offset (0x00 past
 * the end), and its main loop drives PA2 from bit 0 of register 0.
 *
 *   1. write 0x01 0xAB 0xCD             regs[1..2] = AB CD
 *   2. write 0x02 0x5A                  regs[2] = 5A
 *   3. write 0x00 0x01                  regs[0] = 01: PA2 high
 *   4. write 0x00, repeated START, read 4 bytes: 01 AB 5A, then 00 past
 *      the end
 *   5. read 1 byte without a pointer write: 00, the byte the slave loaded
 *      into DR while step 4's last byte was being NACKed (see test.yaml)
 *   6. write one byte to 0x50, where nobody answers: AF
 *
 * Built with -DPAST_END=1, step 2 also writes 0x77 past the end of the
 * array. The slave never reads that byte out of DR, which wedges the bus
 * (see test-past-end.yaml).
 *
 * The CH32V003 boots slower than this controller, so step 1 retries its
 * address until the slave ACKs. RESULT[0..3] the bytes read in step 4,
 * RESULT[4] the byte read in step 5, RESULT[5] address NACKs before the
 * slave came up, RESULT[6] AF in step 6 (1), RESULT[7] = 1 done.
 */
#define CHIP_F1 1
#ifndef PAST_END
#define PAST_END 0
#endif
#include "../../../gpio-net-buses/src/legacy_i2c.h"

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 64] = {
    [0] = STACK_TOP,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
};

static void stop(void) {
    I2C1_CR1 |= CR1_STOP;
    while (I2C1_SR2 & 2u) {}            /* BUSY until STOP is on the wire */
}

/* START and the address byte. Returns 1 when the address was NACKed (the
 * bus is then stopped). */
static uint32_t address(uint8_t byte) {
    I2C1_CR1 |= CR1_START;
    while (!(I2C1_SR1 & SR1_SB)) {}
    I2C1_DR = byte;
    uint32_t sr1;
    do {
        sr1 = I2C1_SR1;
    } while (!(sr1 & (SR1_ADDR | SR1_AF)));
    if (sr1 & SR1_AF) {
        I2C1_SR1 = ~SR1_AF;
        stop();
        return 1u;
    }
    return 0u;
}

/* Write n bytes; STOP unless `hold`. Returns 1 on NACK. */
static uint32_t write(uint8_t addr, const uint8_t *data, uint32_t n, uint32_t hold) {
    if (address((uint8_t)(addr << 1))) {
        return 1u;
    }
    (void)I2C1_SR2;                     /* ADDR cleared: SR1 then SR2 */
    for (uint32_t i = 0; i < n; ++i) {
        while (!(I2C1_SR1 & SR1_TXE)) {}
        I2C1_DR = data[i];
    }
    while (!(I2C1_SR1 & SR1_BTF)) {}
    if (!hold) {
        stop();
    }
    return 0u;
}

/* Read n >= 3 bytes after the address phase, NACK the last, STOP (RM0008
 * §26.3.3, "N > 2"): ACK is cleared with byte N-2 in DR and N-1 in the
 * shift register, STOP is set with N-1 in DR. */
static void read_n(volatile uint32_t *out, uint32_t n) {
    (void)I2C1_SR2;
    for (uint32_t i = 0; i + 3 < n; ++i) {
        while (!(I2C1_SR1 & SR1_RXNE)) {}
        out[i] = I2C1_DR;
    }
    while (!(I2C1_SR1 & SR1_BTF)) {}
    I2C1_CR1 &= ~CR1_ACK;
    out[n - 3] = I2C1_DR;
    while (!(I2C1_SR1 & SR1_BTF)) {}
    I2C1_CR1 |= CR1_STOP;
    out[n - 2] = I2C1_DR;
    while (!(I2C1_SR1 & SR1_RXNE)) {}
    out[n - 1] = I2C1_DR;
    while (I2C1_SR2 & 2u) {}
    I2C1_CR1 |= CR1_ACK;
}

int main(void) {
    i2c1_pins();
    i2c1_timing();
    I2C1_CR1 = CR1_PE | CR1_ACK;

    static const uint8_t first[3] = {0x01, 0xAB, 0xCD};
    uint32_t nacks = 0;
    while (write(0x09, first, 3, 0) != 0u && nacks < 1000u) {
        nacks += 1u;
        delay(200);
    }
    RESULT[5] = nacks;

#if PAST_END
    static const uint8_t second[3] = {0x02, 0x5A, 0x77};
#else
    static const uint8_t second[2] = {0x02, 0x5A};
#endif
    (void)write(0x09, second, sizeof second, 0);
    static const uint8_t led_on[2] = {0x00, 0x01};
    (void)write(0x09, led_on, 2, 0);

    static const uint8_t ptr[1] = {0x00};
    if (write(0x09, ptr, 1, 1) == 0u && address((0x09 << 1) | 1u) == 0u) {
        read_n(&RESULT[0], 4);
    } else {
        RESULT[0] = RESULT[1] = RESULT[2] = RESULT[3] = 0xEEu;
    }

    if (address((0x09 << 1) | 1u) == 0u) {
        /* One byte: NACK it, STOP right after ADDR is cleared. */
        I2C1_CR1 &= ~CR1_ACK;
        (void)I2C1_SR2;
        I2C1_CR1 |= CR1_STOP;
        while (!(I2C1_SR1 & SR1_RXNE)) {}
        RESULT[4] = I2C1_DR;
        while (I2C1_SR2 & 2u) {}
        I2C1_CR1 |= CR1_ACK;
    } else {
        RESULT[4] = 0xEEu;
    }

    static const uint8_t lost[1] = {0x55};
    RESULT[6] = write(0x50, lost, 1, 0);
    RESULT[7] = 1u;
    for (;;) {}
}

void reset(void) { (void)main(); for (;;) {} }
