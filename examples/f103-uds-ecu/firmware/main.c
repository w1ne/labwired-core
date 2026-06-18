/*
 * F103 bxCAN UDS ECU — reproduction harness for w1c/udslib issue #29
 * ("FF first frame receive error").
 *
 * Runs the real UDSLib ISO-TP + UDS core on the emulated STM32F103 bxCAN in
 * internal loopback. The firmware plays BOTH roles on the single looped-back
 * node: as the *tester* it injects the exact CAN frames captured on the
 * reporter's PCAN bus, and as the *ECU* it runs UDSLib and answers. Every
 * frame the ECU emits loops back into RX FIFO0, so we can watch whether the
 * multi-frame request actually produces a response.
 *
 *   tester -> ECU (0x111): 10 0B 27 01 5A 11 22 33   FirstFrame (FF_DL = 11)
 *   ECU -> tester (0x222): 30 08 00 ...              FlowControl CTS
 *   tester -> ECU (0x111): 21 44 55 66 77 88 ..      ConsecutiveFrame SN=1
 *   ECU -> tester (0x222): 06 67 01 DE AD BE EF      SecurityAccess seed  <-- the fix
 */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "uds/uds_core.h"
#include "uds/uds_isotp.h"

/* --- freestanding libc shims (no libc linked) --- */
void *memcpy(void *dst, const void *src, size_t n)
{
    uint8_t *d = dst;
    const uint8_t *s = src;
    while (n-- > 0u) *d++ = *s++;
    return dst;
}
void *memset(void *dst, int v, size_t n)
{
    uint8_t *d = dst;
    while (n-- > 0u) *d++ = (uint8_t) v;
    return dst;
}
int memcmp(const void *a, const void *b, size_t n)
{
    const uint8_t *x = a, *y = b;
    while (n-- > 0u) {
        if (*x != *y) return (int) *x - (int) *y;
        ++x;
        ++y;
    }
    return 0;
}
void *__aeabi_memcpy(void *d, const void *s, size_t n) { return memcpy(d, s, n); }
void *__aeabi_memcpy4(void *d, const void *s, size_t n) { return memcpy(d, s, n); }
void *__aeabi_memcpy8(void *d, const void *s, size_t n) { return memcpy(d, s, n); }
void *__aeabi_memset(void *d, size_t n, int v) { return memset(d, v, n); }
void *__aeabi_memclr(void *d, size_t n) { return memset(d, 0, n); }
void *__aeabi_memclr4(void *d, size_t n) { return memset(d, 0, n); }
void *__aeabi_memclr8(void *d, size_t n) { return memset(d, 0, n); }

#define REG32(addr) (*(volatile uint32_t *) (addr))

/* --- USART1 (F1 layout: SR @ 0x00, DR @ 0x04, CR1 @ 0x0C) --- */
#define USART1_BASE 0x40013800u
#define U1_SR REG32(USART1_BASE + 0x00u)
#define U1_DR REG32(USART1_BASE + 0x04u)
#define U1_CR1 REG32(USART1_BASE + 0x0Cu)
#define SR_TXE (1u << 7)
#define CR1_UE (1u << 13)
#define CR1_TE (1u << 3)

static void uart_init(void) { U1_CR1 = CR1_UE | CR1_TE; }
static void uart_putc(char c)
{
    while ((U1_SR & SR_TXE) == 0u) {
    }
    U1_DR = (uint32_t) (uint8_t) c;
}
static void uart_puts(const char *s)
{
    while (*s) uart_putc(*s++);
}
static void uart_hex8(uint8_t b)
{
    static const char hex[] = "0123456789ABCDEF";
    uart_putc(hex[(b >> 4) & 0xF]);
    uart_putc(hex[b & 0xF]);
}

/* --- bxCAN @ 0x40006400 (RM0008 §24.9) --- */
#define CAN_BASE 0x40006400u
#define CAN_MCR REG32(CAN_BASE + 0x000u)
#define CAN_MSR REG32(CAN_BASE + 0x004u)
#define CAN_TSR REG32(CAN_BASE + 0x008u)
#define CAN_RF0R REG32(CAN_BASE + 0x00Cu)
#define CAN_BTR REG32(CAN_BASE + 0x01Cu)
#define CAN_TI0R REG32(CAN_BASE + 0x180u)
#define CAN_TDT0R REG32(CAN_BASE + 0x184u)
#define CAN_TDL0R REG32(CAN_BASE + 0x188u)
#define CAN_TDH0R REG32(CAN_BASE + 0x18Cu)
#define CAN_RI0R REG32(CAN_BASE + 0x1B0u)
#define CAN_RDT0R REG32(CAN_BASE + 0x1B4u)
#define CAN_RDL0R REG32(CAN_BASE + 0x1B8u)
#define CAN_RDH0R REG32(CAN_BASE + 0x1BCu)
#define MCR_INRQ (1u << 0)
#define TI_TXRQ (1u << 0)
#define RF_RFOM (1u << 5)
#define BTR_LBKM (1u << 30)

typedef struct {
    uint32_t id;
    uint8_t len;
    uint8_t data[8];
} can_frame_t;

static void can_init_loopback(void)
{
    CAN_MCR = MCR_INRQ;          /* request initialization */
    CAN_BTR = BTR_LBKM;          /* internal loopback */
    CAN_MCR = 0u;                /* leave init -> running */
}

static uint32_t pack_lo(const uint8_t *d, uint8_t len)
{
    uint32_t w = 0;
    for (uint8_t i = 0; i < 4 && i < len; ++i) w |= (uint32_t) d[i] << (i * 8);
    return w;
}
static uint32_t pack_hi(const uint8_t *d, uint8_t len)
{
    uint32_t w = 0;
    for (uint8_t i = 4; i < 8 && i < len; ++i) w |= (uint32_t) d[i] << ((i - 4) * 8);
    return w;
}

