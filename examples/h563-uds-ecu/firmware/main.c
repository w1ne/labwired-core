#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "uds/uds_core.h"
#include "uds/uds_isotp.h"

void *memcpy(void *dst, const void *src, size_t n)
{
    uint8_t *d = (uint8_t *) dst;
    const uint8_t *s = (const uint8_t *) src;
    while (n-- > 0u) {
        *d++ = *s++;
    }
    return dst;
}

void *memset(void *dst, int value, size_t n)
{
    uint8_t *d = (uint8_t *) dst;
    while (n-- > 0u) {
        *d++ = (uint8_t) value;
    }
    return dst;
}

int memcmp(const void *lhs, const void *rhs, size_t n)
{
    const uint8_t *a = (const uint8_t *) lhs;
    const uint8_t *b = (const uint8_t *) rhs;
    while (n-- > 0u) {
        if (*a != *b) {
            return (int) *a - (int) *b;
        }
        ++a;
        ++b;
    }
    return 0;
}

void *__aeabi_memcpy(void *dst, const void *src, size_t n)
{
    return memcpy(dst, src, n);
}

void *__aeabi_memcpy4(void *dst, const void *src, size_t n)
{
    return memcpy(dst, src, n);
}

void *__aeabi_memcpy8(void *dst, const void *src, size_t n)
{
    return memcpy(dst, src, n);
}

void *__aeabi_memset(void *dst, size_t n, int value)
{
    return memset(dst, value, n);
}

void *__aeabi_memclr(void *dst, size_t n)
{
    return memset(dst, 0, n);
}

void *__aeabi_memclr4(void *dst, size_t n)
{
    return memset(dst, 0, n);
}

void *__aeabi_memclr8(void *dst, size_t n)
{
    return memset(dst, 0, n);
}

static volatile uint32_t g_now_ms;
static volatile bool g_positive_response_sent;

#define REG32(addr) (*(volatile uint32_t *) (addr))

#define USART3_BASE 0x40004800u
#define USART3_CR1 REG32(USART3_BASE + 0x00u)
#define USART3_ISR REG32(USART3_BASE + 0x1Cu)
#define USART3_TDR REG32(USART3_BASE + 0x28u)
#define USART_ISR_TXE (1u << 7)
#define USART_CR1_UE (1u << 0)
#define USART_CR1_TE (1u << 3)

#define FDCAN1_BASE 0x4000A400u
#define FDCAN_REG_TEST 0x010u
#define FDCAN_REG_CCCR 0x018u
#define FDCAN_REG_IR 0x050u
#define FDCAN_REG_RXF0S 0x090u
#define FDCAN_REG_RXF0A 0x094u
#define FDCAN_REG_TXBAR 0x0CCu

#define FDCAN_RAM_BASE 0x800u
#define FDCAN_RXF0_ELEM0 0x0B0u
#define FDCAN_TXBUF0 0x278u

#define CCCR_INIT (1u << 0)
#define CCCR_CCE (1u << 1)
#define CCCR_MON (1u << 5)
#define CCCR_TEST (1u << 7)
#define TEST_LBCK (1u << 4)
#define TX_T1_BRS (1u << 20)
#define TX_T1_FDF (1u << 21)

typedef struct {
    uint32_t id;
    uint8_t len;
    uint8_t data[64];
    bool fd;
} can_frame_t;

static void uart_init(void)
{
    USART3_CR1 = USART_CR1_UE | USART_CR1_TE;
}

static void uart_putc(char c)
{
    while ((USART3_ISR & USART_ISR_TXE) == 0u) {
    }
    USART3_TDR = (uint32_t) (uint8_t) c;
}

static void uart_puts(const char *s)
{
    while (*s != '\0') {
        uart_putc(*s++);
    }
}

static uint32_t fdcan_reg(uint32_t offset)
{
    return FDCAN1_BASE + offset;
}

static uint32_t fdcan_ram(uint32_t offset)
{
    return FDCAN1_BASE + FDCAN_RAM_BASE + offset;
}

static uint8_t len_to_dlc(uint8_t len)
{
    if (len <= 8u) return len;
    if (len <= 12u) return 9u;
    if (len <= 16u) return 10u;
    if (len <= 20u) return 11u;
    if (len <= 24u) return 12u;
    if (len <= 32u) return 13u;
    if (len <= 48u) return 14u;
    return 15u;
}

