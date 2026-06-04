/* PHY backend over the simulated STM32L476 USART2 (stm32v2 register layout).
 *
 * The simulator transmits on any TDR write and reports TXE ready, and exposes
 * received bytes via RXNE/RDR, so only a token CR1 (UE|TE|RE) init is needed.
 * The IO-Link line speed is irrelevant in the cycle-stepped sim, so set_baudrate
 * is a no-op. detect_wakeup scans for the 0x55 wake-up byte (mirrors phy_virtual).
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

static void phy_set_mode(iolink_phy_mode_t mode) {
    (void)mode;
}

static void phy_set_baudrate(iolink_baudrate_t baudrate) {
    (void)baudrate;
}

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

const iolink_phy_api_t *iolink_phy_labwired_get(void) {
    return &PHY;
}
