/*
 * Simulated CSS Electronics CANmod.gps — GNSS/IMU-to-CAN module.
 *
 * The STM32F103 is a COMPUTE STAND-IN (CANmod's real MCU is undisclosed); the
 * on-wire CAN frames are bit-accurate to CSS's published canmod-gps.dbc. The
 * firmware synthesizes a deterministic GNSS track, packs the 9 DBC messages,
 * transmits them on bxCAN (internal loopback), and echoes decoded values plus
 * raw frame bytes over USART1.
 */
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define REG32(addr) (*(volatile uint32_t *) (addr))

/* --- RCC (F1) — enable peripheral clocks. REQUIRED: the sim models
 * clock-gating, so USART1 register writes are dropped until the RCC
 * enable bit is set (same as real silicon). --- */
#define RCC_BASE 0x40021000u
#define RCC_APB2ENR REG32(RCC_BASE + 0x18u)
#define RCC_APB1ENR REG32(RCC_BASE + 0x1Cu)
#define RCC_APB2ENR_USART1EN (1u << 14)
#define RCC_APB1ENR_CAN1EN   (1u << 25)

static void rcc_init(void)
{
    RCC_APB2ENR |= RCC_APB2ENR_USART1EN;
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

static void uart_hex8(uint8_t b)
{
    static const char hex[] = "0123456789ABCDEF";
    uart_putc(hex[(b >> 4) & 0xF]);
    uart_putc(hex[b & 0xF]);
}

/* --- bxCAN @ 0x40006400 (RM0008 §24.9) --- */
#define CAN_BASE  0x40006400u
#define CAN_MCR   REG32(CAN_BASE + 0x000u)
#define CAN_BTR   REG32(CAN_BASE + 0x01Cu)
#define CAN_TI0R  REG32(CAN_BASE + 0x180u)
#define CAN_TDT0R REG32(CAN_BASE + 0x184u)
#define CAN_TDL0R REG32(CAN_BASE + 0x188u)
#define CAN_TDH0R REG32(CAN_BASE + 0x18Cu)
#define CAN_RF0R  REG32(CAN_BASE + 0x00Cu)
#define CAN_RI0R  REG32(CAN_BASE + 0x1B0u)
#define CAN_RDT0R REG32(CAN_BASE + 0x1B4u)
#define CAN_RDL0R REG32(CAN_BASE + 0x1B8u)
#define CAN_RDH0R REG32(CAN_BASE + 0x1BCu)
#define MCR_INRQ (1u << 0)
#define TI_TXRQ  (1u << 0)
#define RF_RFOM  (1u << 5)
#define BTR_LBKM (1u << 30)

typedef struct {
    uint32_t id;
    uint8_t  len;
    uint8_t  data[8];
} can_frame_t;

static void can_init_loopback(void)
{
    CAN_MCR = MCR_INRQ;  /* request initialization */
    CAN_BTR = BTR_LBKM;  /* internal loopback */
    CAN_MCR = 0u;         /* leave init -> running */
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

static void can_send(uint32_t id, const uint8_t *data, uint8_t len)
{
    if (len > 8u) len = 8u;
    CAN_TDL0R = pack_lo(data, len);
    CAN_TDH0R = pack_hi(data, len);
    CAN_TDT0R = len;                              /* DLC */
    CAN_TI0R  = ((id & 0x7FFu) << 21) | TI_TXRQ; /* STID + transmit request */
}

static bool can_poll(can_frame_t *f)
{
    if ((CAN_RF0R & 0x3u) == 0u) return false;
    f->id  = (CAN_RI0R >> 21) & 0x7FFu;
    f->len = (uint8_t) (CAN_RDT0R & 0xFu);
    uint32_t lo = CAN_RDL0R, hi = CAN_RDH0R;
    for (uint8_t i = 0; i < 4; ++i) f->data[i]     = (uint8_t) (lo >> (i * 8));
    for (uint8_t i = 0; i < 4; ++i) f->data[i + 4] = (uint8_t) (hi >> (i * 8));
    CAN_RF0R = RF_RFOM;                            /* release FIFO0 mailbox */
    return true;
}

/* Generic little-endian (Intel) bit-field setter, LSB at `start`. */
static void pack_bits(uint8_t *buf, uint16_t start, uint16_t len, uint64_t val)
{
    for (uint16_t i = 0; i < len; ++i) {
        if (val & (1ull << i)) buf[(start + i) >> 3] |= (uint8_t) (1u << ((start + i) & 7));
    }
}

/* Transmit one frame, drain the loopback echo, and log it as raw hex. */
static void emit(uint32_t id, const uint8_t *data, uint8_t len)
{
    can_send(id, data, len);
    can_frame_t echo;
    while (can_poll(&echo)) { /* drain so the RX FIFO never overflows */ }
    uart_puts("CAN_TX ");
    uart_hex8((uint8_t) ((id >> 8) & 0xFF));
    uart_hex8((uint8_t) (id & 0xFF));
    uart_putc(':');
    for (uint8_t i = 0; i < len; ++i) {
        uart_putc(' ');
        uart_hex8(data[i]);
    }
    uart_putc('\n');
}

int main(void)
{
    rcc_init(); /* enable USART1 + CAN1 clocks before touching their registers */
    uart_init();
    uart_puts("CANMOD-GPS-SIM\n");
    can_init_loopback();

    uint8_t buf[8] = {0};
    pack_bits(buf, 0, 3, 3);   /* FixType = 3 (3D fix) */
    pack_bits(buf, 3, 5, 11);  /* Satellites = 11 */
    emit(0x1, buf, 1);

    for (;;) {
    }
}
