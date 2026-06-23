/* PHY backend over the simulated STM32L476 USART2, driven through the CMSIS
 * register definitions (USART2->CR1/ISR/RDR/TDR).
 *
 * The simulator transmits on any TDR write and reports TXE ready, and exposes
 * received bytes via RXNE/RDR, so only a token CR1 (UE|TE|RE) init is needed.
 * The IO-Link line speed is irrelevant in the cycle-stepped sim, so set_baudrate
 * is a no-op. detect_wakeup scans for the 0x55 wake-up byte (mirrors phy_virtual).
 */
#include "stm32l476xx.h"
#include "phy_labwired.h"
#include <stdint.h>

static int phy_init(void) {
    USART2->CR1 = USART_CR1_UE | USART_CR1_TE | USART_CR1_RE;
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
        while ((USART2->ISR & USART_ISR_TXE) == 0u) {
        }
        USART2->TDR = (uint32_t)data[i];
    }
    return (int)len;
}

static int phy_recv_byte(uint8_t *byte) {
    if (USART2->ISR & USART_ISR_RXNE) {
        *byte = (uint8_t)USART2->RDR;
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
