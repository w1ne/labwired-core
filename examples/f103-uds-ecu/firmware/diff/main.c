/*
 * F103 bxCAN UDS LOOPBACK differential firmware.
 *
 * Self-contained sim-vs-silicon differential: a single bxCAN node in INTERNAL
 * LOOPBACK plays BOTH the UDS tester and the UDSLib ECU. No external tester is
 * required (the real connected F103 has no second CAN node).
 *
 * Bus traffic (all on the one looped node):
 *   inject  -> ECU (0x111): 10 0B 27 01 5A 11 22 33   FirstFrame  (FF_DL = 11)
 *   ECU     -> bus (0x222): 30 .. .. ..               FlowControl CTS
 *   inject  -> ECU (0x111): 21 44 55 66 77 88 55 55   ConsecutiveFrame SN=1
 *   ECU     -> bus (0x222): 06 67 01 DE AD BE EF       SecurityAccess seed
 *
 * On the final response (0x222 SF, data[1]==0x67) the firmware writes a result
 * block to RAM @ 0x20000400 for SWD readback (magic, length, packed bytes),
 * prints DIFF_RESP=<hex> and DIFF_DONE over USART1, then spins.
 *
 * The bxCAN model is strict: an accept-all filter is required or nothing is
 * received, and a degenerate BTR would bus-off. We use the silicon-valid
 * loopback BTR 0x40DC0009 (LBKM + valid TS1/TS2/BRP).
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

/* --- RCC (F1) — enable peripheral clocks. Harmless in the sim (no clock
 * gating modelled), REQUIRED on real silicon: USART1/CAN1/GPIO are unclocked
 * out of reset, so register writes are dropped and INAK/TXE never assert. --- */
#define RCC_BASE 0x40021000u
#define RCC_APB2ENR REG32(RCC_BASE + 0x18u)
#define RCC_APB1ENR REG32(RCC_BASE + 0x1Cu)
#define RCC_APB2ENR_AFIOEN (1u << 0)
#define RCC_APB2ENR_IOPAEN (1u << 2)
#define RCC_APB2ENR_USART1EN (1u << 14)
#define RCC_APB1ENR_CAN1EN (1u << 25)