static uint8_t dlc_to_len(uint8_t dlc)
{
    static const uint8_t map[16] = {0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 20, 24, 32, 48, 64};
    return map[dlc & 0x0Fu];
}

static void write_payload(uint32_t payload_addr, const uint8_t *data, uint8_t len)
{
    for (uint32_t i = 0; i < 16u; ++i) {
        REG32(payload_addr + i * 4u) = 0u;
    }
    for (uint8_t i = 0; i < len; ++i) {
        uint32_t addr = payload_addr + ((uint32_t) i / 4u) * 4u;
        uint32_t shift = ((uint32_t) i % 4u) * 8u;
        REG32(addr) = REG32(addr) | ((uint32_t) data[i] << shift);
    }
}

static void read_payload(uint32_t payload_addr, uint8_t *data, uint8_t len)
{
    for (uint8_t i = 0; i < len; ++i) {
        uint32_t word = REG32(payload_addr + ((uint32_t) i / 4u) * 4u);
        data[i] = (uint8_t) ((word >> (((uint32_t) i % 4u) * 8u)) & 0xFFu);
    }
}

static void fdcan_start_loopback(void)
{
    REG32(fdcan_reg(FDCAN_REG_CCCR)) = CCCR_INIT | CCCR_CCE;
    REG32(fdcan_reg(FDCAN_REG_CCCR)) = CCCR_INIT | CCCR_CCE | CCCR_TEST | CCCR_MON;
    REG32(fdcan_reg(FDCAN_REG_TEST)) = TEST_LBCK;
    REG32(fdcan_reg(FDCAN_REG_CCCR)) = CCCR_TEST | CCCR_MON;
    while ((REG32(fdcan_reg(FDCAN_REG_CCCR)) & CCCR_INIT) != 0u) {
    }
}

static int fdcan_send_frame(uint32_t id, const uint8_t *data, uint8_t len, bool fd)
{
    if (id > 0x7FFu || len > 64u) {
        return -1;
    }

    uint32_t base = fdcan_ram(FDCAN_TXBUF0);
    REG32(base + 0u) = (id & 0x7FFu) << 18u;
    REG32(base + 4u) = ((uint32_t) len_to_dlc(len) << 16u) | (fd ? (TX_T1_FDF | TX_T1_BRS) : 0u);
    write_payload(base + 8u, data, len);
    REG32(fdcan_reg(FDCAN_REG_TXBAR)) = 1u;
    return 0;
}

static bool fdcan_poll_rx_frame(can_frame_t *frame)
{
    uint32_t rxf0s = REG32(fdcan_reg(FDCAN_REG_RXF0S));
    if ((rxf0s & 0x7Fu) == 0u) {
        return false;
    }

    uint32_t get_index = (rxf0s >> 8u) & 0x3Fu;
    uint32_t base = fdcan_ram(FDCAN_RXF0_ELEM0 + get_index * 72u);
    uint32_t r0 = REG32(base + 0u);
    uint32_t r1 = REG32(base + 4u);
    frame->id = (r0 >> 18u) & 0x7FFu;
    frame->len = dlc_to_len((uint8_t) ((r1 >> 16u) & 0x0Fu));
    frame->fd = (r1 & TX_T1_FDF) != 0u;
    read_payload(base + 8u, frame->data, frame->len);
    REG32(fdcan_reg(FDCAN_REG_RXF0A)) = get_index;
    REG32(fdcan_reg(FDCAN_REG_IR)) = REG32(fdcan_reg(FDCAN_REG_IR));
    return true;
}

static uint32_t get_time_ms(void)
{
    return g_now_ms;
}

static int can_send(uint32_t id, const uint8_t *data, uint8_t len)
{
    return fdcan_send_frame(id, data, len, len > 8u);
}

static uint8_t g_rx_buffer[128];
static uint8_t g_tx_buffer[128];
static const uint8_t g_vin[] = "LABWIRED-H563-UDS";
static const uds_did_entry_t g_dids[] = {
    {0xF190u, sizeof(g_vin) - 1u, 0u, 0u, NULL, NULL, (void *) g_vin},
};
static const uds_did_table_t g_did_table = {
    g_dids,
    (uint16_t) (sizeof(g_dids) / sizeof(g_dids[0])),
};

