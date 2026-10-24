/* nRF52840 TWIM0 controller on gpio nets against the STM32G071 I2C1 target
 * (i2c_target.c) at 0x42. P0.27 SCL, P0.26 SDA (the pull-up is the net's),
 * 250 kHz (FREQUENCY = K250). The same sequence as i2c_controller.c:
 *
 *   1. write 0x10 0xDE 0xAD (SHORTS LASTTX_STOP)
 *   2. write 0x10, repeated START, read two bytes
 *      (SHORTS LASTTX_STARTRX | LASTRX_STOP)
 *   3. write one byte to 0x50, where nobody answers: ERROR with
 *      ERRORSRC.ANACK, then STOP (nrfx's RESUME + STOP)
 *
 * RESULT[0..1] the two bytes read, RESULT[2] ERRORSRC after step 1 (0),
 * RESULT[3] ERRORSRC after step 3 (ANACK = 2), RESULT[4] STOPPED count,
 * RESULT[5] TXD.AMOUNT of step 3 (0), RESULT[6] RXD.AMOUNT of step 2 (2),
 * RESULT[7] done.
 */
#include "nrf52.h"

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 48] = {
    [0] = 0x20040000u,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
};

static uint32_t stops;

/* Wait for STOPPED; on ERROR first, stop the held bus. */
static void finish(void) {
    for (;;) {
        if (TWIM_EVENTS_ERROR) {
            TWIM_EVENTS_ERROR = 0u;
            TWIM_TASKS_RESUME = 1u;
            TWIM_TASKS_STOP = 1u;
        }
        if (TWIM_EVENTS_STOPPED) {
            TWIM_EVENTS_STOPPED = 0u;
            stops += 1u;
            return;
        }
    }
}

int main(void) {
    P0_PIN_CNF(26) = PIN_TWI;
    P0_PIN_CNF(27) = PIN_TWI;
    TWIM_PSEL_SCL = 27u;
    TWIM_PSEL_SDA = 26u;
    TWIM_FREQUENCY = 0x04000000u;       /* K250 */
    TWIM_ADDRESS = 0x42u;
    S0_ENABLE = 6u;                     /* TWIM */

    delay(400);                         /* let the target come up */
    TXBUF[0] = 0x10u;
    TXBUF[1] = 0xDEu;
    TXBUF[2] = 0xADu;
    S0_TXD_PTR = (uint32_t)TXBUF;
    S0_TXD_MAXCNT = 3u;
    S0_SHORTS = TWIM_SHORT_LASTTX_STOP;
    TWIM_TASKS_STARTTX = 1u;
    finish();
    RESULT[2] = TWIM_ERRORSRC;

    S0_TXD_MAXCNT = 1u;                 /* the pointer 0x10 again */
    S0_RXD_PTR = (uint32_t)RXBUF;
    S0_RXD_MAXCNT = 2u;
    S0_SHORTS = TWIM_SHORT_LASTTX_STARTRX | TWIM_SHORT_LASTRX_STOP;
    TWIM_TASKS_STARTTX = 1u;
    finish();
    RESULT[0] = RXBUF[0];
    RESULT[1] = RXBUF[1];
    RESULT[6] = S0_RXD_AMOUNT;

    TWIM_ADDRESS = 0x50u;
    TXBUF[0] = 0x55u;
    S0_SHORTS = TWIM_SHORT_LASTTX_STOP;
    TWIM_TASKS_STARTTX = 1u;
    finish();
    RESULT[3] = TWIM_ERRORSRC;
    RESULT[5] = S0_TXD_AMOUNT;
    TWIM_ERRORSRC = 0x7u;               /* W1C */
    RESULT[4] = stops;
    RESULT[7] = 1u;
    for (;;) {}
}

void reset(void) { (void)main(); for (;;) {} }
