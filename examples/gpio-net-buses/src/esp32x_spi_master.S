/* ESP32-S3 GP-SPI2 (INST1: GP-SPI3; or --defsym CHIP_ESP32=1: classic ESP32 VSPI) master on
 * gpio nets, register level: full duplex (DOUTDIN, USR_MOSI, USR_MISO),
 * mode 0, CS0 framing the whole transfer with CS_SETUP / CS_HOLD, SCK =
 * 80 MHz / (16 * 50) = 100 kHz. The sequence of esp32c_spi_master.c.
 *
 * Sends A5 3C 5A C3 in one transaction and keeps what came back:
 * RESULT[0..3] received bytes, RESULT[4] the transaction-done flag word
 * (S3 SPI_DMA_INT_RAW, TRANS_DONE bit 12; classic SPI_SLAVE, TRANS_DONE
 * bit 4), RESULT[7] = 1 done.
 */
    .include "esp32x.inc"

    .section .text,"ax"
    .global _start
_start:
    LI a14, RESULT
    ENABLE_CLOCKS
    MATRIX_OUT PAD_SCK, IOMUX_SCK, SIG_M_CLK, 0
    MATRIX_OUT PAD_MOSI, IOMUX_MOSI, SIG_M_D, 0
    MATRIX_OUT PAD_CS, IOMUX_CS, SIG_M_CS0, 0
    MATRIX_IN PAD_MISO, IOMUX_MISO, SIG_M_Q

.ifdef CHIP_ESP32
    /* Classic SPI (spi_reg.h): SLAVE 0x38 (master, TRANS_DONE clear),
     * CLOCK 0x18, USER 0x1C (DOUTDIN 0, CS_HOLD 4, CS_SETUP 5, USR_MOSI 27,
     * USR_MISO 28), USER1 0x20, CTRL2 0x14 (no extra setup / hold), PIN 0x34
     * (CS0 on, CS1/CS2 off, mode 0), MOSI_DLEN 0x28 / MISO_DLEN 0x2C. */
    WR SPI_MASTER + 0x38, 0
    WR SPI_MASTER + 0x18, (15 << 18) | (49 << 12) | (24 << 6) | 49
    WR SPI_MASTER + 0x1C, (1 << 0) | (1 << 4) | (1 << 5) | (1 << 27) | (1 << 28)
    WR SPI_MASTER + 0x20, 0
    WR SPI_MASTER + 0x14, 0
    WR SPI_MASTER + 0x34, 0x6
    WR SPI_MASTER + 0x28, 31
    WR SPI_MASTER + 0x2C, 31
    /* A5 3C 5A C3, byte 0 first */
    WR SPI_MASTER + 0x80, 0xC35A3CA5
    /* let the slave come up */
    DELAY_US 400
    /* SPI_CMD.USR */
    WR SPI_MASTER + 0x00, 1 << 18
.Lbusy:
    RD a6, SPI_MASTER + 0x00
    bbsi a6, 18, .Lbusy
    RD a6, SPI_MASTER + 0x80
    RD a7, SPI_MASTER + 0x38
.else
    /* GP-SPI2 (the C3's register file): CLK_GATE 0xE8, SLAVE 0xE0, CLOCK
     * 0x0C, USER 0x10 (DOUTDIN 0, CS_HOLD 6, CS_SETUP 7, USR_MOSI 27,
     * USR_MISO 28), USER1 0x14, MISC 0x20, MS_DLEN 0x1C, W0 0x98, CMD
     * UPDATE 23 / USR 24, DMA_INT_CLR 0x38 / DMA_INT_RAW 0x3C. */
    WR SPI_MASTER + 0xE8, 7
    WR SPI_MASTER + 0xE0, 0
    WR SPI_MASTER + 0x0C, (15 << 18) | (49 << 12) | (24 << 6) | 49
    WR SPI_MASTER + 0x10, (1 << 0) | (1 << 6) | (1 << 7) | (1 << 27) | (1 << 28)
    WR SPI_MASTER + 0x14, 0
    WR SPI_MASTER + 0x20, 0x3E
    WR SPI_MASTER + 0x1C, 31
    WR SPI_MASTER + 0x98, 0xC35A3CA5
    WR SPI_MASTER + 0x00, 1 << 23
.Lupdate:
    RD a6, SPI_MASTER + 0x00
    bbsi a6, 23, .Lupdate
    DELAY_US 400
    WR SPI_MASTER + 0x38, 0xFFFFFFFF
    WR SPI_MASTER + 0x00, 1 << 24
.Lbusy:
    RD a6, SPI_MASTER + 0x00
    bbsi a6, 24, .Lbusy
    RD a6, SPI_MASTER + 0x98
    RD a7, SPI_MASTER + 0x3C
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
    movi a5, 1
    s32i a5, a14, 28
.Lhalt:
    j .Lhalt
