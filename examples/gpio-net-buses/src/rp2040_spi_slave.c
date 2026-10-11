/* RP2040 SPI0 (PL022) slave on gpio nets, Motorola mode 3 (SPO = SPH = 1),
 * 8-bit frames. As a slave the PL022's TX pad is MISO and its RX pad MOSI.
 *
 *   GP2 SCK in   GP3 TX (MISO)   GP4 RX (MOSI)   GP5 CSn in     (FUNCSEL 1)
 *
 * Preloads its four answers (0x81..0x84) in the TX FIFO and polls the RX
 * FIFO. RESULT[0..3] bytes received, RESULT[4] count, RESULT[7] done.
 */
#include "rp2040.h"

int main(void) {
    for (unsigned pin = 2; pin <= 5; ++pin) {
        IO_CTRL(pin) = FUNC_SPI;
    }
    SSPCPSR = 2u;
    SSPCR0 = CR0_SPO | CR0_SPH | 7u;    /* mode 3, DSS 8 bits */
    SSPCR1 = CR1_MS;                    /* slave, disabled */
    for (unsigned i = 0; i < 4u; ++i) {
        SSPDR = 0x81u + i;
    }
    SSPCR1 = CR1_MS | CR1_SSE;
    for (unsigned i = 0; i < 4u; ++i) {
        while (!(SSPSR & SR_RNE)) {}
        RESULT[i] = SSPDR;
        RESULT[4] = i + 1u;
    }
    RESULT[7] = 1u;
    for (;;) {}
}

void reset(void) { (void)main(); for (;;) {} }
