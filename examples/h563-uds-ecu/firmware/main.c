#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "uds/uds_core.h"
#include "uds/uds_isotp.h"
#include "uds_ecu_app.h"

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

#define REG32(addr) (*(volatile uint32_t *) (addr))

#define USART3_BASE 0x40004800u
#define USART3_CR1 REG32(USART3_BASE + 0x00u)
#define USART3_ISR REG32(USART3_BASE + 0x1Cu)
#define USART3_TDR REG32(USART3_BASE + 0x28u)
#define USART_ISR_TXE (1u << 7)
#define USART_CR1_UE (1u << 0)
#define USART_CR1_TE (1u << 3)

/* RCC: FDCAN1 hangs off APB1H and is clock-gated out of reset (RM0481 §11.8.38).
 * Its register surface reads 0 / ignores writes until RCC_APB1HENR.FDCAN1EN is
 * set, so the clock MUST be enabled before any FDCAN access — the simulator
 * models this gate (chip YAML `clock: { reg: apb1henr, bit: 9 }`). */
#define RCC_BASE 0x44020C00u
#define RCC_APB1HENR REG32(RCC_BASE + 0x0A0u)
#define RCC_APB1HENR_FDCAN1EN (1u << 9)

#define FDCAN1_BASE 0x4000A400u
#define FDCAN_REG_TEST 0x010u
#define FDCAN_REG_CCCR 0x018u
#define FDCAN_REG_IR 0x050u
#define FDCAN_REG_RXF0S 0x090u
#define FDCAN_REG_RXF0A 0x094u
#define FDCAN_REG_TXBRP 0x0C8u
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

void uds_ecu_app_log(const char *msg) { uart_puts(msg); }

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

static void fdcan_start(void)
{
    /* Enable the FDCAN1 bus-interface clock before touching its registers;
     * without this the peripheral is held unclocked and every access is a no-op
     * (reads 0), which silently breaks ISO-TP RX on real silicon and in-sim. */
    RCC_APB1HENR |= RCC_APB1HENR_FDCAN1EN;
    (void) RCC_APB1HENR; /* read-back: ensure the enable lands before use */

    REG32(fdcan_reg(FDCAN_REG_CCCR)) = CCCR_INIT | CCCR_CCE;
    REG32(fdcan_reg(FDCAN_REG_TEST)) = 0u;
    REG32(fdcan_reg(FDCAN_REG_CCCR)) = 0u;
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

/* tx_buffer and the ISO-TP SDU buffer are sized > 512 so the >512-byte DID
 * 0xF1A0 calibration block (62 F1 A0 + 600 bytes = 603 bytes) is built in
 * tx_buffer and then streamed as a multi-frame ISO-TP response (use case 1). */
static uds_isotp_ctx_t g_iso;
static uint8_t g_iso_tx_sdu[768];
static uint8_t g_rx_buffer[128];
static uint8_t g_tx_buffer[768];

static int isotp_send_adapter(struct uds_ctx *ctx, const uint8_t *data, uint16_t len)
{
    (void) ctx;
    return uds_isotp_send(&g_iso, data, len);
}

/* fn_tx_complete hook (udslib v2.0.0, use case 2): TXBRP bit 0 stays set while
 * TX buffer 0 still holds a pending request; it clears once the FDCAN has
 * arbitrated the frame onto the wire. udslib polls this once per uds_process
 * tick (bounded by reset_tx_wait_ms) and holds fn_reset until it returns true,
 * so SCB SYSRESETREQ cannot reboot before the 0x51 response drains (udslib
 * #88). */
static bool can_tx_complete(struct uds_ctx *ctx)
{
    (void) ctx;
    return (REG32(fdcan_reg(FDCAN_REG_TXBRP)) & 0x1u) == 0u;
}

int main(void)
{
    uart_init();
    uart_puts("H563-UDS-ECU\n");

    fdcan_start();
    uds_tp_isotp_init(&g_iso, can_send, 0x7E8u, 0x7E0u, g_iso_tx_sdu, sizeof(g_iso_tx_sdu));
    uds_tp_isotp_set_fd(&g_iso, true);
    uart_puts("ECU_READY\n");

    uds_config_t cfg = {
        .ecu_address = 0x10u,
        .get_time_ms = get_time_ms,
        .fn_tp_send = isotp_send_adapter,
        .rx_buffer = g_rx_buffer,
        .rx_buffer_size = sizeof(g_rx_buffer),
        .tx_buffer = g_tx_buffer,
        .tx_buffer_size = sizeof(g_tx_buffer),
        .p2_ms = 50u,
        .p2_star_ms = 2000u,
        .fn_tx_complete = can_tx_complete, /* gate 0x11 reset on frame-on-wire */
        .reset_tx_wait_ms = 20u,           /* budget before forcing the reset */
    };
    uds_ecu_app_fill_config(&cfg, "LABWIRED-H563-UDS");

    uds_ctx_t ctx;
    if (uds_init(&ctx, &cfg) != UDS_OK) {
        uart_puts("UDS_INIT_FAIL\n");
        for (;;) {
        }
    }

    for (;;) {
        can_frame_t frame;
        if (fdcan_poll_rx_frame(&frame)) {
            uds_isotp_rx_callback(&g_iso, &ctx, frame.id, frame.data, frame.len);
        }
        uds_process(&ctx);
        uds_tp_isotp_process(&g_iso, g_now_ms);
        ++g_now_ms;
    }
}
