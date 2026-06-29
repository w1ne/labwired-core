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

/* Print a fixed-point decimal: value = whole.fraction, `frac_digits` wide. */
static void uart_udec(uint32_t v)
{
    char tmp[10];
    int n = 0;
    if (v == 0) { uart_putc('0'); return; }
    while (v && n < 10) { tmp[n++] = (char) ('0' + (v % 10u)); v /= 10u; }
    while (n--) uart_putc(tmp[n]);
}
static void uart_fixed(uint32_t scaled, uint32_t divisor, uint32_t frac_digits)
{
    uart_udec(scaled / divisor);
    uart_putc('.');
    uint32_t frac = scaled % divisor;
    /* zero-pad the fraction to frac_digits */
    for (uint32_t p = 1, d = 1; d < frac_digits; ++d) { p *= 10; if (frac < p) uart_putc('0'); }
    uart_udec(frac);
}

/* canmod-gps.dbc raw-field encoders (scale/offset already applied to raw ints). */
#define RAW_LAT_BASE 145676100u   /* (55.676100 + 90) / 1e-6 */
#define RAW_LON_BASE 192568300u   /* (12.568300 + 180) / 1e-6, tick 0 */
#define LON_STEP     100u         /* +0.000100 deg/tick = +100 in 1e-6 units */

static uint32_t g_frames;

static void broadcast_tick(uint32_t tick)
{
    uint8_t b[8];
    uint32_t raw_lon = RAW_LON_BASE + tick * LON_STEP;

    /* 0x1 gnss_status */
    for (int i = 0; i < 8; ++i) b[i] = 0;
    pack_bits(b, 0, 3, 3);            /* FixType = 3 */
    pack_bits(b, 3, 5, 11);           /* Satellites = 11 */
    emit(0x1, b, 1); g_frames++;

    /* 0x2 gnss_time */
    for (int i = 0; i < 8; ++i) b[i] = 0;
    pack_bits(b, 0, 1, 1);            /* TimeValid */
    pack_bits(b, 1, 1, 1);            /* TimeConfirmed */
    pack_bits(b, 8, 40, (uint64_t) tick); /* Epoch delta (raw, scale 0.001) */
    emit(0x2, b, 6); g_frames++;

    /* 0x3 gnss_pos */
    for (int i = 0; i < 8; ++i) b[i] = 0;
    pack_bits(b, 0, 1, 1);            /* PositionValid */
    pack_bits(b, 1, 28, RAW_LAT_BASE);
    pack_bits(b, 29, 29, raw_lon);
    pack_bits(b, 58, 6, 3);          /* PositionAccuracy = 3 m */
    emit(0x3, b, 8); g_frames++;

    /* 0x4 gnss_altitude (12.0 m -> raw (12.0+6000)/0.1 = 60120) */
    for (int i = 0; i < 8; ++i) b[i] = 0;
    pack_bits(b, 0, 1, 1);
    pack_bits(b, 1, 18, 60120u);
    pack_bits(b, 19, 13, 5);
    emit(0x4, b, 4); g_frames++;

    /* 0x5 gnss_attitude (roll 0, pitch 0, heading 90.0 -> raw 900) */
    for (int i = 0; i < 8; ++i) b[i] = 0;
    pack_bits(b, 0, 1, 1);
    pack_bits(b, 1, 12, 1800u);      /* Roll 0 -> (0+180)/0.1 */
    pack_bits(b, 22, 12, 900u);      /* Pitch 0 -> (0+90)/0.1 */
    pack_bits(b, 43, 12, 900u);      /* Heading 90 -> 90/0.1 */
    emit(0x5, b, 8); g_frames++;

    /* 0x6 gnss_odo */
    for (int i = 0; i < 8; ++i) b[i] = 0;
    pack_bits(b, 0, 1, 1);
    pack_bits(b, 1, 22, (uint64_t) (tick * 10u)); /* DistanceTrip grows */
    emit(0x6, b, 8); g_frames++;

    /* 0x7 gnss_speed (10.000 m/s -> raw 10000) */
    for (int i = 0; i < 8; ++i) b[i] = 0;
    pack_bits(b, 0, 1, 1);
    pack_bits(b, 1, 20, 10000u);
    pack_bits(b, 21, 19, 100u);      /* SpeedAccuracy 0.1 m/s */
    emit(0x7, b, 5); g_frames++;

    /* 0x8 gnss_geofence */
    for (int i = 0; i < 8; ++i) b[i] = 0;
    pack_bits(b, 0, 1, 1);
    pack_bits(b, 1, 2, 1);           /* FenceCombined = Inside */
    emit(0x8, b, 2); g_frames++;

    /* 0x9 gnss_imu (Az ~1g -> (9.875+64)/0.125 = 591; gyro centered) */
    for (int i = 0; i < 8; ++i) b[i] = 0;
    pack_bits(b, 0, 1, 1);
    pack_bits(b, 1, 10, 512u);       /* Ax 0 -> (0+64)/0.125 */
    pack_bits(b, 11, 10, 512u);      /* Ay 0 */
    pack_bits(b, 21, 10, 591u);      /* Az ~1g */
    pack_bits(b, 31, 11, 1024u);     /* gyro X 0 -> (0+256)/0.25 */
    pack_bits(b, 42, 11, 1024u);     /* gyro Y 0 */
    pack_bits(b, 53, 11, 1024u);     /* gyro Z 0 */
    emit(0x9, b, 8); g_frames++;

    /* Human-readable beat for the playground UART console. */
    uart_puts("FIX type=3 sats=11\n");
    uart_puts("POS lat=");
    uart_fixed(RAW_LAT_BASE - 90000000u, 1000000u, 6); /* lat back to degrees */
    uart_puts(" lon=");
    uart_fixed(raw_lon - 180000000u, 1000000u, 6);
    uart_putc('\n');
    uart_puts("SPEED 10.000 m/s\n");
}

int main(void)
{
    rcc_init(); /* enable USART1 + CAN1 clocks before touching their registers */
    uart_init();
    uart_puts("CANMOD-GPS-SIM\n");
    can_init_loopback();

    const uint32_t TICKS = 5u;
    for (uint32_t t = 0; t < TICKS; ++t) broadcast_tick(t);

    uart_puts("CANMOD_OK frames=");
    uart_udec(g_frames);
    uart_putc('\n');
    for (;;) {
    }
}
