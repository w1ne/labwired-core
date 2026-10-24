/* ESP32-C3 (or -DCHIP_C6 ESP32-C6) I2C0 master on gpio nets, register level:
 * the command lists esp-idf and esp-hal build (the address is the first byte
 * of the TX FIFO, ACK checked on every written byte, the last byte read is
 * NACKed). GPIO8 SDA, GPIO9 SCL, open drain; the pull-up is the net's.
 * SCL 100 kHz: XTAL 40 MHz undivided, SCL low and high 200 ticks, SDA hold
 * and sample 40 ticks. The sequence of i2c_controller.c, against target 0x42:
 *
 *   1. write 0x10 0xDE 0xAD
 *   2. write 0x10, repeated START, read two bytes
 *   3. write one byte to 0x50, where nobody answers: NACK
 *
 * RESULT[0..1] the two bytes read, RESULT[2] NACK in step 1 (0),
 * RESULT[3] NACK in step 3 (1), RESULT[4] transfers ended by
 * TRANS_COMPLETE, RESULT[7] = 1 done.
 */
#include "esp32c.h"

/* Run the command list; returns 1 when the transfer was NACKed. */
static uint32_t run(void) {
    I2C_INT_CLR = 0xFFFFFFFFu;
    I2C_CTR = I2C_MS_MODE | I2C_FORCE_OUT | I2C_CLK_EN | I2C_TRANS_START;
    uint32_t raw;
    do {
        raw = I2C_INT_RAW;
    } while (!(raw & (INT_TRANS_COMPLETE | INT_NACK)));
    if (raw & INT_NACK) {
        /* Let the STOP that frees the bus go out before the next transfer. */
        while (I2C_SR & (1u << 4)) {}
        return 1u;
    }
    RESULT[4] += 1u;
    return 0u;
}

static void fifo_reset(void) {
    I2C_FIFO_CONF = (1u << 14) | (1u << 13) | (1u << 12) | (8u << 5) | 1u;
    I2C_FIFO_CONF = (1u << 14) | (8u << 5) | 1u;
}

int main(void) {
    enable_clocks();
    matrix_out(8, SIG_I2C_SDA, PAD_WPU);
    matrix_out(9, SIG_I2C_SCL, PAD_WPU);
    GPIO_PIN(8) = PAD_DRIVER;
    GPIO_PIN(9) = PAD_DRIVER;
    GPIO_OUT_W1TS = (1u << 8) | (1u << 9);
    /* The controller reads SDA and SCL back through the input matrix. */
    GPIO_IN_SEL(SIG_I2C_SDA) = IN_SEL(8);
    GPIO_IN_SEL(SIG_I2C_SCL) = IN_SEL(9);

    I2C_CTR = I2C_MS_MODE | I2C_FORCE_OUT | I2C_CLK_EN;
    I2C_CLK_CONF = (1u << 21);                 /* SCLK_ACTIVE, XTAL, /1 */
    I2C_SCL_LOW = 199u;
    I2C_SCL_HIGH = 199u;
    I2C_SDA_HOLD = 39u;
    I2C_SDA_SAMPLE = 39u;
    I2C_START_HOLD = 199u;
    I2C_RSTART_SETUP = 199u;
    I2C_STOP_HOLD = 199u;
    I2C_STOP_SETUP = 199u;
    I2C_CTR = I2C_MS_MODE | I2C_FORCE_OUT | I2C_CLK_EN | I2C_CONF_UPGATE;

    delay_us(400);                             /* let the target come up */

    fifo_reset();
    I2C_DATA = 0x42u << 1;
    I2C_DATA = 0x10u;
    I2C_DATA = 0xDEu;
    I2C_DATA = 0xADu;
    I2C_COMD(0) = CMD(OP_RSTART, 0, 0);
    I2C_COMD(1) = CMD(OP_WRITE, 4, ACK_CHECK);
    I2C_COMD(2) = CMD(OP_STOP, 0, 0);
    RESULT[2] = run();

    fifo_reset();
    I2C_DATA = 0x42u << 1;
    I2C_DATA = 0x10u;
    I2C_DATA = (0x42u << 1) | 1u;
    I2C_COMD(0) = CMD(OP_RSTART, 0, 0);
    I2C_COMD(1) = CMD(OP_WRITE, 2, ACK_CHECK);
    I2C_COMD(2) = CMD(OP_RSTART, 0, 0);
    I2C_COMD(3) = CMD(OP_WRITE, 1, ACK_CHECK);
    I2C_COMD(4) = CMD(OP_READ, 1, 0);
    I2C_COMD(5) = CMD(OP_READ, 1, NACK_LAST);
    I2C_COMD(6) = CMD(OP_STOP, 0, 0);
    if (run() == 0u) {
        RESULT[0] = I2C_DATA & 0xFFu;
        RESULT[1] = I2C_DATA & 0xFFu;
    } else {
        RESULT[0] = RESULT[1] = 0xEEu;
    }

    fifo_reset();
    I2C_DATA = 0x50u << 1;
    I2C_DATA = 0x55u;
    I2C_COMD(0) = CMD(OP_RSTART, 0, 0);
    I2C_COMD(1) = CMD(OP_WRITE, 2, ACK_CHECK);
    I2C_COMD(2) = CMD(OP_STOP, 0, 0);
    RESULT[3] = run();
    RESULT[7] = 1u;
    for (;;) {}
}
