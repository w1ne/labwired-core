/* MASTER-side PHY over the simulated STM32L476 USART2 (stm32v2 layout).
 *
 * Same register seam as the device firmware's phy: the simulator transmits on
 * any TDR write (TXE always ready) and exposes RX bytes via RXNE/RDR. Line
 * speed is irrelevant in the cycle-stepped sim, so baudrate/mode are no-ops.
 *
 * The master config callbacks (set_mode_checked, set_baudrate_checked, flush_rx,
 * prepare_tx, prepare_rx, wake_up) are the contract iolink-master requires for
 * an IO-Link port. wake_up sends the 0x55 wake-up byte the device firmware's
 * detect_wakeup scans for. The config matches the iolink-dido device firmware:
 * M-sequence type 1_1, 1-byte PD in, no PD out.
 */
#include "phy_labwired.h"
#include <stdint.h>

#define USART2_BASE 0x40004400u
#define REG(a) (*(volatile uint32_t *)(a))
#define U2_CR1 REG(USART2_BASE + 0x00u)
#define U2_ISR REG(USART2_BASE + 0x1Cu)
#define U2_RDR REG(USART2_BASE + 0x24u)
#define U2_TDR REG(USART2_BASE + 0x28u)
#define ISR_RXNE (1u << 5)
#define ISR_TXE (1u << 7)
#define CR1_UE (1u << 0)
#define CR1_RE (1u << 2)
#define CR1_TE (1u << 3)

static int phy_init(void) {
    U2_CR1 = CR1_UE | CR1_TE | CR1_RE;
    return 0;
}

static void phy_set_mode(iolink_phy_mode_t mode) { (void)mode; }
static void phy_set_baudrate(iolink_baudrate_t baudrate) { (void)baudrate; }

static int phy_send(const uint8_t *data, size_t len) {
    for (size_t i = 0; i < len; i++) {
        while ((U2_ISR & ISR_TXE) == 0u) {
        }
        U2_TDR = (uint32_t)data[i];
    }
    return (int)len;
}

static int phy_recv_byte(uint8_t *byte) {
    if (U2_ISR & ISR_RXNE) {
        *byte = (uint8_t)U2_RDR;
        return 1;
    }
    return 0;
}

/* ---- master config callbacks ---- */
static int m_set_mode_checked(iolink_phy_mode_t mode) {
    phy_set_mode(mode);
    return 0;
}
static int m_set_baudrate_checked(iolink_baudrate_t baudrate) {
    phy_set_baudrate(baudrate);
    return 0;
}
static int m_flush_rx(void) {
    uint8_t b;
    while (phy_recv_byte(&b) > 0) {
    }
    return 0;
}
static int m_prepare_tx(void) { return 0; }
static int m_prepare_rx(void) { return 0; }
static int m_wake_up(void) {
    uint8_t w = 0x55u;
    return phy_send(&w, 1u) == 1 ? 0 : -1;
}

static const iolink_phy_api_t PHY = {
    .init = phy_init,
    .set_mode = phy_set_mode,
    .set_baudrate = phy_set_baudrate,
    .send = phy_send,
    .recv_byte = phy_recv_byte,
    .detect_wakeup = 0,
    .set_cq_line = 0,
    .get_voltage_mv = 0,
    .is_short_circuit = 0,
};

const iolink_phy_api_t *phy_labwired_master_phy(void) { return &PHY; }

iolink_master_config_t phy_labwired_master_config(void) {
    iolink_master_config_t c = {0};
    c.port_mode = IOLINK_MASTER_PORT_MODE_IOLINK;
    c.m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_1_1; /* match iolink-dido device */
    c.baudrate = IOLINK_BAUDRATE_COM2;
    c.min_cycle_time = 20;
    c.pd_in_len = 1;
    c.pd_out_len = 0;
    c.auto_baudrate = false;
    c.response_timeout_100us = 3;
    c.set_mode_checked = m_set_mode_checked;
    c.set_baudrate_checked = m_set_baudrate_checked;
    c.flush_rx = m_flush_rx;
    c.prepare_tx = m_prepare_tx;
    c.prepare_rx = m_prepare_rx;
    c.wake_up = m_wake_up;
    return c;
}
