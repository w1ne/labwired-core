/* ESP32-S3 GP-SPI2 (INST1: GP-SPI3; or --defsym CHIP_ESP32=1: classic ESP32 HSPI) slave on
 * gpio nets, register level, CPU-controlled (W buffer) as esp-idf's
 * spi_slave does it without DMA: slave mode, mode 0, full duplex, the answer
 * preloaded in W0, USR set; the transaction ends when the master lets CS
 * go. The sequence of esp32c_spi_slave.c.
 *
 * Answers 81 82 83 84. RESULT[0..3] bytes received, RESULT[4] bits
 * received (S3 SPI_SLAVE1.DATA_BITLEN, classic SPI_SLV_RD_BIT), RESULT[5]
 * the transaction-done flag word (S3 SPI_DMA_INT_RAW, classic SPI_SLAVE),
 * RESULT[7] = 1 done.
 */
    /* The pads of the slave role (classic PADS_HI: input-only GPIO34..36
     * take the master's lines in). */
    .set SPI_ROLE_SLAVE, 1
    .include "esp32x.inc"

    .section .text,"ax"
    .global _start
_start:
    LI a14, RESULT
    ENABLE_CLOCKS
    MATRIX_IN PAD_SCK, IOMUX_SCK, SIG_S_CLK
    MATRIX_IN PAD_MOSI, IOMUX_MOSI, SIG_S_D
    MATRIX_IN PAD_CS, IOMUX_CS, SIG_S_CS0
    MATRIX_OUT PAD_MISO, IOMUX_MISO, SIG_S_Q, 0

.ifdef CHIP_ESP32
    /* Classic: SLAVE 0x38 SLAVE_MODE (30); USER 0x1C DOUTDIN, CK_I_EDGE (6,
     * mode 0 per esp-idf spi_ll_slave_set_mode), USR_MOSI, USR_MISO; PIN
     * 0x34 CK_IDLE_EDGE 0. */
    WR SPI_SLAVE + 0x38, 1 << 30
    WR SPI_SLAVE + 0x1C, (1 << 0) | (1 << 6) | (1 << 27) | (1 << 28)
    WR SPI_SLAVE + 0x34, 0
    WR SPI_SLAVE + 0x28, 31
    WR SPI_SLAVE + 0x2C, 31
    WR SPI_SLAVE + 0x80, 0x84838281
    WR SPI_SLAVE + 0x00, 1 << 18
.Lwait:
    RD a6, SPI_SLAVE + 0x38
    bbci a6, 4, .Lwait
    RD a6, SPI_SLAVE + 0x80
    RD a7, SPI_SLAVE + 0x64
    LI a3, 0xFFFFFF
    and a7, a7, a3
    RD a8, SPI_SLAVE + 0x38
.else
    WR SPI_SLAVE + 0xE8, 7
    /* slave, mode 0/2 timing */
    WR SPI_SLAVE + 0xE0, 1 << 26
    WR SPI_SLAVE + 0x10, (1 << 0) | (1 << 27) | (1 << 28)
    /* CK_IDLE_EDGE 0 */
    WR SPI_SLAVE + 0x20, 0x3E
    WR SPI_SLAVE + 0x1C, 31
    WR SPI_SLAVE + 0x98, 0x84838281
    WR SPI_SLAVE + 0x38, 0xFFFFFFFF
    WR SPI_SLAVE + 0x00, 1 << 23
.Lupdate:
    RD a6, SPI_SLAVE + 0x00
    bbsi a6, 23, .Lupdate
    WR SPI_SLAVE + 0x00, 1 << 24
.Lwait:
    RD a6, SPI_SLAVE + 0x3C
    bbci a6, 12, .Lwait
    RD a6, SPI_SLAVE + 0x98
    RD a7, SPI_SLAVE + 0xE4
    LI a3, 0x3FFFF
    and a7, a7, a3
    RD a8, SPI_SLAVE + 0x3C
.endif
    extui a5, a6, 0, 8
    s32i a5, a14, 0
    extui a5, a6, 8, 8
    s32i a5, a14, 4
    extui a5, a6, 16, 8
    s32i a5, a14, 8
    extui a5, a6, 24, 8
    s32i a5, a14, 12
    s32i a7, a14, 16
    s32i a8, a14, 20
    movi a5, 1
    s32i a5, a14, 28
.Lhalt:
    j .Lhalt
