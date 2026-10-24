/* ESP32-C3 (or -DCHIP_C6 ESP32-C6) GP-SPI2 master on gpio nets, full duplex
 * by GDMA, register level: the transmit bytes stream from an OUT descriptor
 * and the received ones land through an IN descriptor (spi_dma_start in
 * esp32c.h, esp-idf's order), mode 0, CS0 framing the transfer, SCK =
 * 100 kHz as in esp32c_spi_master.c.
 *
 *   GPIO4 SCK   GPIO5 MOSI   GPIO3 MISO   GPIO1 CS0     (GPIO matrix)
 *
 * Sends A5 3C 5A C3 0F F0 81 7E. RESULT[0..7] the bytes the IN descriptor
 * received, RESULT[8] SPI_DMA_INT_RAW, RESULT[9] the IN descriptor's dw0
 * after the transfer, RESULT[15] = 1 done.
 */
#include "esp32c.h"

#define N 8u

int main(void) {
    enable_clocks();
    matrix_out(4, SIG_FSPICLK, 0);
    matrix_out(5, SIG_FSPID, 0);
    matrix_out(1, SIG_FSPICS0, 0);
    matrix_in(3, SIG_FSPIQ);

    SPI_CLK_GATE = 7u;
    SPI_SLAVE = 0u;
    SPI_CLOCK = (15u << 18) | (49u << 12) | (24u << 6) | 49u;
    SPI_USER = (1u << 0) | (1u << 6) | (1u << 7) | (1u << 27) | (1u << 28);
    SPI_USER1 = 0u;
    SPI_MISC = 0x3Eu;
    SPI_MS_DLEN = 8u * N - 1u;
    REG(TX_BUF) = 0xC35A3CA5u;                 /* A5 3C 5A C3 */
    REG(TX_BUF + 4u) = 0x7E81F00Fu;            /* 0F F0 81 7E */
    REG(RX_BUF) = 0u;
    REG(RX_BUF + 4u) = 0u;
    spi_dma_start(N);
    SPI_CMD = SPI_UPDATE;
    while (SPI_CMD & SPI_UPDATE) {}

    delay_us(400);                             /* let the slave come up */
    SPI_INT_CLR = 0xFFFFFFFFu;
    SPI_CMD = SPI_USR;
    while (SPI_CMD & SPI_USR) {}
    for (unsigned i = 0; i < N; ++i) {
        RESULT[i] = *(volatile uint8_t *)(RX_BUF + i);
    }
    RESULT[8] = SPI_INT_RAW;
    RESULT[9] = REG(IN_DESC);
    RESULT[15] = 1u;
    for (;;) {}
}
