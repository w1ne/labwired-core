/* IO-Link PHY over the simulated ESP32-C3 UART1 — see phy_c3_iolink.h.
 *
 * The LabWired generic UART model (Stm32F1 layout) at 0x60010000:
 *   - SR (status) at offset 0x00: read returns TX-ready bits, with RXNE (bit 5)
 *     set whenever the RX buffer (fed by the IO-Link master) has a byte.
 *   - DR at offset 0x04: write transmits one byte; read pops one RX byte.
 *   - A byte write to offset 0x00 is also a legacy TX alias.
 * The IO-Link line speed is irrelevant in the cycle-stepped sim, so set_baudrate
 * is a no-op. detect_wakeup scans for the 0x55 wake-up byte the master sends
 * first (mirrors the al2205 phy_labwired / iolinki phy_virtual). No timing is
 * enforced here — the device firmware calls iolink_set_timing_enforcement(false)
 * and the handshake is driven purely by byte arrival. */
#include "phy_c3_iolink.h"
#include <stdint.h>

#define UART1_BASE 0x60010000u
#define U1_SR (*(volatile uint32_t *)(UART1_BASE + 0x00u))
#define U1_DR (*(volatile uint32_t *)(UART1_BASE + 0x04u))
#define SR_RXNE (1u << 5)

static int phy_init(void) {
    /* The generic UART model needs no explicit enable to TX/RX in the sim. */
    return 0;
}

static void phy_set_mode(iolink_phy_mode_t mode) { (void)mode; }

static void phy_set_baudrate(iolink_baudrate_t baudrate) { (void)baudrate; }

static int phy_send(const uint8_t *data, size_t len) {
    for (size_t i = 0; i < len; i++) {
        U1_DR = (uint32_t)data[i];
    }
    return (int)len;
}

static int phy_recv_byte(uint8_t *byte) {
    if (U1_SR & SR_RXNE) {
        *byte = (uint8_t)U1_DR;
        return 1;
    }
    return 0;
}

static int phy_detect_wakeup(void) {
    uint8_t b;
    while (phy_recv_byte(&b) > 0) {
        if (b == 0x55u) {
            return 1;
        }
    }
    return 0;
}

static const iolink_phy_api_t PHY = {
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
