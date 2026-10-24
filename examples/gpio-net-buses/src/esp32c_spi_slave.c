/* ESP32-C3 (or -DCHIP_C6 ESP32-C6) GP-SPI2 slave on gpio nets, register
 * level, CPU-controlled (W buffer) as esp-idf's spi_slave does it without
 * DMA: SPI_SLAVE.MODE, mode 0 (CK_IDLE_EDGE 0, CLK_MODE_13 0), full duplex,
 * the answer preloaded in W0, USR set; the transaction ends when the master
 * lets CS go.
 *
 *   GPIO4 SCK   GPIO5 MOSI   GPIO3 MISO   GPIO1 CS0     (GPIO matrix)
 *
 * Answers 81 82 83 84. RESULT[0..3] bytes received, RESULT[4]
 * SPI_SLAVE1.DATA_BITLEN (bits received), RESULT[5] SPI_DMA_INT_RAW,
 * RESULT[7] = 1 done.
 */
#include "esp32c.h"

int main(void) {
    enable_clocks();
    matrix_in(4, SIG_FSPICLK);
    matrix_in(5, SIG_FSPID);
    matrix_in(1, SIG_FSPICS0);
    matrix_out(3, SIG_FSPIQ, 0);

    SPI_CLK_GATE = 7u;
    SPI_SLAVE = 1u << 26;                      /* slave, mode 0/2 timing */
    SPI_USER = (1u << 0) | (1u << 27) | (1u << 28);
    SPI_MISC = 0x3Eu;                          /* CK_IDLE_EDGE 0 */
    SPI_MS_DLEN = 31u;
    SPI_W(0) = 0x84838281u;
    SPI_INT_CLR = 0xFFFFFFFFu;
    SPI_CMD = SPI_UPDATE;
    while (SPI_CMD & SPI_UPDATE) {}
    SPI_CMD = SPI_USR;

    while (!(SPI_INT_RAW & SPI_TRANS_DONE)) {}
    uint32_t w = SPI_W(0);
    for (int i = 0; i < 4; ++i) {
        RESULT[i] = (w >> (8 * i)) & 0xFFu;
    }
    RESULT[4] = SPI_SLAVE1 & 0x3FFFFu;
    RESULT[5] = SPI_INT_RAW;
    RESULT[7] = 1u;
    for (;;) {}
}
