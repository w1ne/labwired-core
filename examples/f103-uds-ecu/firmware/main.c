/*
 * F103 bxCAN UDS ECU — silicon-correct, normal-mode (NOT loopback).
 *
 * Real UDSLib on the emulated STM32F103 bxCAN, configured exactly as real
 * silicon requires: a valid BTR bit-timing and an acceptance filter for the
 * request ID. A separate virtual UDS tester node drives the CAN bus (sends the
 * multi-frame SecurityAccess request); this firmware is just the ECU — it
 * receives filtered frames, runs UDSLib ISO-TP, and answers on the bus.
 *
 *   tester -> ECU (0x111): 10 0B 27 01 5A 11 22 33   FirstFrame  (FF_DL = 11)
 *   ECU -> tester (0x222): 30 08 00 ...              FlowControl CTS
 *   tester -> ECU (0x111): 21 44 55 66 77 88 ..      ConsecutiveFrame SN=1
 *   ECU -> tester (0x222): 06 67 01 DE AD BE EF      SecurityAccess seed
 *
 * Because the bxCAN model is strict (no filter => no RX; degenerate BTR =>
 * bus-off), this firmware must do the real silicon setup — no lenient shortcut.
 */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "uds/uds_core.h"
#include "uds/uds_isotp.h"
#include "uds_ecu_app.h"

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

/* --- RCC (F1) — enable peripheral clocks. REQUIRED on real silicon and now in
 * the sim (clock-gating modelled): USART1/CAN1/GPIOA/AFIO are unclocked out of
 * reset, so their register writes are dropped and INAK/TXE never assert until
 * the matching RCC enable bit is set. --- */
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
    while ((U1_SR & SR_TXE) == 0u) {
    }
    U1_DR = (uint32_t) (uint8_t) c;
}
static void uart_puts(const char *s)
{
    while (*s) uart_putc(*s++);
}

void uds_ecu_app_log(const char *msg) { uart_puts(msg); }

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
/* Valid bit timing (TS1=12, TS2=5, BRP=9) — silicon-captured working value
 * (loopback used 0x40DC0009; normal mode drops the LBKM bit). A degenerate
 * BTR with zero segments would bus-off on the real chip and in the model. */
#define BTR_NORMAL_VALID 0x00DC0009u

#define ECU_RX_ID 0x111u /* tester -> ECU (requests)  */
#define ECU_TX_ID 0x222u /* ECU -> tester (responses) */

static void can_init_normal(void)
{
    CAN_MCR = MCR_INRQ; /* request initialization */
    while ((CAN_MSR & MSR_INAK) == 0u) {
    }
    CAN_BTR = BTR_NORMAL_VALID; /* valid timing, NORMAL mode (no loopback) */

    /* Acceptance filter: bank0, 32-bit mask mode, accept only ECU_RX_ID into
     * FIFO0. Without this the strict model (and real silicon) receive nothing. */
    CAN_FMR |= 1u;                         /* FINIT: filter init mode */
    CAN_FA1R &= ~1u;                       /* deactivate bank0 while configuring */
    CAN_FS1R |= 1u;                        /* bank0 = single 32-bit scale */
    CAN_FM1R &= ~1u;                       /* bank0 = mask mode */
    CAN_F0R1 = (ECU_RX_ID & 0x7FFu) << 21; /* id   = 0x111 in STID[31:21] */
    CAN_F0R2 = (ECU_RX_ID & 0x7FFu) << 21; /* mask = those id bits must match */
    CAN_FFA1R &= ~1u;                      /* bank0 -> FIFO0 */
    CAN_FA1R |= 1u;                        /* activate bank0 */
    CAN_FMR &= ~1u;                        /* leave filter init */

    CAN_MCR = 0u; /* leave init -> normal mode running */
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

/* UDSLib can_send hook: standard-ID classical frame via TX mailbox 0 (the
 * model puts it on the bus for the virtual tester to read). */
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

#ifdef BROKEN_NCR
/* Reproduce w1c/udslib issue #29: the reporter's port armed the ISO-TP N_Cr
 * timer from a clock that read 0 (an unset get_time_ms) while feeding
 * uds_tp_isotp_process() a real, large HAL_GetTick(). That clock-source
 * mismatch tears the multi-frame RX session down on the first process() tick
 * after the FirstFrame, so the ConsecutiveFrame is dropped and the ECU never
 * answers. We model it the same way: get_time_ms() returns 0 (used to ARM
 * N_Cr), and the loop feeds process() a large tick (below). */
static uint32_t get_time_ms(void) { return 0u; }
#else
static uint32_t get_time_ms(void) { return g_now_ms; }
#endif

static int isotp_send_adapter(struct uds_ctx *ctx, const uint8_t *data, uint16_t len)
{
    (void) ctx;
    return uds_isotp_send(&g_iso, data, len);
}

int main(void)
{
    rcc_init(); /* enable USART1/CAN1/GPIOA/AFIO clocks before touching them */
    uart_init();
    uart_puts("F103-UDS-ECU\n");

    can_init_normal();
    uart_puts("ECU_READY\n"); /* bxCAN normal-mode + filter configured */

    uds_tp_isotp_init(&g_iso, can_send, ECU_TX_ID, ECU_RX_ID, g_iso_tx_sdu, sizeof(g_iso_tx_sdu));
    uds_tp_isotp_set_fd(&g_iso, false); /* classical CAN */

    uds_config_t cfg = {
        .ecu_address = 0x10u,
        .get_time_ms = get_time_ms,
        .fn_tp_send = isotp_send_adapter,
        .rx_buffer = g_rx_buf,
        .rx_buffer_size = sizeof(g_rx_buf),
        .tx_buffer = g_tx_buf,
        .tx_buffer_size = sizeof(g_tx_buf),
        .p2_ms = 50u,
        .p2_star_ms = 2000u,
    };
    uds_ecu_app_fill_config(&cfg, "LABWIRED-F103-UDS");

    uds_ctx_t ctx;
    if (uds_init(&ctx, &cfg) != UDS_OK) {
        uart_puts("UDS_INIT_FAIL\n");
        for (;;) {
        }
    }

    /* Pure ECU loop: receive filtered frames, run ISO-TP/UDS, answer. The
     * virtual tester node drives the request over the bus. */
    for (;;) {
        can_frame_t f;
        if (can_poll(&f)) {
            uds_isotp_rx_callback(&g_iso, &ctx, f.id, f.data, f.len);
        }
        uds_process(&ctx);
#ifdef BROKEN_NCR
        /* Real, large tick into process() — vs the 0 that armed N_Cr. The
         * (large - 0) >= n_cr check fires immediately after the FirstFrame. */
        uds_tp_isotp_process(&g_iso, 100000u + g_now_ms);
#else
        uds_tp_isotp_process(&g_iso, g_now_ms);
#endif
        ++g_now_ms;
    }
}
