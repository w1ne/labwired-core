/* 4-port MASTER PHY over the simulated L476 USART2/3/4/5, driven through the
 * CMSIS register definitions.
 *
 * Each port gets its own generated set of send/recv/init/wake/flush functions
 * bound to one USART instance (PORT macro). Shared port-agnostic no-ops cover
 * mode/baudrate/prepare. Config matches the iolink-dido device firmware:
 * M-sequence type 1_1, 1-byte PD in.
 */
#include "stm32l476xx.h"
#include "phy_labwired.h"
#include <stdint.h>

static void p_set_mode(void *user, iolink_phy_mode_t m) {
    (void)user;
    (void)m;
}
static void p_set_baud(void *user, iolink_baudrate_t b) {
    (void)user;
    (void)b;
}
static int p_set_mode_chk(iolink_phy_mode_t m) {
    (void)m;
    return 0;
}
static int p_set_baud_chk(iolink_baudrate_t b) {
    (void)b;
    return 0;
}
static int p_prepare(void) { return 0; }

/* COM2 (38.4 kbaud) is what fill_one() asks the master stack for, so it is what
 * the wire must actually carry. The L476 runs this lab on the MSI reset clock,
 * 4 MHz (master/system.yaml `cpu_hz: 4_000_000` — no PLL is ever configured),
 * and that clock feeds APB1/APB2 unprescaled. Under the default 16x
 * oversampling BRR IS USARTDIV (RM0351 §38.5.4):
 *
 *   USARTDIV = f_ck / baud = 4 000 000 / 38 400 = 104.17 -> 104
 *   actual baud = 4 000 000 / 104 = 38 461.5 (+0.16%, well inside 8N1 tolerance)
 *
 * Leaving BRR at its 0 reset is not a slow link, it is no link: the divisor is
 * invalid, so there is no bit period at all. */
#define IOLINK_COM2_BRR 104u

/* Hand one pad to a USART: MODER = 10 (alternate function), push-pull, high
 * speed, no pull, and the AF nibble in AFR[pin/8]. The AF number is what
 * actually selects WHICH peripheral gets the pad, so it is the one field that
 * must come from the datasheet rather than from habit. */
static void pad_af(GPIO_TypeDef *g, uint32_t pin, uint32_t af) {
    const uint32_t shift = pin * 2u;
    g->MODER = (g->MODER & ~(3u << shift)) | (2u << shift);
    g->OTYPER &= ~(1u << pin);
    g->OSPEEDR |= 3u << shift;
    g->PUPDR &= ~(3u << shift);
    g->AFR[pin >> 3u] = (g->AFR[pin >> 3u] & ~(0xFu << ((pin & 7u) * 4u))) |
                        (af << ((pin & 7u) * 4u));
}

#define PORT(IDX, U, GTX, TXP, GRX, RXP, AF)                                    \
    static int send_##IDX(void *user, const uint8_t *d, size_t n) {             \
        (void)user;                                                             \
        for (size_t i = 0; i < n; i++) {                                        \
            while (((U)->ISR & USART_ISR_TXE) == 0u) {                          \
            }                                                                   \
            (U)->TDR = (uint32_t)d[i];                                          \
        }                                                                       \
        return (int)n;                                                         \
    }                                                                           \
    static int recv_##IDX(void *user, uint8_t *b) {                             \
        (void)user;                                                             \
        if ((U)->ISR & USART_ISR_RXNE) {                                        \
            *b = (uint8_t)(U)->RDR;                                             \
            return 1;                                                          \
        }                                                                       \
        return 0;                                                              \
    }                                                                           \
    static int init_##IDX(void *user) {                                         \
        (void)user;                                                             \
        pad_af((GTX), (TXP), (AF));                                             \
        pad_af((GRX), (RXP), (AF));                                             \
        (U)->CR1 = 0u;                                                          \
        (U)->CR2 = 0u;                                                          \
        (U)->CR3 = 0u;                                                          \
        (U)->BRR = IOLINK_COM2_BRR;                                             \
        (U)->CR1 = USART_CR1_UE | USART_CR1_TE | USART_CR1_RE;                  \
        return 0;                                                              \
    }                                                                           \
    static int wake_##IDX(void) {                                               \
        uint8_t w = 0x55u;                                                      \
        return send_##IDX(0, &w, 1u) == 1 ? 0 : -1;                            \
    }                                                                           \
    static int flush_##IDX(void) {                                              \
        uint8_t b;                                                             \
        while (recv_##IDX(0, &b) > 0) {                                         \
        }                                                                       \
        return 0;                                                              \
    }

/* Pad + AF per port, every row read off STM32L476xx datasheet DS10198 Rev 11:
 * Table 17 "Alternate function AF0 to AF7" for the USART1/2/3 rows and Table 18
 * "AF8 to AF15" for the UART4/5 rows.
 *   PA2/PA3   AF7  USART2_TX / USART2_RX   (Table 17, p88, port A)
 *   PB10/PB11 AF7  USART3_TX / USART3_RX   (Table 17, p89, port B)
 *   PA0/PA1   AF8  UART4_TX  / UART4_RX    (Table 18, p95, port A)
 *   PC12      AF8  UART5_TX                (Table 18, p97, port C)
 *   PD2       AF8  UART5_RX                (Table 18, p98, port D)
 * UART5 is the one port whose TX and RX sit on different GPIO ports, which is
 * why the macro takes a port per direction rather than one for both. */
PORT(0, USART2, GPIOA, 2u, GPIOA, 3u, 7u)
PORT(1, USART3, GPIOB, 10u, GPIOB, 11u, 7u)
PORT(2, UART4, GPIOA, 0u, GPIOA, 1u, 8u)
PORT(3, UART5, GPIOC, 12u, GPIOD, 2u, 8u)

static void fill_one(iolink_phy_api_t *phy, iolink_master_config_t *cfg,
                     int (*init)(void *), int (*send)(void *, const uint8_t *, size_t),
                     int (*recv)(void *, uint8_t *), int (*wake)(void), int (*flush)(void)) {
    phy->user = 0;
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
