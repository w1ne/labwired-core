/* RP2040 I2C0 (DW_apb_i2c) slave at 0x42 on gpio nets, polled: the same
 * register file as i2c_target.c. GP4 SDA, GP5 SCL (FUNCSEL 3). The first
 * byte after the address (IC_DATA_CMD.FIRST_DATA_BYTE) sets the pointer,
 * later bytes store, reads (RD_REQ) return from it; SCL is stretched until
 * the answer is written.
 *
 * Built with -DSLOW=1 it waits about 10 us before answering a read, longer
 * than an SCL low period, so the controller must be held by the stretch.
 *
 * RESULT[0] transfers ended by STOP (STOP_DET, addressed only),
 * RESULT[1..2] regs[0x10..0x11], RESULT[3] bytes received, RESULT[4] bytes
 * sent, RESULT[7] = 1 once running.
 */
#include "rp2040.h"

#ifndef SLOW
#define SLOW 0
#endif

/* The register file lives in SRAM by address: the firmware keeps no
 * .data/.bss (rp2040.ld). */
#define regs ((volatile uint8_t *)0x20001000u)

int main(void) {
    uint8_t ptr = 0;
    IO_CTRL(4) = FUNC_I2C;
    IO_CTRL(5) = FUNC_I2C;
    IC_ENABLE = 0u;
    IC_CON = 1u << 7;                   /* slave, STOP_DET_IFADDRESSED */
    IC_SAR = 0x42u;
    IC_RX_TL = 0u;
    IC_ENABLE = 1u;
    RESULT[7] = 1u;
    for (;;) {
        uint32_t raw = IC_RAW_INTR_STAT;
        if (raw & INTR_RX_FULL) {
            uint32_t v = IC_DATA_CMD;
            if (v & FIRST_DATA_BYTE) {
                ptr = (uint8_t)v;
            } else {
                regs[ptr++] = (uint8_t)v;
            }
            RESULT[3] += 1u;
        } else if (raw & INTR_RD_REQ) {
#if SLOW
            delay(300);
#endif
            (void)IC_CLR_RD_REQ;
            IC_DATA_CMD = regs[ptr++];
            RESULT[4] += 1u;
        } else if (raw & INTR_STOP_DET) {
            (void)IC_CLR_STOP_DET;
            RESULT[0] += 1u;
            RESULT[1] = regs[0x10];
            RESULT[2] = regs[0x11];
        }
    }
}

void reset(void) { (void)main(); for (;;) {} }
