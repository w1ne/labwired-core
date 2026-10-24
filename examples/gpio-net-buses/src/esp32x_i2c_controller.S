/* ESP32-S3 (or --defsym CHIP_ESP32=1: classic ESP32) I2C0 (INST1: I2C1) master on gpio
 * nets, register level: the command lists esp-idf and esp-hal build (the
 * address is the first byte of the TX FIFO, ACK checked on every written
 * byte, the last byte read is NACKed), open drain, the pull-up is the net's.
 * SCL 100 kHz: S3 XTAL 40 MHz undivided, SCL low and high 200 ticks;
 * classic APB 80 MHz, 400 cycles each. The sequence of
 * esp32c_i2c_controller.c, against target 0x42:
 *
 *   1. write 0x10 0xDE 0xAD
 *   2. write 0x10, repeated START, read two bytes
 *   3. write one byte to 0x50, where nobody answers: NACK
 *
 * RESULT[0..1] the two bytes read, RESULT[2] NACK in step 1 (0),
 * RESULT[3] NACK in step 3 (1), RESULT[4] transfers ended by
 * TRANS_COMPLETE, RESULT[7] = 1 done.
 */
    .include "esp32x.inc"

    .set CTR_RUN, I2C_MS_MODE | I2C_FORCE_OUT | I2C_CLK_EN

    .section .text,"ax"
    .global _start
_start:
    LI a14, RESULT
    ENABLE_CLOCKS
    MATRIX_OUT PAD_SDA, IOMUX_SDA, SIG_I2C_SDA, PAD_WPU
    MATRIX_OUT PAD_SCL, IOMUX_SCL, SIG_I2C_SCL, PAD_WPU
    WR GPIO_BASE + GPIO_PIN0 + 4 * PAD_SDA, PAD_DRIVER
    WR GPIO_BASE + GPIO_PIN0 + 4 * PAD_SCL, PAD_DRIVER
    OUT_HIGH2 PAD_SDA, PAD_SCL
    /* The controller reads SDA and SCL back through the input matrix. */
    WR GPIO_BASE + GPIO_IN_SEL0 + 4 * SIG_I2C_SDA, IN_SEL | PAD_SDA
    WR GPIO_BASE + GPIO_IN_SEL0 + 4 * SIG_I2C_SCL, IN_SEL | PAD_SCL

    WR I2C_CTR, CTR_RUN
.ifdef CHIP_ESP32
    WR I2C_SCL_LOW, 400
    WR I2C_SCL_HIGH, 400
    WR I2C_SDA_HOLD, 80
    WR I2C_SDA_SAMPLE, 80
    WR I2C_START_HOLD, 400
    WR I2C_RSTART_SETUP, 400
    WR I2C_STOP_HOLD, 400
    WR I2C_STOP_SETUP, 400
.else
    /* SCLK_ACTIVE, XTAL, /1 */
    WR I2C_CLK_CONF, 1 << 21
    WR I2C_SCL_LOW, 199
    WR I2C_SCL_HIGH, 199
    WR I2C_SDA_HOLD, 39
    WR I2C_SDA_SAMPLE, 39
    WR I2C_START_HOLD, 199
    WR I2C_RSTART_SETUP, 199
    WR I2C_STOP_HOLD, 199
    WR I2C_STOP_SETUP, 199
    WR I2C_CTR, CTR_RUN | I2C_CONF_UPGATE
.endif
    /* let the target come up */
    DELAY_US 400

    /* 1. write 0x10 0xDE 0xAD to 0x42 */
    WR I2C_FIFO_CONF, FIFO_RESET
    WR I2C_FIFO_CONF, FIFO_RUN
    WR I2C_DATA, 0x42 << 1
    WR I2C_DATA, 0x10
    WR I2C_DATA, 0xDE
    WR I2C_DATA, 0xAD
    WR I2C_COMD0 + 0, OP_RSTART << 11
    WR I2C_COMD0 + 4, (OP_WRITE << 11) | 4 | ACK_CHECK
    WR I2C_COMD0 + 8, OP_STOP << 11
    call0 run
    s32i a6, a14, 8

    /* 2. write 0x10, repeated START, read two bytes */
    WR I2C_FIFO_CONF, FIFO_RESET
    WR I2C_FIFO_CONF, FIFO_RUN
    WR I2C_DATA, 0x42 << 1
    WR I2C_DATA, 0x10
    WR I2C_DATA, (0x42 << 1) | 1
    WR I2C_COMD0 + 0, OP_RSTART << 11
    WR I2C_COMD0 + 4, (OP_WRITE << 11) | 2 | ACK_CHECK
    WR I2C_COMD0 + 8, OP_RSTART << 11
    WR I2C_COMD0 + 12, (OP_WRITE << 11) | 1 | ACK_CHECK
    WR I2C_COMD0 + 16, (OP_READ << 11) | 1
    WR I2C_COMD0 + 20, (OP_READ << 11) | 1 | NACK_LAST
    WR I2C_COMD0 + 24, OP_STOP << 11
    call0 run
    movi a7, 0xEE
    movi a8, 0xEE
    bnez a6, .Lstore_read
    RD a7, I2C_DATA
    extui a7, a7, 0, 8
    RD a8, I2C_DATA
    extui a8, a8, 0, 8
.Lstore_read:
    s32i a7, a14, 0
    s32i a8, a14, 4

    /* 3. one byte to 0x50, where nobody answers */
    WR I2C_FIFO_CONF, FIFO_RESET
    WR I2C_FIFO_CONF, FIFO_RUN
    WR I2C_DATA, 0x50 << 1
    WR I2C_DATA, 0x55
    WR I2C_COMD0 + 0, OP_RSTART << 11
    WR I2C_COMD0 + 4, (OP_WRITE << 11) | 2 | ACK_CHECK
    WR I2C_COMD0 + 8, OP_STOP << 11
    call0 run
    s32i a6, a14, 12
    movi a5, 1
    s32i a5, a14, 28
.Lhalt:
    j .Lhalt

/* Run the command list; a6 = 1 when the transfer was NACKed. */
    .balign 4
run:
    WR I2C_INT_CLR, 0xFFFFFFFF
    WR I2C_CTR, CTR_RUN | I2C_TRANS_START
.Lrun_wait:
    RD a8, I2C_INT_RAW
    LI a3, INT_TRANS_COMPLETE | INT_NACK
    and a4, a8, a3
    beqz a4, .Lrun_wait
    bbci a8, 10, .Lrun_ok
    /* Let the STOP that frees the bus go out (SR.BUS_BUSY, bit 4). */
.Lrun_busy:
    RD a4, I2C_SR
    bbsi a4, 4, .Lrun_busy
    movi a6, 1
    ret
.Lrun_ok:
    INC 4
    movi a6, 0
    ret
