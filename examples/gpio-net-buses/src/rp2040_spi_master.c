/* RP2040 SPI0 (PL022) master on gpio nets, Motorola mode 0, 8-bit frames,
 * SCK = clk_peri / 512 (CPSDVSR 2, SCR 255), hardware chip select: with
 * SPH = 0 the PL022 pulses CSn high between frames.
 *
 *   GP2 SCK   GP3 TX (MOSI)   GP4 RX (MISO)   GP5 CSn     (FUNCSEL 1)
 *
 * Queues 4 bytes in the TX FIFO at once and reads what came back.
 * RESULT[0..3] received bytes, RESULT[4] SSPSR at the end, RESULT[7] done.
 */
#include "rp2040.h"

static const uint8_t tx[4] = {0xA5, 0x3C, 0x5A, 0xC3};

int main(void) {
    for (unsigned pin = 2; pin <= 5; ++pin) {
        IO_CTRL(pin) = FUNC_SPI;
    }
    SSPCPSR = 2u;
    SSPCR0 = (255u << 8) | 7u;          /* SCR 255, mode 0, DSS 8 bits */
    SSPCR1 = CR1_SSE;                   /* master, enabled */

    delay(2000);                        /* let the slave come up */
    for (int i = 0; i < 4; ++i) {
        SSPDR = tx[i];
    }
    for (int i = 0; i < 4; ++i) {
        while (!(SSPSR & SR_RNE)) {}
        RESULT[i] = SSPDR;
    }
    while (SSPSR & SR_BSY) {}
    RESULT[4] = SSPSR;
    RESULT[7] = 1u;
    for (;;) {}
}

void reset(void) { (void)main(); for (;;) {} }
