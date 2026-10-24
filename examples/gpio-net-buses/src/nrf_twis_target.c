/* nRF52840 TWIS0 target at 0x42 on gpio nets against the STM32G071 I2C1
 * controller (i2c_controller.c). P0.27 SCL, P0.26 SDA. A 256-byte register
 * file, as i2c_target.c: the first byte of a write sets the pointer, later
 * bytes store, reads return from it, auto-incrementing. Polled: on WRITE the
 * firmware prepares the RX buffer, on READ it fills and prepares the TX
 * buffer; until it does the target stretches SCL.
 *
 * Built with -DSLOW=1 the firmware waits about 25 us before each prepare,
 * several of the controller's SCL low periods, so the controller must be
 * held by the stretch.
 *
 * RESULT[0] transfers ended by STOP, RESULT[1..2] regs[0x10..0x11],
 * RESULT[3] bytes received, RESULT[4] bytes sent, RESULT[5] READ events,
 * RESULT[6] WRITE events.
 */
#include "nrf52.h"

#ifndef SLOW
#define SLOW 0
#endif

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 48] = {
    [0] = 0x20040000u,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
};

static uint8_t regs[256];
static uint32_t ptr;
static uint32_t pending_rx;
static uint32_t pending_tx;

/* Take what the last write phase delivered: a pointer, then data. */
static void take_rx(void) {
    if (!pending_rx) {
        return;
    }
    pending_rx = 0u;
    uint32_t n = S0_RXD_AMOUNT;
    RESULT[3] += n;
    if (n == 0u) {
        return;
    }
    ptr = RXBUF[0];
    for (uint32_t i = 1; i < n; ++i) {
        regs[ptr & 0xFFu] = RXBUF[i];
        ptr += 1u;
    }
}

int main(void) {
    P0_PIN_CNF(26) = PIN_TWI;
    P0_PIN_CNF(27) = PIN_TWI;
    TWIS_PSEL_SCL = 27u;
    TWIS_PSEL_SDA = 26u;
    TWIS_ADDRESS0 = 0x42u;
    TWIS_CONFIG = 1u;                   /* ADDRESS[0] */
    S0_ORC = 0xFFu;
    S0_ENABLE = 9u;                     /* TWIS */
    for (;;) {
        if (TWIS_EVENTS_WRITE) {
            TWIS_EVENTS_WRITE = 0u;
            RESULT[6] += 1u;
#if SLOW
            delay(400);
#endif
            S0_RXD_PTR = (uint32_t)RXBUF;
            S0_RXD_MAXCNT = 8u;
            pending_rx = 1u;
            TWIS_TASKS_PREPARERX = 1u;
        }
        if (TWIS_EVENTS_READ) {
            TWIS_EVENTS_READ = 0u;
            RESULT[5] += 1u;
            take_rx();
#if SLOW
            delay(400);
#endif
            for (uint32_t i = 0; i < 8u; ++i) {
                TXBUF[i] = regs[(ptr + i) & 0xFFu];
            }
            S0_TXD_PTR = (uint32_t)TXBUF;
            S0_TXD_MAXCNT = 8u;
            pending_tx = 1u;
            TWIS_TASKS_PREPARETX = 1u;
            /* What went out is counted at STOP (TXD.AMOUNT). */
        }
        if (TWIS_EVENTS_STOPPED) {
            TWIS_EVENTS_STOPPED = 0u;
            take_rx();
            if (pending_tx) {
                pending_tx = 0u;
                RESULT[4] += S0_TXD_AMOUNT;
                ptr += S0_TXD_AMOUNT;
            }
            RESULT[0] += 1u;
            RESULT[1] = regs[0x10];
            RESULT[2] = regs[0x11];
        }
    }
}

void reset(void) { (void)main(); for (;;) {} }
