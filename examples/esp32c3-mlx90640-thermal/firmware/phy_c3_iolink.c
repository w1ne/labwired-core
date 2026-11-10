/* IO-Link PHY over the simulated ESP32-C3 UART1 — see phy_c3_iolink.h.
 *
 * Uses the native C3 FIFO at 0x00 and RXFIFO_CNT in STATUS at 0x1c. The old
 * STM32 SR/DR offsets read the FIFO as status and wrote INT_RAW, so no reply
 * reached the peer. This fixture uses a fixed COM2 UART, not a transceiver's
 * electrical wake-up or baud negotiation. The synthetic peer sends 0x55 as
 * its wake marker. Device DLL timing enforcement remains disabled. */
#include "phy_c3_iolink.h"
#include "c3_time.h"
#include <stdint.h>

#define UART1_BASE 0x60010000u
#define U1_FIFO (*(volatile uint32_t *)(UART1_BASE + 0x00u))
#define U1_CLKDIV (*(volatile uint32_t *)(UART1_BASE + 0x14u))
#define U1_STATUS (*(volatile uint32_t *)(UART1_BASE + 0x1cu))
#define U1_CONF0 (*(volatile uint32_t *)(UART1_BASE + 0x20u))
#define U1_CLK_CONF (*(volatile uint32_t *)(UART1_BASE + 0x78u))

static int phy_init(void *user) {
    (void)user;
    U1_CLK_CONF = (1u << 20) | (1u << 22) | (1u << 24) | (1u << 25);
    U1_CLKDIV = 2083u; /* 80 MHz APB / 38400 baud, integer divisor */
    U1_CONF0 = (1u << 1) | (3u << 2) | (1u << 4); /* 8E1 */
    return 0;
}

static void phy_set_mode(void *user, iolink_phy_mode_t mode) {
    (void)user;
    (void)mode;
}

static void phy_set_baudrate(void *user, iolink_baudrate_t baudrate) {
    (void)user;
    (void)baudrate;
}

static int phy_send(void *user, const uint8_t *data, size_t len) {
    (void)user;
    for (size_t i = 0; i < len; i++) {
        while (((U1_STATUS >> 16) & 0x3ffu) >= 128u) {
        }
        U1_FIFO = (uint32_t)data[i];
    }
    return (int)len;
}

static int phy_recv_byte(void *user, uint8_t *byte) {
    (void)user;
    if ((U1_STATUS & 0x3ffu) != 0u) {
        *byte = (uint8_t)U1_FIFO;
        return 1;
    }
    return 0;
}

void iolink_phy_c3_flush_tx(void) {
    while (((U1_STATUS >> 16) & 0x3ffu) != 0u) {
        c3_sleep_us(100u);
    }
}

static int phy_detect_wakeup(void *user) {
    uint8_t b;
    while (phy_recv_byte(user, &b) > 0) {
        if (b == 0x55u) {
            return 1;
        }
    }
    return 0;
}

static const iolink_phy_api_t PHY = {
    .user = 0,
    .init = phy_init,
    .set_mode = phy_set_mode,
    .set_baudrate = phy_set_baudrate,
    .send = phy_send,
    .recv_byte = phy_recv_byte,
    .detect_wakeup = phy_detect_wakeup,
    .set_cq_line = 0,
    .get_voltage_mv = 0,
    .is_short_circuit = 0,
};

const iolink_phy_api_t *iolink_phy_c3_get(void) { return &PHY; }