static void rcc_init(void)
{
    RCC_APB2ENR |= RCC_APB2ENR_AFIOEN | RCC_APB2ENR_IOPAEN | RCC_APB2ENR_USART1EN;
    RCC_APB1ENR |= RCC_APB1ENR_CAN1EN;
}

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
    /* BOUNDED TXE wait. In the simulator the USART1 model asserts SR.TXE and
     * this drains immediately. On the REAL F103 USART1 is unclocked here (no
     * RCC enable / GPIO AF / baud), so SR.TXE never sets — an unbounded wait
     * would hang before the CAN/UDS exchange ever runs. The RAM result block
     * @ 0x20000400 is the silicon source of truth; UART is just a convenience
     * mirror, so we cap the wait and move on. */
    uint32_t guard = 0;
    while ((U1_SR & SR_TXE) == 0u) {
        if (++guard >= 100000u) return;
    }
    U1_DR = (uint32_t) (uint8_t) c;
}
static void uart_puts(const char *s)
{
    while (*s) uart_putc(*s++);
}
static void uart_hex8(uint8_t b)
{
    static const char hx[] = "0123456789ABCDEF";
    uart_putc(hx[(b >> 4) & 0xF]);
    uart_putc(hx[b & 0xF]);
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
/* Filter registers */
#define CAN_FMR REG32(CAN_BASE + 0x200u)
#define CAN_FM1R REG32(CAN_BASE + 0x204u)
#define CAN_FS1R REG32(CAN_BASE + 0x20Cu)
#define CAN_FFA1R REG32(CAN_BASE + 0x214u)
#define CAN_FA1R REG32(CAN_BASE + 0x21Cu)
#define CAN_F0R1 REG32(CAN_BASE + 0x240u)
#define CAN_F0R2 REG32(CAN_BASE + 0x244u)
#define MCR_INRQ (1u << 0)
#define MSR_INAK (1u << 0)
#define TI_TXRQ (1u << 0)
#define RF_RFOM (1u << 5)
/* Valid loopback bit timing: LBKM (bit30) + TS1=12, TS2=5, BRP=9. A degenerate
 * BTR with zero segments would bus-off on the real chip and in the model. */
#define BTR_LOOPBACK_VALID 0x40DC0009u

#define ECU_RX_ID 0x111u /* tester -> ECU (requests)  */
#define ECU_TX_ID 0x222u /* ECU -> tester (responses) */

static void can_init_loopback(void)
{
    CAN_MCR = MCR_INRQ; /* request initialization */
    while ((CAN_MSR & MSR_INAK) == 0u) {
    }
    CAN_BTR = BTR_LOOPBACK_VALID; /* valid timing + internal loopback (LBKM) */

    /* Accept-all filter: bank0, 32-bit, mask mode, mask=0 matches every id,
     * routed to FIFO0. Without a filter the strict model receives nothing. */
    CAN_FMR |= 1u;    /* FINIT: filter init mode */
    CAN_FA1R &= ~1u;  /* deactivate bank0 while configuring */
    CAN_FS1R |= 1u;   /* bank0 = single 32-bit scale */
    CAN_FM1R &= ~1u;  /* bank0 = mask mode */
    CAN_F0R1 = 0u;    /* id   = don't-care */
    CAN_F0R2 = 0u;    /* mask = 0 => every id matches */
    CAN_FFA1R &= ~1u; /* bank0 -> FIFO0 */
    CAN_FA1R |= 1u;   /* activate bank0 */
    CAN_FMR &= ~1u;   /* leave filter init */

    CAN_MCR = 0u; /* leave init -> running; LBKM stays asserted via BTR */
    while ((CAN_MSR & MSR_INAK) != 0u) {
    }
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

/* Raw frame TX via mailbox 0 — used both by the injector (tester role) and as
 * the UDSLib can_send hook (ECU role). On the looped node every TX is received
 * back into FIFO0. */
static int can_send(uint32_t id, const uint8_t *data, uint8_t len)
{
    if (len > 8u) len = 8u;
    CAN_TDL0R = pack_lo(data, len);
    CAN_TDH0R = pack_hi(data, len);
    CAN_TDT0R = len;                            /* DLC */
    CAN_TI0R = ((id & 0x7FFu) << 21) | TI_TXRQ; /* STID + transmit request */
    return 0;
}

typedef struct {
    uint32_t id;
    uint8_t len;
    uint8_t data[8];
} can_frame_t;

static bool can_poll(can_frame_t *f)
{
    if ((CAN_RF0R & 0x3u) == 0u) return false;
    f->id = (CAN_RI0R >> 21) & 0x7FFu;
    f->len = (uint8_t) (CAN_RDT0R & 0xFu);
    uint32_t lo = CAN_RDL0R, hi = CAN_RDH0R;
    for (uint8_t i = 0; i < 4; ++i) f->data[i] = (uint8_t) (lo >> (i * 8));
    for (uint8_t i = 0; i < 4; ++i) f->data[i + 4] = (uint8_t) (hi >> (i * 8));
    CAN_RF0R = RF_RFOM; /* release FIFO0 mailbox */
    return true;
}

/* --- UDS stack --- */
static uds_isotp_ctx_t g_iso;
static uint8_t g_iso_tx_sdu[64];
static uint8_t g_rx_buf[128];
static uint8_t g_tx_buf[128];
static uint32_t g_now_ms;

/* FIXED behaviour: a single consistent clock. N_Cr is armed from get_time_ms()
 * and uds_tp_isotp_process() is fed from the SAME counter, so the multi-frame
 * RX session never spuriously times out. */
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
    uart_puts("UDS_SEED_SERVED\n");
    seed[0] = 0xDE;
    seed[1] = 0xAD;
    seed[2] = 0xBE;
    seed[3] = 0xEF;
    return 4;
}

/* --- result block for SWD readback --- */
#define RESULT_ADDR 0x20000400u
#define RESULT_MAGIC 0x5EED0067u

static void store_result(const uint8_t *resp, uint8_t len)
{
    volatile uint32_t *r = (volatile uint32_t *) RESULT_ADDR;
    r[0] = RESULT_MAGIC;
    r[1] = (uint32_t) len;
    /* Pack response bytes little-endian into r[2], r[3], ... */
    for (uint8_t i = 0; i < 8; ++i) {
        uint8_t shift = (uint8_t) ((i & 3u) * 8u);
        if ((i & 3u) == 0u) r[2u + (i >> 2)] = 0u;
        if (i < len) r[2u + (i >> 2)] |= (uint32_t) resp[i] << shift;
    }
}

int main(void)
{
    rcc_init();
    uart_init();
    uart_puts("F103-UDS-DIFF\n");

    can_init_loopback();
    uart_puts("LOOPBACK_READY\n"); /* bxCAN loopback + accept-all filter up */

    uds_tp_isotp_init(&g_iso, can_send, ECU_TX_ID, ECU_RX_ID, g_iso_tx_sdu, sizeof(g_iso_tx_sdu));
    uds_tp_isotp_set_fd(&g_iso, false); /* classical CAN */

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

    /* Injected request frames (tester role). */
    static const uint8_t FF[8] = {0x10, 0x0B, 0x27, 0x01, 0x5A, 0x11, 0x22, 0x33};
    static const uint8_t CF[8] = {0x21, 0x44, 0x55, 0x66, 0x77, 0x88, 0x55, 0x55};

    bool ff_sent = false;
    bool fc_seen = false;
    bool cf_sent = false;
    bool done = false;
    uint8_t resp[8];
    uint8_t resp_len = 0;

    /* Bounded inject/poll loop — cannot hang. */
    for (uint32_t iter = 0; iter < 200000u && !done; ++iter) {
        /* Kick off the exchange: inject the FirstFrame once, as the tester. */
        if (!ff_sent) {
            can_send(ECU_RX_ID, FF, 8);
            ff_sent = true;
        }

        can_frame_t f;
        if (can_poll(&f)) {
            if (f.id == ECU_RX_ID) {
                /* Request frame: feed the ECU's ISO-TP receiver. */
                uds_isotp_rx_callback(&g_iso, &ctx, f.id, f.data, f.len);
            } else if (f.id == ECU_TX_ID) {
                /* ECU output: FlowControl, then final SecurityAccess response. */
                if (!fc_seen && (f.data[0] & 0xF0u) == 0x30u) {
                    fc_seen = true;
                } else if ((f.data[0] & 0xF0u) == 0x00u && f.len >= 2u && f.data[1] == 0x67u) {
                    /* Single Frame final response: payload len in low nibble. */
                    resp_len = (uint8_t) (f.data[0] & 0x0Fu);
                    if (resp_len > 7u) resp_len = 7u;
                    for (uint8_t i = 0; i < resp_len; ++i) resp[i] = f.data[1u + i];
                    done = true;
                }
            }
        }

        /* After the FlowControl, inject the ConsecutiveFrame once (tester). */
        if (fc_seen && !cf_sent) {
            can_send(ECU_RX_ID, CF, 8);
            cf_sent = true;
        }

        /* Drive the ECU stack from the SAME clock that arms N_Cr. */
        uds_process(&ctx);
        uds_tp_isotp_process(&g_iso, g_now_ms);
        ++g_now_ms;
    }

    if (done) {
        store_result(resp, resp_len);
        uart_puts("DIFF_RESP=");
        for (uint8_t i = 0; i < resp_len; ++i) uart_hex8(resp[i]);
        uart_putc('\n');
        uart_puts("DIFF_DONE\n");
    } else {
        uart_puts("DIFF_TIMEOUT\n");
    }

    for (;;) {
    }
}
