/* ESP32-C3 (or -DCHIP_C6 ESP32-C6) I2C0 slave at 0x42 on gpio nets, register
 * level, polled: the 256-byte register file of i2c_target.c (the first byte
 * of a write sets the pointer, later bytes store, reads return from it,
 * auto-incrementing). GPIO8 SDA, GPIO9 SCL, open drain. A read finds the TX
 * FIFO empty: the slave stretches SCL (SLAVE_SCL_STRETCH_EN) with
 * SLAVE_STRETCH until the firmware has put the byte in and written
 * SLAVE_SCL_STRETCH_CLR.
 *
 * Built with -DSLOW=1 it waits about 15 us before every answer, longer than
 * an SCL low period, so the controller is held by the stretch.
 *
 * RESULT[0] transfers ended by STOP, RESULT[1..2] regs[0x10..0x11],
 * RESULT[3] bytes received, RESULT[4] bytes sent.
 */
#include "esp32c.h"

#ifndef SLOW
#define SLOW 0
#endif

static volatile uint8_t *const regs = (volatile uint8_t *)(&RESULT[16]);

int main(void) {
    enable_clocks();
    matrix_out(8, SIG_I2C_SDA, PAD_WPU);
    matrix_out(9, SIG_I2C_SCL, PAD_WPU);
    GPIO_PIN(8) = PAD_DRIVER;
    GPIO_PIN(9) = PAD_DRIVER;
    GPIO_OUT_W1TS = (1u << 8) | (1u << 9);
    GPIO_IN_SEL(SIG_I2C_SDA) = IN_SEL(8);
    GPIO_IN_SEL(SIG_I2C_SCL) = IN_SEL(9);

    I2C_CTR = I2C_FORCE_OUT | I2C_CLK_EN;      /* MS_MODE 0: slave */
    I2C_CLK_CONF = (1u << 21);
    I2C_SDA_HOLD = 39u;
    I2C_SLAVE_ADDR = 0x42u;
    I2C_FIFO_CONF = (1u << 14) | (1u << 13) | (1u << 12) | (8u << 5) | 1u;
    I2C_FIFO_CONF = (1u << 14) | (8u << 5) | 1u;
    I2C_STRETCH_CONF = STRETCH_EN;
    I2C_INT_CLR = 0xFFFFFFFFu;
    I2C_CTR = I2C_FORCE_OUT | I2C_CLK_EN | I2C_CONF_UPGATE;

    uint32_t ptr = 0u, have_ptr = 0u;
    for (;;) {
        uint32_t raw = I2C_INT_RAW;
        while ((I2C_SR >> 8) & 0x3Fu) {        /* RXFIFO_CNT */
            uint8_t b = (uint8_t)I2C_DATA;
            if (!have_ptr) {
                ptr = b;
                have_ptr = 1u;
            } else {
                regs[ptr & 0xFFu] = b;
                ptr += 1u;
            }
            RESULT[3] += 1u;
        }
        if (raw & INT_SLAVE_STRETCH) {
#if SLOW
            delay_us(15);
#endif
            I2C_INT_CLR = INT_SLAVE_STRETCH;
            I2C_DATA = regs[ptr & 0xFFu];
            ptr += 1u;
            RESULT[4] += 1u;
            I2C_STRETCH_CONF = STRETCH_EN | STRETCH_CLR;
        }
        if (raw & INT_TRANS_COMPLETE) {
            I2C_INT_CLR = INT_TRANS_COMPLETE | INT_RXFIFO_WM;
            /* Bytes that arrived before the STOP belong to this transfer. */
            while ((I2C_SR >> 8) & 0x3Fu) {
                uint8_t b = (uint8_t)I2C_DATA;
                if (!have_ptr) {
                    ptr = b;
                    have_ptr = 1u;
                } else {
                    regs[ptr & 0xFFu] = b;
                    ptr += 1u;
                }
                RESULT[3] += 1u;
            }
            have_ptr = 0u;
            RESULT[0] += 1u;
            RESULT[1] = regs[0x10];
            RESULT[2] = regs[0x11];
        }
    }
}
