/*
 * Real XC16 firmware exercising the dsPIC33F DSP MAC class on the
 * dsPIC33FJ32MC204 model.
 *
 * Build (XC16 v1.70 container, same recipe as the M0 firmware):
 *   xc16-gcc -mcpu=33FJ32MC204 -O0 -Wall \
 *     -Wl,-T,/opt/microchip/xc16/support/dsPIC33F/gld/p33FJ32MC204.gld \
 *     -o dspic33fj32mc204-xc16-dsp-mac.elf xc16-dsp-mac.c
 *
 * The program runs a fixed sequence of MAC-class instructions whose inputs
 * are the DS70000157G worked examples (example 1 of MAC/MPY/SAC/SAC.R), then
 * clamps ACCA with normal saturation and writes a completion marker to
 * 0x0E00. `crates/core/tests/pic24_xc16_dsp.rs` steps until the marker
 * appears and checks the accumulators, W13 (write-back), the prefetch
 * registers and the SR flags.
 *
 * Data layout: X data 0x0A00, Y data 0x0C00 (the part's X/Y split is
 * 0x0800-0x0BFF / 0x0C00-0x0FFF), results 0x0E00-0x0E04.
 */

#include <xc.h>

int main(void)
{
    CORCON = 0x00C0; /* fractional multiplies, normal (1.31) saturation */

    __asm__ volatile(
        /* X and Y operands for the prefetched MAC (PRM MAC example 1). */
        "mov #0x2567, w0 \n"
        "mov w0, 0x0A00 \n"
        "mov #0x909C, w0 \n"
        "mov w0, 0x0C00 \n"
        /* ACCA = 0x1200_0000 (the example's pre-MAC accumulator). */
        "mov #0x1200, w2 \n"
        "lac w2, #0, a \n"
        /* MAC W4*W5, A, [W8]+=2, W4, [W10]+=2, W5
           ACCA = 0x1200_0000 + (0xA022 * 0xB900 << 1) = 0x472D_2400. */
        "mov #0xA022, w4 \n"
        "mov #0xB900, w5 \n"
        "mov #0x0A00, w8 \n"
        "mov #0x0C00, w10 \n"
        "mac w4*w5, a, [w8]+=2, w4, [w10]+=2, w5 \n"
        /* MPY W6*W7, B: the product replaces ACCB; MSC subtracts it again. */
        "mov #0x1234, w6 \n"
        "mov #0x5678, w7 \n"
        "mpy w6*w7, b \n"
        "msc w6*w7, b, w13 \n"
        /* SAC A, #4, W0: W0 = ACCxH after an arithmetic right shift. */
        "sac a, #4, w0 \n"
        "mov w0, 0x0E02 \n"
        /* SAC.R A, #4, W1 with ACCA = 0x120F_FF00 (PRM SAC.R example 1):
           the low word rounds 0xFF00 up. */
        "mov #0x120F, w0 \n"
        "mov w0, 0x24 \n"
        "mov #0xFF00, w0 \n"
        "mov w0, 0x22 \n"
        "sac.r a, #4, w1 \n"
        "mov w1, 0x0E04 \n"
        /* Normal saturation: MPY W4*W4, A with W4 = 0x8000 is exactly +2^31,
           so ACCA clamps to 0x7FFF_FFFF and SA is set. */
        "mov #0x8000, w4 \n"
        "mpy w4*w4, a \n"
        /* Completion marker. */
        "mov #0xBEEF, w0 \n"
        "mov w0, 0x0E00 \n");

    for (;;) {
    }
}
