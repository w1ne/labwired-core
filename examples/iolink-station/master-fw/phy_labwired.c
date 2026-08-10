/* MASTER-side PHY over the simulated STM32L476 USART2, driven through the CMSIS
 * register definitions (USART2->CR1/ISR/RDR/TDR).
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
#include "stm32l476xx.h"
#include "phy_labwired.h"
#include <stdint.h>

/* COM2 (38.4 kbaud) is the IO-Link line speed this port runs at, so it is what
 * the wire must actually carry. The L476 stays on its MSI reset clock - 4 MHz,
 * the value this lab's system.yaml records as `cpu_hz`, since no PLL is ever
 * configured - and that clock reaches APB1 unprescaled. Under the default 16x
 * oversampling BRR IS USARTDIV (RM0351 38.5.4):
 *
 *   USARTDIV = f_ck / baud = 4 000 000 / 38 400 = 104.17 -> 104
 *   actual baud = 4 000 000 / 104 = 38 461.5 (+0.16%, inside 8N1 tolerance)
 *
 * BRR left at its 0 reset is not a slow link, it is no link: the divisor is
 * invalid, so there is no bit period at all and nothing reaches the wire. */
#define IOLINK_COM2_BRR 104u

/* PA2 = USART2_TX, PA3 = USART2_RX, both AF7 - STM32L476xx datasheet DS10198
 * Rev 11, Table 17 "Alternate function AF0 to AF7", p88, port A. */
#define CQ_TX_PIN 2u
#define CQ_RX_PIN 3u
#define CQ_AF 7u

/* Hand one pad to USART2: MODER = 10 (alternate function), push-pull, high
 * speed, no pull, and the AF nibble in AFR[0]. The AF number is what actually
 * selects which peripheral owns the pad, so it is the one field that has to
 * come from the datasheet rather than from habit. */
static void cq_pad_af(uint32_t pin) {
    const uint32_t shift = pin * 2u;
    GPIOA->MODER = (GPIOA->MODER & ~(3u << shift)) | (2u << shift);
    GPIOA->OTYPER &= ~(1u << pin);
    GPIOA->OSPEEDR |= 3u << shift;
    GPIOA->PUPDR &= ~(3u << shift);
    GPIOA->AFR[0] = (GPIOA->AFR[0] & ~(0xFu << (pin * 4u))) | (CQ_AF << (pin * 4u));
}

static int phy_init(void *user) {
    (void)user;
    cq_pad_af(CQ_TX_PIN);
    cq_pad_af(CQ_RX_PIN);
    USART2->CR1 = 0u;
    USART2->CR2 = 0u;
    USART2->CR3 = 0u;
    USART2->BRR = IOLINK_COM2_BRR;
    USART2->CR1 = USART_CR1_UE | USART_CR1_TE | USART_CR1_RE;
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
        while ((USART2->ISR & USART_ISR_TXE) == 0u) {
        }
        USART2->TDR = (uint32_t)data[i];
    }
    return (int)len;
}

static int phy_recv_byte(void *user, uint8_t *byte) {
    (void)user;
    if (USART2->ISR & USART_ISR_RXNE) {
        *byte = (uint8_t)USART2->RDR;
        return 1;
    }
    return 0;
}

/* ---- master config callbacks ---- */
static int m_set_mode_checked(iolink_phy_mode_t mode) {
    phy_set_mode(0, mode);
    return 0;
}
static int m_set_baudrate_checked(iolink_baudrate_t baudrate) {
    phy_set_baudrate(0, baudrate);
    return 0;
}
static int m_flush_rx(void) {
    uint8_t b;
    while (phy_recv_byte(0, &b) > 0) {
    }
    return 0;
}
static int m_prepare_tx(void) { return 0; }
static int m_prepare_rx(void) { return 0; }
static int m_wake_up(void) {
    uint8_t w = 0x55u;
    return phy_send(0, &w, 1u) == 1 ? 0 : -1;
}

static const iolink_phy_api_t PHY = {
    .user = 0,
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
