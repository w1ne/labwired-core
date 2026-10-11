/* RP2040 I2C0 (DW_apb_i2c) master on gpio nets, register level, polled.
 * GP4 SDA, GP5 SCL (FUNCSEL 3, open drain; the pull-up is the net's).
 * Standard mode: SCL high 600, low 650 clk_sys cycles. The same sequence
 * as i2c_controller.c, against target 0x42:
 *
 *   1. write 0x10 0xDE 0xAD (STOP on the last)
 *   2. write 0x10, then two reads (RESTART on the first, STOP on the last)
 *   3. write one byte to 0x50, where nobody answers: TX_ABRT
 *
 * RESULT[0..1] the two bytes read, RESULT[2] TX_ABRT in step 1 (0),
 * RESULT[3] TX_ABRT in step 3 (1), RESULT[4] STOP_DET count,
 * RESULT[5] IC_TX_ABRT_SOURCE in step 3, RESULT[7] done.
 */
#include "rp2040.h"

/* Wait for STOP_DET (counted in RESULT[4]); returns 1 when the transfer
 * aborted. The firmware keeps no .data/.bss (rp2040.ld). */
static uint32_t finish(void) {
    while (!(IC_RAW_INTR_STAT & INTR_STOP_DET)) {}
    (void)IC_CLR_STOP_DET;
    RESULT[4] += 1u;
    return (IC_RAW_INTR_STAT & INTR_TX_ABRT) ? 1u : 0u;
}

int main(void) {
    IO_CTRL(4) = FUNC_I2C;
    IO_CTRL(5) = FUNC_I2C;
    IC_ENABLE = 0u;
    /* MASTER_MODE, SPEED = standard, RESTART_EN, SLAVE_DISABLE, TX_EMPTY_CTRL */
    IC_CON = 1u | (1u << 1) | (1u << 5) | (1u << 6) | (1u << 8);
    IC_SS_SCL_HCNT = 600u;
    IC_SS_SCL_LCNT = 650u;
    IC_TAR = 0x42u;
    IC_ENABLE = 1u;

    delay(4000);                        /* let the target come up */
    IC_DATA_CMD = 0x10u;
    IC_DATA_CMD = 0xDEu;
    IC_DATA_CMD = 0xADu | CMD_STOP;
    RESULT[2] = finish();

    IC_DATA_CMD = 0x10u;
    IC_DATA_CMD = CMD_READ | CMD_RESTART;
    IC_DATA_CMD = CMD_READ | CMD_STOP;
    if (finish() == 0u) {
        RESULT[0] = IC_DATA_CMD & 0xFFu;
        RESULT[1] = IC_DATA_CMD & 0xFFu;
    } else {
        (void)IC_CLR_TX_ABRT;
        RESULT[0] = RESULT[1] = 0xEEu;
    }

    IC_ENABLE = 0u;
    IC_TAR = 0x50u;
    IC_ENABLE = 1u;
    IC_DATA_CMD = 0x55u | CMD_STOP;
    RESULT[3] = finish();
    RESULT[5] = IC_TX_ABRT_SOURCE;
    (void)IC_CLR_TX_ABRT;
    RESULT[7] = 1u;
    for (;;) {}
}

void reset(void) { (void)main(); for (;;) {} }
