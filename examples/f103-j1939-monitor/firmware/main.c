/*
 * f103-j1939-monitor — RX-only J1939 node on bxCAN, silicon-correct
 * normal-mode (NOT loopback) with an accept-all acceptance filter so both
 * standard and 29-bit extended frames reach FIFO0. Fed by the can-player
 * external device replaying a captured J1939 bus trace (candump .log) over
 * the simulated bus; this firmware only listens and decodes.
 *
 * UART/CAN init lifted from examples/f103-uds-ecu/firmware/main.c (RCC
 * clock-gating idiom + normal-mode bxCAN bring-up); hex/decimal print
 * helpers lifted from examples/canmod-gps-sim/firmware/main.c.
 */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define REG32(addr) (*(volatile uint32_t *) (addr))

/* --- RCC (F1) — enable peripheral clocks. REQUIRED: the sim models
 * clock-gating, so USART1/CAN1/GPIOA/AFIO register writes are dropped until
 * the matching RCC enable bit is set (same as real silicon). --- */
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

/* Uppercase hex helpers (assertions depend on uppercase digits). */
static void uart_hex8(uint8_t b)
{
    static const char hex[] = "0123456789ABCDEF";
    uart_putc(hex[(b >> 4) & 0xFu]);
    uart_putc(hex[b & 0xFu]);
}
static void uart_hex16(uint16_t v)
{
    uart_hex8((uint8_t) (v >> 8));
    uart_hex8((uint8_t) v);
}

/* Unsigned decimal, no leading zeros. */
static void uart_dec(uint32_t v)
{
    char tmp[10];
    int n = 0;
    if (v == 0u) {
        uart_putc('0');
        return;
    }
    while (v && n < 10) {
        tmp[n++] = (char) ('0' + (v % 10u));
        v /= 10u;
    }
    while (n--) uart_putc(tmp[n]);
}

/* --- bxCAN @ 0x40006400 (RM0008 §24.9) --- */
#define CAN_BASE 0x40006400u
#define CAN_MCR REG32(CAN_BASE + 0x000u)
#define CAN_MSR REG32(CAN_BASE + 0x004u)
#define CAN_RF0R REG32(CAN_BASE + 0x00Cu)
#define CAN_BTR REG32(CAN_BASE + 0x01Cu)
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
#define RF_RFOM (1u << 5)
#define RI_IDE (1u << 2)
/* Valid bit timing (TS1=12, TS2=5, BRP=9) — silicon-captured working value
 * (same as f103-uds-ecu); a degenerate BTR with zero segments would bus-off
 * on the real chip and in the model. */
#define BTR_NORMAL_VALID 0x00DC0009u

static void can_init(void)
{
    CAN_MCR = MCR_INRQ; /* request initialization */
    while ((CAN_MSR & MSR_INAK) == 0u) {
    }
    CAN_BTR = BTR_NORMAL_VALID; /* valid timing, NORMAL mode (no loopback) */

    /* Acceptance filter: bank0, 32-bit mask mode, mask = 0 => accept every
     * frame (standard or extended, any id) into FIFO0. This is a monitor —
     * it must see all traffic on the simulated bus, not just one PGN. */
    CAN_FMR |= 1u;    /* FINIT: filter init mode */
    CAN_FA1R &= ~1u;  /* deactivate bank0 while configuring */
    CAN_FS1R |= 1u;   /* bank0 = single 32-bit scale */
    CAN_FM1R &= ~1u;  /* bank0 = mask mode */
    CAN_F0R1 = 0u;    /* id   = don't care */
    CAN_F0R2 = 0u;    /* mask = don't care about any bit -> accept all */
    CAN_FFA1R &= ~1u; /* bank0 -> FIFO0 */
    CAN_FA1R |= 1u;   /* activate bank0 */
    CAN_FMR &= ~1u;   /* leave filter init */

    CAN_MCR = 0u; /* leave init -> normal mode running */
    while ((CAN_MSR & MSR_INAK) != 0u) {
    }
}

typedef struct {
    uint32_t id; /* right-aligned: 11-bit standard or 29-bit extended */
    uint8_t  len;
    uint8_t  data[8];
} can_frame_t;

static bool can_poll(can_frame_t *f)
{
    if ((CAN_RF0R & 0x3u) == 0u) return false;

    uint32_t rir = CAN_RI0R;
    if ((rir & RI_IDE) != 0u) {
        f->id = rir >> 3;  /* extended 29-bit id, right-aligned */
    } else {
        f->id = rir >> 21; /* standard 11-bit id, right-aligned */
    }
    f->len = (uint8_t) (CAN_RDT0R & 0xFu); /* DLC field */

    uint32_t lo = CAN_RDL0R, hi = CAN_RDH0R;
    for (uint8_t i = 0; i < 4; ++i) f->data[i] = (uint8_t) (lo >> (i * 8));
    for (uint8_t i = 0; i < 4; ++i) f->data[i + 4] = (uint8_t) (hi >> (i * 8));

    CAN_RF0R = RF_RFOM; /* release FIFO0 mailbox */
    return true;
}

/*
 * f103-j1939-monitor — RX-only J1939 node: reassembles BAM transport
 * sessions PER SOURCE ADDRESS and tabulates DM1 lamp status. Fed by the
 * can-player external device replaying a captured J1939 bus trace. Signal
 * layouts hand-decoded from public J1939-71 tables; no DBC involved.
 */

