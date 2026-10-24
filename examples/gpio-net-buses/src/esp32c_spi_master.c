/* ESP32-C3 (or -DCHIP_C6 ESP32-C6) GP-SPI2 master on gpio nets, register
 * level, the sequence esp-idf's spi_master and esp-hal program: full duplex
 * (DOUTDIN, USR_MOSI, USR_MISO), mode 0, CS0 framing the whole transfer with
 * CS_SETUP / CS_HOLD, SCK = 80 MHz / (16 * 50) = 100 kHz.
 *
 *   GPIO4 SCK   GPIO5 MOSI   GPIO3 MISO   GPIO1 CS0     (GPIO matrix)
 *
 * Sends A5 3C 5A C3 in one transaction and keeps what came back:
 * RESULT[0..3] received bytes, RESULT[4] SPI_DMA_INT_RAW after the
 * transaction (TRANS_DONE, bit 12), RESULT[7] = 1 done.
 */
#include "esp32c.h"

int main(void) {
    enable_clocks();
    matrix_out(4, SIG_FSPICLK, 0);
    matrix_out(5, SIG_FSPID, 0);
    matrix_out(1, SIG_FSPICS0, 0);
    matrix_in(3, SIG_FSPIQ);

    SPI_CLK_GATE = 7u;                         /* CLK_EN, MST_CLK_ACTIVE, MST_CLK_SEL */
    SPI_SLAVE = 0u;                            /* master */
    /* CLKDIV_PRE 15, CLKCNT_N 49, CLKCNT_H 24, CLKCNT_L 49 */
    SPI_CLOCK = (15u << 18) | (49u << 12) | (24u << 6) | 49u;
    SPI_USER = (1u << 0) | (1u << 6) | (1u << 7) | (1u << 27) | (1u << 28);
    SPI_USER1 = 0u;
    SPI_MISC = 0x3Eu;                          /* CS0 on, CS1..5 off, mode 0 */
    SPI_MS_DLEN = 31u;
    SPI_W(0) = 0xC35A3CA5u;                    /* A5 3C 5A C3, byte 0 first */
    SPI_CMD = SPI_UPDATE;
    while (SPI_CMD & SPI_UPDATE) {}

    delay_us(400);                             /* let the slave come up */
    SPI_INT_CLR = 0xFFFFFFFFu;
    SPI_CMD = SPI_USR;
    while (SPI_CMD & SPI_USR) {}
    uint32_t w = SPI_W(0);
    for (int i = 0; i < 4; ++i) {
        RESULT[i] = (w >> (8 * i)) & 0xFFu;
    }
    RESULT[4] = SPI_INT_RAW;
    RESULT[7] = 1u;
    for (;;) {}
}
