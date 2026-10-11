/* STM32F103 / STM32F401 legacy I2C1 controller on gpio nets, polled, in the
 * RM0008 / RM0368 master sequences. Target 0x42 is a register file (see
 * legacy_i2c_target.c and i2c_target.c).
 *
 *   1. write 0x10 0xDE 0xAD to 0x42, STOP
 *   2. write 0x10, repeated START, read 2 bytes (POS/ACK two-byte
 *      sequence), STOP
 *   3. read 1 byte from 0x42 (ACK cleared before ADDR), STOP
 *   4. write one byte to 0x50, where nobody answers: AF, STOP
 *
 * RESULT[0..1] the bytes read in step 2 (0xEE when it was NACKed),
 * RESULT[2] AF in step 1 (0), RESULT[3] AF in step 4 (1), RESULT[4]
 * transfers ended by STOP, RESULT[5] the byte read in step 3, RESULT[7] = 1
 * done.
 */
#include "legacy_i2c.h"

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 64] = {
    [0] = STACK_TOP,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
};

static uint32_t stops;

static void stop(void) {
    I2C1_CR1 |= CR1_STOP;
    while (I2C1_SR2 & 2u) {}            /* BUSY until STOP is on the wire */
    stops += 1u;
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

int main(void) {
    i2c1_pins();
    i2c1_timing();
    I2C1_CR1 = CR1_PE | CR1_ACK;

    delay(2000);                        /* let the target come up */
    static const uint8_t first[3] = {0x10, 0xDE, 0xAD};
    RESULT[2] = write(0x42, first, 3, 0);

    static const uint8_t ptr[1] = {0x10};
    if (write(0x42, ptr, 1, 1) == 0u && address((0x42 << 1) | 1u) == 0u) {
        /* Two bytes: NACK the second (POS), STOP once both are in. */
        I2C1_CR1 = (I2C1_CR1 & ~CR1_ACK) | CR1_POS;
        (void)I2C1_SR2;
        while (!(I2C1_SR1 & SR1_BTF)) {}
        I2C1_CR1 |= CR1_STOP;
        RESULT[0] = I2C1_DR;
        RESULT[1] = I2C1_DR;
        while (I2C1_SR2 & 2u) {}
        stops += 1u;
        I2C1_CR1 = (I2C1_CR1 & ~CR1_POS) | CR1_ACK;
    } else {
        RESULT[0] = RESULT[1] = 0xEEu;
    }

    if (address((0x42 << 1) | 1u) == 0u) {
        /* One byte: NACK it, STOP right after ADDR is cleared. */
        I2C1_CR1 &= ~CR1_ACK;
        (void)I2C1_SR2;
        I2C1_CR1 |= CR1_STOP;
        while (!(I2C1_SR1 & SR1_RXNE)) {}
        RESULT[5] = I2C1_DR;
        while (I2C1_SR2 & 2u) {}
        stops += 1u;
        I2C1_CR1 |= CR1_ACK;
    } else {
        RESULT[5] = 0xEEu;
    }

    static const uint8_t lost[1] = {0x55};
    RESULT[3] = write(0x50, lost, 1, 0);
    RESULT[4] = stops;
    RESULT[7] = 1u;
    for (;;) {}
}

void reset(void) { (void)main(); for (;;) {} }
