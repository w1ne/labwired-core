/* ESP32-C3 (or -DCHIP_C6 ESP32-C6) GP-SPI2 slave on gpio nets, full duplex
 * by GDMA, register level, as esp-idf's spi_slave does it with DMA: the
 * answer streams from an OUT descriptor, what the master sends lands through
 * an IN descriptor (spi_dma_start in esp32c.h), mode 0, USR set; the
 * transaction ends when the master lets CS go.
 *
 *   GPIO4 SCK   GPIO5 MOSI   GPIO3 MISO   GPIO1 CS0     (GPIO matrix)
 *
 * Answers 81 82 .. 88. RESULT[0..7] the bytes received, RESULT[8]
 * SPI_SLAVE1.DATA_BITLEN, RESULT[9] SPI_DMA_INT_RAW, RESULT[10] the IN
 * descriptor's dw0 after the transfer, RESULT[15] = 1 done.
 */
#include "esp32c.h"

#define N 8u

int main(void) {
    enable_clocks();
    matrix_in(4, SIG_FSPICLK);
    matrix_in(5, SIG_FSPID);
    matrix_in(1, SIG_FSPICS0);
    matrix_out(3, SIG_FSPIQ, 0);

    SPI_CLK_GATE = 7u;
    SPI_SLAVE = 1u << 26;                      /* slave, mode 0 */
    SPI_USER = (1u << 0) | (1u << 27) | (1u << 28);
    SPI_MISC = 0x3Eu;
    SPI_MS_DLEN = 8u * N - 1u;
    REG(TX_BUF) = 0x84838281u;
    REG(TX_BUF + 4u) = 0x88878685u;
    REG(RX_BUF) = 0u;
    REG(RX_BUF + 4u) = 0u;
    spi_dma_start(N);
    SPI_INT_CLR = 0xFFFFFFFFu;
    SPI_CMD = SPI_UPDATE;
    while (SPI_CMD & SPI_UPDATE) {}
    SPI_CMD = SPI_USR;

    while (!(SPI_INT_RAW & SPI_TRANS_DONE)) {}
    for (unsigned i = 0; i < N; ++i) {
        RESULT[i] = *(volatile uint8_t *)(RX_BUF + i);
    }
    RESULT[8] = SPI_SLAVE1 & 0x3FFFFu;
    RESULT[9] = SPI_INT_RAW;
    RESULT[10] = REG(IN_DESC);
    RESULT[15] = 1u;
    for (;;) {}
}
