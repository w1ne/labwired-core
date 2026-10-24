/* ESP32-S3 (or --defsym CHIP_ESP32=1: classic ESP32) I2C0 (INST1: I2C1) slave at 0x42 on
 * gpio nets, register level, polled: the 256-byte register file of
 * i2c_target.c (the first byte of a write sets the pointer, later bytes
 * store, reads return from it, auto-incrementing). Open drain.
 *
 * S3: a read finds the TX FIFO empty, so the slave stretches SCL
 * (SLAVE_SCL_STRETCH_EN) with SLAVE_STRETCH until the firmware has put the
 * byte in and written SLAVE_SCL_STRETCH_CLR, as esp32c_i2c_target.c does.
 * Built with --defsym SLOW=1 it waits about 15 us before every answer,
 * longer than an SCL low period, so the controller is held by the stretch.
 *
 * Classic ESP32: the part has no SCL_STRETCH_CONF, and a slave whose TX
 * FIFO is empty sends 0xFF. So the firmware keeps the TX FIFO loaded with
 * the next four register bytes: every byte that moves the pointer resets the
 * TX FIFO and refills it from the new pointer, well before a repeated START
 * and the address byte have gone by.
 *
 * RESULT[0] transfers ended by STOP, RESULT[1..2] regs[0x10..0x11],
 * RESULT[3] bytes received, RESULT[4] bytes sent. regs live at RESULT[16..].
 *
 * a6 pointer, a7 pointer set, a9 regs, a10 bytes queued in the TX FIFO
 * (classic).
 */
    .include "esp32x.inc"

.ifndef SLOW
    .set SLOW, 0
.endif

    .section .text,"ax"
    .global _start
_start:
    LI a14, RESULT
    addi a9, a14, 64
    movi a6, 0
    movi a7, 0
    movi a10, 0
    ENABLE_CLOCKS
    MATRIX_OUT PAD_SDA, IOMUX_SDA, SIG_I2C_SDA, PAD_WPU
    MATRIX_OUT PAD_SCL, IOMUX_SCL, SIG_I2C_SCL, PAD_WPU
    WR GPIO_BASE + GPIO_PIN0 + 4 * PAD_SDA, PAD_DRIVER
    WR GPIO_BASE + GPIO_PIN0 + 4 * PAD_SCL, PAD_DRIVER
    OUT_HIGH2 PAD_SDA, PAD_SCL
    WR GPIO_BASE + GPIO_IN_SEL0 + 4 * SIG_I2C_SDA, IN_SEL | PAD_SDA
    WR GPIO_BASE + GPIO_IN_SEL0 + 4 * SIG_I2C_SCL, IN_SEL | PAD_SCL

    /* MS_MODE 0: slave */
    WR I2C_CTR, I2C_FORCE_OUT | I2C_CLK_EN
.ifdef CHIP_ESP32
    WR I2C_SDA_HOLD, 80
.else
    WR I2C_CLK_CONF, 1 << 21
    WR I2C_SDA_HOLD, 39
.endif
    WR I2C_SLAVE_ADDR, 0x42
    WR I2C_FIFO_CONF, FIFO_RESET
    WR I2C_FIFO_CONF, FIFO_RUN
.ifndef CHIP_ESP32
    WR I2C_STRETCH_CONF, STRETCH_EN
.endif
    WR I2C_INT_CLR, 0xFFFFFFFF
    WR I2C_CTR, I2C_FORCE_OUT | I2C_CLK_EN | I2C_CONF_UPGATE

.Lloop:
    RD a8, I2C_INT_RAW
    call0 drain_rx
.ifndef CHIP_ESP32
    bbsi a8, 16, .Lstretched
    j .Lno_stretch
.Lstretched:
.if SLOW
    DELAY_US 15
.endif
    WR I2C_INT_CLR, INT_SLAVE_STRETCH
    extui a4, a6, 0, 8
    add a4, a4, a9
    l8ui a5, a4, 0
    WRR I2C_DATA, a5
    addi a6, a6, 1
    INC 4
    WR I2C_STRETCH_CONF, STRETCH_EN | STRETCH_CLR
.Lno_stretch:
.endif
    bbsi a8, 7, .Lcomplete
    j .Lloop
.Lcomplete:
    WR I2C_INT_CLR, INT_TRANS_COMPLETE | INT_RXFIFO_WM
    /* Bytes that arrived before the STOP belong to this transfer. */
    call0 drain_rx
.ifdef CHIP_ESP32
    /* Sent = queued - still in the TX FIFO (SR.TXFIFO_CNT [23:18]). */
    RD a4, I2C_SR
    extui a4, a4, 18, 6
    sub a5, a10, a4
    l32i a4, a14, 16
    add a4, a4, a5
    s32i a4, a14, 16
    movi a10, 0
    WR I2C_FIFO_CONF, FIFO_TX_RESET
    WR I2C_FIFO_CONF, FIFO_RUN
.endif
    movi a7, 0
    INC 0
    l8ui a4, a9, 0x10
    s32i a4, a14, 4
    l8ui a4, a9, 0x11
    s32i a4, a14, 8
    j .Lloop

/* Take every byte in the RX FIFO: the first of a transfer is the pointer,
 * the rest are stored. */
    .balign 4
drain_rx:
    RD a4, I2C_SR
    extui a4, a4, 8, 6
    beqz a4, .Ldrain_done
    RD a5, I2C_DATA
    extui a5, a5, 0, 8
    bnez a7, .Ldrain_store
    or a6, a5, a5
    movi a7, 1
    j .Ldrain_count
.Ldrain_store:
    extui a4, a6, 0, 8
    add a4, a4, a9
    s8i a5, a4, 0
    addi a6, a6, 1
.Ldrain_count:
    INC 3
.ifdef CHIP_ESP32
    /* The pointer moved: the TX FIFO holds regs[ptr..ptr+3] from now on. */
    WR I2C_FIFO_CONF, FIFO_TX_RESET
    WR I2C_FIFO_CONF, FIFO_RUN
    movi a10, 0
.Lrefill:
    add a4, a6, a10
    extui a4, a4, 0, 8
    add a4, a4, a9
    l8ui a5, a4, 0
    WRR I2C_DATA, a5
    addi a10, a10, 1
    bnei a10, 4, .Lrefill
.endif
    j drain_rx
.Ldrain_done:
    ret