#define PGN_TP_CM  0xEC00u
#define PGN_TP_DT  0xEB00u
#define PGN_DM1    0xFECAu
#define PGN_ENGCFG 0xFEE3u

typedef struct {
    uint8_t  in_use;
    uint8_t  sa;
    uint32_t pgn;
    uint16_t size;
    uint8_t  num_pkts;
    uint8_t  got;          /* count of received DT packets            */
    uint8_t  buf[64];      /* BAM payload cap for this monitor        */
} bam_sess_t;

#define MAX_SESS 4u
static bam_sess_t sess[MAX_SESS];

/* per-SA keying — THE fix.
 *
 * Deliberately scoped: eviction/timeouts are NOT implemented, so an orphan
 * TP.DT (one with no matching TP.CM open, or one arriving after its session
 * already completed and was freed) can walk into the fallback slot-0 return
 * below and pin a session that never gets reclaimed. This capture only has
 * 2 BAM sources against MAX_SESS=4, so the gap never bites here — a busier
 * bus (or a malicious/broken sender) could exhaust all 4 slots.
 */
static bam_sess_t *sess_for(uint8_t sa)
{
    uint32_t i;
    for (i = 0u; i < MAX_SESS; i++)
        if (sess[i].in_use && sess[i].sa == sa) return &sess[i];
    for (i = 0u; i < MAX_SESS; i++)
        if (!sess[i].in_use) { sess[i].in_use = 1u; sess[i].sa = sa; return &sess[i]; }
    return &sess[0];
}

static uint32_t dm1_seen[8];   /* 256-bit SA bitmap */
static uint8_t  dm1_sources;
static uint32_t rx_total;

static void print_bam(const bam_sess_t *s)
{
    uart_puts("BAM sa=");   uart_hex8((uint8_t) s->sa);
    uart_puts(" pgn=");     uart_hex16((uint16_t) s->pgn);
    uart_puts(" len=");     uart_dec(s->size);
    uart_puts(" data=");
    for (uint32_t i = 0u; i < 8u && i < s->size; i++) uart_hex8(s->buf[i]);
    uart_puts("\r\n");
    if (s->pgn == PGN_ENGCFG && s->size >= 2u) {
        /* SPN 188 engine speed at idle: bytes 0-1 LE, 0.125 rpm/bit */
        uint32_t rpm = ((uint32_t) s->buf[0] | ((uint32_t) s->buf[1] << 8)) / 8u;
        uart_puts("ENGINE idle_rpm="); uart_dec(rpm); uart_puts("\r\n");
    }
}

static void on_frame(uint32_t id, const uint8_t *d, uint32_t len)
{
    uint8_t  sa = (uint8_t) (id & 0xFFu);
    uint8_t  pf = (uint8_t) ((id >> 16) & 0xFFu);
    uint8_t  ps = (uint8_t) ((id >> 8) & 0xFFu);
    uint32_t pgn = (pf < 0xF0u) ? ((uint32_t) pf << 8) : (((uint32_t) pf << 8) | ps);

    rx_total++;
    if ((rx_total % 1000u) == 0u) {
        uart_puts("RX total="); uart_dec(rx_total); uart_puts("\r\n");
    }

    if (pgn == PGN_TP_CM && len == 8u && d[0] == 0x20u) {        /* BAM open */
        bam_sess_t *s = sess_for(sa);
        s->pgn      = (uint32_t) d[5] | ((uint32_t) d[6] << 8) | ((uint32_t) d[7] << 16);
        s->size     = (uint16_t) ((uint16_t) d[1] | ((uint16_t) d[2] << 8));
        s->num_pkts = d[3];
        s->got      = 0u;
        /* Clamp to this monitor's BAM payload cap. For a BAM >64 bytes the
         * printed "len=" is the CLAMPED size (sizeof s->buf), not the size
         * TP.CM actually announced — bytes past the cap are dropped by the
         * reassembly loop below and never appear in the printed payload. */
        if (s->size > sizeof s->buf) s->size = (uint16_t) sizeof s->buf;
    } else if (pgn == PGN_TP_DT && len == 8u) {
        bam_sess_t *s = sess_for(sa);
        uint8_t seq = d[0];
        if (s->num_pkts != 0u && seq >= 1u && seq <= s->num_pkts) {
            uint32_t off = (uint32_t) (seq - 1u) * 7u;
            for (uint32_t i = 0u; i < 7u && off + i < s->size; i++) s->buf[off + i] = d[1u + i];
            s->got++;
            if (s->got == s->num_pkts) {
                print_bam(s);
                s->in_use = 0u; s->num_pkts = 0u;
            }
        }
    } else if (pgn == PGN_DM1 && len >= 2u) {
        uint32_t word = sa >> 5, bit = 1u << (sa & 31u);
        if ((dm1_seen[word] & bit) == 0u) {
            dm1_seen[word] |= bit;
            dm1_sources++;
            uart_puts("DM1 sa="); uart_hex8(sa);
            uart_puts(" lamps="); uart_hex8(d[0]); uart_puts("\r\n");
            uart_puts("DM1 sources="); uart_dec(dm1_sources); uart_puts("\r\n");
        }
    }
}

int main(void)
{
    rcc_init(); /* enable USART1/CAN1/GPIOA/AFIO clocks before touching them */
    uart_init();
    can_init();                /* normal mode, accept-all filter */
    uart_puts("J1939-MONITOR START\r\n");
    for (;;) {
        can_frame_t f;
        while (can_poll(&f)) on_frame(f.id, f.data, f.len);
    }
}