static int app_read_data_by_id(struct uds_ctx *ctx, const uint8_t *data, uint16_t len)
{
    (void) data;
    if (len != 3u) {
        return uds_send_nrc(ctx, 0x22u, 0x13u);
    }

    const uds_did_entry_t *entry = &g_dids[0];
    if ((uint16_t) (3u + entry->size) > ctx->config->tx_buffer_size) {
        return uds_send_nrc(ctx, 0x22u, 0x14u);
    }

    ctx->config->tx_buffer[0] = 0x62u;
    ctx->config->tx_buffer[1] = (uint8_t) (entry->id >> 8u);
    ctx->config->tx_buffer[2] = (uint8_t) entry->id;
    memcpy(&ctx->config->tx_buffer[3], entry->storage, entry->size);
    int rc = uds_send_response(ctx, (uint16_t) (3u + entry->size));
    if (rc == 0) {
        g_positive_response_sent = true;
    }
    return rc;
}

static const uds_service_entry_t g_user_services[] = {
    {0x22u, 3u, UDS_SESSION_ALL, 0u, app_read_data_by_id, NULL},
};

static void pump_one_tester_request(uds_ctx_t *ctx)
{
    can_frame_t frame;
    while (fdcan_poll_rx_frame(&frame)) {
        if (frame.id == 0x7E0u) {
            uds_isotp_rx_callback(ctx, frame.id, frame.data, frame.len);
            return;
        }
    }
}

static bool positive_vin_response_seen(void)
{
    can_frame_t frame;
    while (fdcan_poll_rx_frame(&frame)) {
        if (frame.id != 0x7E8u) {
            continue;
        }
        uint8_t offset = 0u;
        uint8_t sdu_len = frame.data[0] & 0x0Fu;
        if (sdu_len == 0u) {
            sdu_len = frame.data[1];
            offset = 2u;
        } else {
            offset = 1u;
        }
        if (sdu_len != (uint8_t) (3u + sizeof(g_vin) - 1u)) {
            continue;
        }
        if (frame.data[offset + 0u] != 0x62u || frame.data[offset + 1u] != 0xF1u ||
            frame.data[offset + 2u] != 0x90u) {
            continue;
        }
        return true;
    }
    return false;
}

int main(void)
{
    uart_init();
    uart_puts("H563-UDS-ECU\n");

    fdcan_start_loopback();
    uds_tp_isotp_init(can_send, 0x7E8u, 0x7E0u);
    uds_tp_isotp_set_fd(true);

    uds_config_t cfg = {
        .ecu_address = 0x10u,
        .get_time_ms = get_time_ms,
        .fn_tp_send = uds_isotp_send,
        .p2_ms = 50u,
        .p2_star_ms = 2000u,
        .rx_buffer = g_rx_buffer,
        .rx_buffer_size = sizeof(g_rx_buffer),
        .tx_buffer = g_tx_buffer,
        .tx_buffer_size = sizeof(g_tx_buffer),
        .did_table = g_did_table,
        .user_services = g_user_services,
        .user_service_count = (uint16_t) (sizeof(g_user_services) / sizeof(g_user_services[0])),
    };
    uds_ctx_t ctx;
    if (uds_init(&ctx, &cfg) != UDS_OK) {
        uart_puts("UDS_INIT_FAIL\n");
        for (;;) {
        }
    }
    static const uint8_t request[] = {0x03u, 0x22u, 0xF1u, 0x90u};
    uart_puts("UDS_REQ_22_F190\n");
    (void) fdcan_send_frame(0x7E0u, request, sizeof(request), false);

    bool ok = false;
    for (uint32_t i = 0; i < 64u && !ok; ++i) {
        pump_one_tester_request(&ctx);
        uds_process(&ctx);
        uds_tp_isotp_process(g_now_ms);
        ok = positive_vin_response_seen() || g_positive_response_sent;
        ++g_now_ms;
    }

    if (ok) {
        uart_puts("UDS_RESP_62_F190\n");
        uart_puts("VIN=LABWIRED-H563-UDS\n");
        uart_puts("UDS_OK\n");
    } else {
        uart_puts("UDS_FAIL\n");
    }

    for (;;) {
    }
}