/* UDSLib can_send hook: standard-ID classical frame via TX mailbox 0. */
static int can_send(uint32_t id, const uint8_t *data, uint8_t len)
{
    if (len > 8u) len = 8u;
    CAN_TDL0R = pack_lo(data, len);
    CAN_TDH0R = pack_hi(data, len);
    CAN_TDT0R = len;                                /* DLC */
    CAN_TI0R = ((id & 0x7FFu) << 21) | TI_TXRQ;     /* STID + transmit request */
    return 0;
}

static bool can_poll(can_frame_t *f)
{
    if ((CAN_RF0R & 0x3u) == 0u) return false;
    f->id = (CAN_RI0R >> 21) & 0x7FFu;
    f->len = (uint8_t) (CAN_RDT0R & 0xFu);
    uint32_t lo = CAN_RDL0R, hi = CAN_RDH0R;
    for (uint8_t i = 0; i < 4; ++i) f->data[i] = (uint8_t) (lo >> (i * 8));
    for (uint8_t i = 0; i < 4; ++i) f->data[i + 4] = (uint8_t) (hi >> (i * 8));
    CAN_RF0R = RF_RFOM;                             /* release FIFO0 mailbox */
    return true;
}

/* --- UDS stack --- */
#define ECU_RX_ID 0x111u /* tester -> ECU */
#define ECU_TX_ID 0x222u /* ECU -> tester */

static uds_isotp_ctx_t g_iso;
static uint8_t g_iso_tx_sdu[64];
static uint8_t g_rx_buf[128];
static uint8_t g_tx_buf[128];
static uint32_t g_now_ms;

/* Captured ECU->tester response carrying a SecurityAccess positive reply. */
static bool g_resp_seen;
static can_frame_t g_resp;

static uint32_t get_time_ms(void) { return g_now_ms; }

static int isotp_send_adapter(struct uds_ctx *ctx, const uint8_t *data, uint16_t len)
{
    (void) ctx;
    return uds_isotp_send(&g_iso, data, len);
}

static int security_seed(struct uds_ctx *ctx, uint8_t level, uint8_t *seed, uint16_t max_len)
{
    (void) ctx;
    (void) level;
    (void) max_len;
    seed[0] = 0xDE;
    seed[1] = 0xAD;
    seed[2] = 0xBE;
    seed[3] = 0xEF;
    return 4;
}

/* Drain RX FIFO0: tester frames feed the ECU; ECU frames (0x222) are the bus
   monitor — a single-frame 0x67 reply is the SecurityAccess seed response. */
static void pump(uds_ctx_t *ctx)
{
    can_frame_t f;
    while (can_poll(&f)) {
        if (f.id == ECU_RX_ID) {
            uds_isotp_rx_callback(&g_iso, ctx, f.id, f.data, f.len);
        } else if (f.id == ECU_TX_ID) {
            uart_puts("CAN_RX 222:");
            for (uint8_t i = 0; i < f.len; ++i) {
                uart_putc(' ');
                uart_hex8(f.data[i]);
            }
            uart_putc('\n');
            if (f.data[0] == 0x06u && f.data[1] == 0x67u) {
                g_resp = f;
                g_resp_seen = true;
            }
        }
    }
}

int main(void)
{
    uart_init();
    uart_puts("F103-UDS-ECU\n");

    can_init_loopback();

    uds_tp_isotp_init(&g_iso, can_send, ECU_TX_ID, ECU_RX_ID, g_iso_tx_sdu, sizeof(g_iso_tx_sdu));
    uds_tp_isotp_set_fd(&g_iso, false); /* classical CAN, like the F103 */

    uds_config_t cfg = {
        .ecu_address = 0x10u,
        .get_time_ms = get_time_ms,
        .fn_tp_send = isotp_send_adapter,
        .fn_security_seed = security_seed,
        .rx_buffer = g_rx_buf,
        .rx_buffer_size = sizeof(g_rx_buf),
        .tx_buffer = g_tx_buf,
        .tx_buffer_size = sizeof(g_tx_buf),
        .p2_ms = 50u,
        .p2_star_ms = 2000u,
    };
    uds_ctx_t ctx;
    if (uds_init(&ctx, &cfg) != UDS_OK) {
        uart_puts("UDS_INIT_FAIL\n");
        for (;;) {
        }
    }

    /* FirstFrame: SecurityAccess requestSeed carrying 9 extra bytes (FF_DL=11). */
    static const uint8_t ff[8] = {0x10, 0x0B, 0x27, 0x01, 0x5A, 0x11, 0x22, 0x33};
    uart_puts("UDS_REQ_27_01_FF\n");
    can_send(ECU_RX_ID, ff, 8);
    pump(&ctx);
    uds_process(&ctx);
    uds_tp_isotp_process(&g_iso, ++g_now_ms);

    /* ConsecutiveFrame SN=1: last 5 payload bytes + padding. */
    static const uint8_t cf[8] = {0x21, 0x44, 0x55, 0x66, 0x77, 0x88, 0x55, 0x55};
    uart_puts("UDS_REQ_27_01_CF\n");
    can_send(ECU_RX_ID, cf, 8);
    pump(&ctx);
    uds_process(&ctx);
    uds_tp_isotp_process(&g_iso, ++g_now_ms);
    pump(&ctx);

    if (g_resp_seen) {
        uart_puts("UDS_RESP_67_SEED=");
        for (uint8_t i = 0; i < g_resp.len; ++i) uart_hex8(g_resp.data[i]);
        uart_putc('\n');
        uart_puts("UDS_OK\n");
    } else {
        uart_puts("UDS_FAIL_NO_RESPONSE\n");
    }

    for (;;) {
    }
}
