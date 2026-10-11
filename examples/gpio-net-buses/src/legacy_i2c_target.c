/* STM32F103 / STM32F401 legacy I2C1 slave at 0x42 on gpio nets, interrupt
 * driven (ITEVTEN, ITBUFEN, ITERREN): the same 256-byte register file as
 * i2c_target.c, preset to regs[i] = 3i + 1. The first byte of a write sets
 * the pointer, later bytes store, reads return from it, auto-incrementing.
 * Every TXE loads the next byte into DR, so the byte loaded when the master
 * NACKs stays in DR and is the first one the next read gets. SCL is stretched while
 * ADDR is up and while the handler has not caught up.
 *
 * Built with -DSLOW=1 its handler waits about 10 us before clearing ADDR,
 * longer than an SCL low period, so the master must be held by the stretch.
 *
 * RESULT[0] write transfers ended by STOP (STOPF), RESULT[1..2]
 * regs[0x10..0x11], RESULT[3] bytes received, RESULT[4] bytes loaded into
 * DR, RESULT[5] reads ended by the master's NACK (AF).
 */
#include "legacy_i2c.h"

#ifndef SLOW
#define SLOW 0
#endif

void i2c1_ev_handler(void);
void i2c1_er_handler(void);

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 64] = {
    [0] = STACK_TOP,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
    [16 + 31] = (uintptr_t)i2c1_ev_handler,
    [16 + 32] = (uintptr_t)i2c1_er_handler,
};

static uint8_t regs[256];
static uint8_t ptr;
static uint32_t have_ptr;

void i2c1_ev_handler(void) {
    uint32_t sr1 = I2C1_SR1;
    if (sr1 & SR1_ADDR) {
#if SLOW
        delay(150);
#endif
        uint32_t sr2 = I2C1_SR2;        /* SR1 then SR2: ADDR cleared */
        if (!(sr2 & SR2_TRA)) {
            have_ptr = 0u;
        }
    }
    if (sr1 & SR1_RXNE) {
        uint8_t b = (uint8_t)I2C1_DR;
        if (!have_ptr) {
            ptr = b;
            have_ptr = 1u;
        } else {
            regs[ptr++] = b;
        }
        RESULT[3] += 1u;
    }
    if ((sr1 & SR1_TXE) && (I2C1_SR2 & SR2_TRA)) {
        I2C1_DR = regs[ptr++];
        RESULT[4] += 1u;
    }
    if (sr1 & SR1_STOPF) {
        I2C1_CR1 = I2C1_CR1;            /* SR1 read, CR1 write: STOPF cleared */
        RESULT[0] += 1u;
        RESULT[1] = regs[0x10];
        RESULT[2] = regs[0x11];
    }
}

void i2c1_er_handler(void) {
    if (I2C1_SR1 & SR1_AF) {
        I2C1_SR1 = ~SR1_AF;             /* the master's NACK ends a read */
        RESULT[5] += 1u;
    }
}

int main(void) {
    for (unsigned i = 0; i < 256u; ++i) {
        regs[i] = (uint8_t)(i * 3u + 1u);
    }
    i2c1_pins();
    i2c1_timing();
    I2C1_OAR1 = (1u << 14) | (0x42u << 1);
    I2C1_CR2 |= (1u << 8) | (1u << 9) | (1u << 10);  /* ITERREN ITEVTEN ITBUFEN */
    I2C1_CR1 = CR1_PE | CR1_ACK;
    NVIC_ISER0 = 1u << 31;
    NVIC_ISER1 = 1u << (32 - 32);
    __asm__ volatile("cpsie i" ::: "memory");
    for (;;) {
        __asm__ volatile("wfi");
    }
}

void reset(void) { (void)main(); for (;;) {} }
