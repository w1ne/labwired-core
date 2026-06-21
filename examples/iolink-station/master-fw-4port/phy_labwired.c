/* 4-port MASTER PHY over the simulated L476 USART2/3/4/5 (stm32v2 layout).
 *
 * The iolink_phy_api_t callbacks carry no port context, so each port gets its
 * own generated set of send/recv/init/wake/flush functions bound to one USART
 * base (PORT macro). Shared port-agnostic no-ops cover mode/baudrate/prepare.
 * Config matches the iolink-dido device firmware: M-sequence type 1_1, 1-byte PD in.
 */
#include "phy_labwired.h"
#include <stdint.h>

#define REG(a) (*(volatile uint32_t *)(a))
#define ISR_RXNE (1u << 5)
#define ISR_TXE (1u << 7)
#define CR1_UE (1u << 0)
#define CR1_RE (1u << 2)
#define CR1_TE (1u << 3)

static void p_set_mode(iolink_phy_mode_t m) { (void)m; }
static void p_set_baud(iolink_baudrate_t b) { (void)b; }
static int p_set_mode_chk(iolink_phy_mode_t m) {
    (void)m;
    return 0;
}
static int p_set_baud_chk(iolink_baudrate_t b) {
    (void)b;
    return 0;
}
static int p_prepare(void) { return 0; }

#define PORT(IDX, BASE)                                                          \
    static int send_##IDX(const uint8_t *d, size_t n) {                          \
        for (size_t i = 0; i < n; i++) {                                         \
            while ((REG((BASE) + 0x1Cu) & ISR_TXE) == 0u) {                      \
            }                                                                    \
            REG((BASE) + 0x28u) = (uint32_t)d[i];                               \
        }                                                                        \
        return (int)n;                                                          \
    }                                                                            \
    static int recv_##IDX(uint8_t *b) {                                          \
        if (REG((BASE) + 0x1Cu) & ISR_RXNE) {                                    \
            *b = (uint8_t)REG((BASE) + 0x24u);                                   \
            return 1;                                                            \
        }                                                                        \
        return 0;                                                               \
    }                                                                            \
    static int init_##IDX(void) {                                                \
        REG((BASE) + 0x00u) = CR1_UE | CR1_TE | CR1_RE;                          \
        return 0;                                                               \
    }                                                                            \
    static int wake_##IDX(void) {                                                \
        uint8_t w = 0x55u;                                                       \
        return send_##IDX(&w, 1u) == 1 ? 0 : -1;                                 \
    }                                                                            \
    static int flush_##IDX(void) {                                               \
        uint8_t b;                                                              \
        while (recv_##IDX(&b) > 0) {                                             \
        }                                                                        \
        return 0;                                                               \
    }

PORT(0, 0x40004400u) /* USART2 */
PORT(1, 0x40004800u) /* USART3 */
PORT(2, 0x40004C00u) /* UART4  */
PORT(3, 0x40005000u) /* UART5  */

static void fill_one(iolink_phy_api_t *phy, iolink_master_config_t *cfg,
                     int (*init)(void), int (*send)(const uint8_t *, size_t),
                     int (*recv)(uint8_t *), int (*wake)(void), int (*flush)(void)) {
    phy->init = init;
    phy->set_mode = p_set_mode;
    phy->set_baudrate = p_set_baud;
    phy->send = send;
    phy->recv_byte = recv;
    phy->detect_wakeup = 0;
    phy->set_cq_line = 0;
    phy->get_voltage_mv = 0;
    phy->is_short_circuit = 0;

    iolink_master_config_t c = {0};
    c.port_mode = IOLINK_MASTER_PORT_MODE_IOLINK;
    c.m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_1_1;
    c.baudrate = IOLINK_BAUDRATE_COM2;
    c.min_cycle_time = 20;
    c.pd_in_len = 1;
    c.pd_out_len = 0;
    c.auto_baudrate = false;
    c.response_timeout_100us = 3;
    c.set_mode_checked = p_set_mode_chk;
    c.set_baudrate_checked = p_set_baud_chk;
    c.flush_rx = flush;
    c.prepare_tx = p_prepare;
    c.prepare_rx = p_prepare;
    c.wake_up = wake;
    *cfg = c;
}

void phy_labwired_fill(iolink_phy_api_t phys[LW_MASTER_PORTS],
                       iolink_master_config_t cfgs[LW_MASTER_PORTS]) {
    fill_one(&phys[0], &cfgs[0], init_0, send_0, recv_0, wake_0, flush_0);
    fill_one(&phys[1], &cfgs[1], init_1, send_1, recv_1, wake_1, flush_1);
    fill_one(&phys[2], &cfgs[2], init_2, send_2, recv_2, wake_2, flush_2);
    fill_one(&phys[3], &cfgs[3], init_3, send_3, recv_3, wake_3, flush_3);
}
