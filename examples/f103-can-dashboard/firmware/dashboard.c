/* DASHBOARD node: shows engine RPM and sensor temperature on an SSD1306 OLED.
 *
 * CAN: bxCAN1 at 500 kbit/s, an identifier-list filter for 0x100 (RPM, from
 * the engine) and 0x200 (temperature x10, from the sensor), both into FIFO0.
 * The RX0 interrupt (FMPIE0) copies the values out.
 *
 * Display: SSD1306 128x64 on I2C1 (PB6 SCL / PB7 SDA, address 0x3C, 400 kHz).
 *   pages 0-1  "RPM 3200"  (5x7 font at 2x2)
 *   page  3    a horizontal bar, 800..7000 rpm over 126 columns
 *   pages 5-6  "TEMP 21.5 C" (or "TEMP --" until a 0x200 frame arrives)
 *   page  7    "CAN OK", or this controller's own error state from ESR
 *              (BUS OFF / PASSIVE / WARNING), or NO RPM when 0x100 went quiet
 * The screen is redrawn into RAM at 10 Hz (TIM3) and only pages that changed
 * are sent over I2C.
 *
 * USART1 (PA9, 115200 8N1): one status line per second.
 * The main loop sleeps (WFI) between the 100 ms ticks; CAN is interrupt driven.
 */
#include "common.h"
#include "font.h"

#define CAN_ID_RPM 0x100u
#define CAN_ID_TEMP 0x200u
#define RPM_MIN 800u
#define RPM_MAX 7000u
#define STALE_TICKS 2u /* no 0x100 for >200 ms = the engine data is stale */

/* ---- received state (written by the RX0 interrupt) -------------------- */
static volatile uint32_t g_rpm;
static volatile int32_t g_temp_x10;
static volatile uint8_t g_have_rpm;
static volatile uint8_t g_have_temp;
static volatile uint32_t g_rx_rpm_frames;
static volatile uint32_t g_rx_temp_frames;
static volatile uint32_t g_last_rpm_tick;
static volatile uint32_t g_tick; /* 100 ms ticks, from TIM3 */
static volatile uint8_t g_refresh;

void TIM3_IRQHandler(void)
{
    TIM_SR(TIM3_BASE) = 0u;
    g_tick++;
    g_refresh = 1u;
}

void USB_LP_CAN1_RX0_IRQHandler(void)
{
    while ((CAN_RF0R & 3u) != 0u) { /* FMP0: messages pending */
        uint32_t rir = CAN_RIR;
        uint32_t dlc = CAN_RDTR & 0xFu;
        uint32_t lo = CAN_RDLR;
        CAN_RF0R = CAN_RF0R_RFOM0; /* release the FIFO slot */
        if ((rir & 4u) != 0u || dlc < 2u) { /* extended id or short frame: ignore */
            continue;
        }
        uint32_t value = ((lo & 0xFFu) << 8) | ((lo >> 8) & 0xFFu); /* big-endian */
        uint32_t id = rir >> 21;
        if (id == CAN_ID_RPM) {
            g_rpm = value;
            g_have_rpm = 1u;
            g_rx_rpm_frames++;
            g_last_rpm_tick = g_tick;
        } else if (id == CAN_ID_TEMP) {
            g_temp_x10 = (int16_t) value;
            g_have_temp = 1u;
            g_rx_temp_frames++;
        }
    }
}

/* ---- SSD1306 over I2C1 (STM32F1 I2C: RM0008 section 26) ------------------ */
#define I2C1_BASE 0x40005400u
#define I2C1_CR1 REG32(I2C1_BASE + 0x00u)
#define I2C1_CR2 REG32(I2C1_BASE + 0x04u)
#define I2C1_DR REG32(I2C1_BASE + 0x10u)
#define I2C1_SR1 REG32(I2C1_BASE + 0x14u)
#define I2C1_SR2 REG32(I2C1_BASE + 0x18u)
#define I2C1_CCR REG32(I2C1_BASE + 0x1Cu)
#define I2C1_TRISE REG32(I2C1_BASE + 0x20u)
#define SR1_SB (1u << 0)
#define SR1_ADDR (1u << 1)
#define SR1_BTF (1u << 2)
#define SR1_TXE (1u << 7)
#define SR1_AF (1u << 10)
#define OLED_ADDR_W (0x3Cu << 1)

/* Bounded wait so a missing display cannot hang the node. */
static int i2c_wait(uint32_t mask)
{
    for (uint32_t i = 0u; i < 200000u; i++) {
        if ((I2C1_SR1 & mask) != 0u) {
            return 1;
        }
    }
    return 0;
}

static int i2c_begin(uint8_t control)
{
    I2C1_CR1 = 0x0001u | 0x0100u; /* PE | START */
    if (!i2c_wait(SR1_SB)) {
        return 0;
    }
    I2C1_DR = OLED_ADDR_W;
    if (!i2c_wait(SR1_ADDR)) {
        return 0;
    }
    (void) I2C1_SR1; /* ADDR clears on SR1 then SR2 read */
    (void) I2C1_SR2;
    I2C1_DR = control;
    return i2c_wait(SR1_TXE);
}

static void i2c_put(uint8_t b)
{
    I2C1_DR = b;
    (void) i2c_wait(SR1_TXE);
}

static void i2c_end(void)
{
    (void) i2c_wait(SR1_BTF);
    I2C1_CR1 = 0x0001u | 0x0200u; /* PE | STOP */
}

static void oled_cmds(const uint8_t *c, unsigned n)
{
    if (i2c_begin(0x00u)) { /* control byte: command stream */
        for (unsigned i = 0u; i < n; i++) {
            i2c_put(c[i]);
        }
    }
    i2c_end();
}

static void oled_init(void)
{
    static const uint8_t init[] = {
        0xAE,       /* display off */
        0xD5, 0x80, /* clock divide / oscillator */
        0xA8, 0x3F, /* multiplex 1/64 */
        0xD3, 0x00, /* display offset 0 */
        0x40,       /* start line 0 */
        0x8D, 0x14, /* charge pump on */
        0x20, 0x00, /* horizontal addressing */
        0xA1,       /* segment remap */
        0xC8,       /* COM scan reversed */
        0xDA, 0x12, /* COM pins */
        0x81, 0xCF, /* contrast */
        0xD9, 0xF1, /* pre-charge */
        0xDB, 0x40, /* VCOMH */
        0xA4,       /* follow RAM */
        0xA6,       /* normal polarity */
        0xAF,       /* display on */
    };
    oled_cmds(init, sizeof init);
}

static void i2c_init(void)
{
    RCC_APB2ENR |= RCC_APB2ENR_IOPBEN | RCC_APB2ENR_AFIOEN;
    RCC_APB1ENR |= RCC_APB1ENR_I2C1EN;
    gpio_crl_set(GPIOB_BASE, 6u, CNF_AF_OD_50MHZ); /* SCL */
    gpio_crl_set(GPIOB_BASE, 7u, CNF_AF_OD_50MHZ); /* SDA */
    I2C1_CR1 = 0u;
    I2C1_CR2 = 36u;                  /* PCLK1 = 36 MHz */
    I2C1_CCR = (1u << 15) | 30u;     /* fast mode, 36e6 / (3 x 30) = 400 kHz */
    I2C1_TRISE = 11u;                /* 300 ns / 27.8 ns + 1 */
    I2C1_CR1 = 1u;
}

/* ---- frame buffer: 8 pages x 128 columns, bit 0 = top row of the page ----- */
static uint8_t g_fb[8][128];
static uint8_t g_shown[8][128]; /* what the panel holds */

static int font_index(char c)
{
    const char *chars = FONT_CHARS;
    for (int i = 0; chars[i] != '\0'; i++) {
        if (chars[i] == c) {
            return i;
        }
    }
    return 0; /* unknown -> blank */
}

/* Spread the low 8 bits of `b` to 16 (each bit doubled). */
static uint16_t double_bits(uint8_t b)
{
    uint16_t r = 0u;
    for (unsigned i = 0u; i < 8u; i++) {
        if ((b >> i) & 1u) {
            r |= (uint16_t) (3u << (2u * i));
        }
    }
    return r;
}

/* Draw `s` at page `page`, column `x`; each glyph column repeated `hs` times,
 * rows doubled when `vs` is 2 (the glyph then spans `page` and `page + 1`). */
static void draw_text(unsigned page, unsigned x, const char *s, unsigned hs, unsigned vs)
{
    for (; *s != '\0'; s++) {
        const unsigned char *g = font5x7[font_index(*s)];
        for (unsigned col = 0u; col < 6u; col++) { /* 5 glyph columns + 1 gap */
            uint8_t b = col < 5u ? g[col] : 0u;
            uint16_t wide = vs == 2u ? double_bits(b) : b;
            for (unsigned r = 0u; r < hs; r++) {
                if (x >= 128u) {
                    return;
                }
                g_fb[page][x] = (uint8_t) wide;
                if (vs == 2u) {
                    g_fb[page + 1u][x] = (uint8_t) (wide >> 8);
                }
                x++;
            }
        }
    }
}

static void fb_clear(void)
{
    for (unsigned p = 0u; p < 8u; p++) {
        for (unsigned c = 0u; c < 128u; c++) {
            g_fb[p][c] = 0u;
        }
    }
}

/* Bar on page 3: 1-pixel frame, filled inside up to `fill` columns. */
static void draw_bar(uint32_t fill)
{
    for (unsigned c = 0u; c < 128u; c++) {
        uint8_t v = 0x81u; /* top and bottom edge */
        if (c == 0u || c == 127u) {
            v = 0xFFu;
        } else if (c <= fill) {
            v = 0xFFu;
        }
        g_fb[3][c] = v;
    }
}

static char *put_uint_padded(char *p, uint32_t v, unsigned width)
{
    char tmp[10];
    unsigned n = 0u;
    do {
        tmp[n++] = (char) ('0' + v % 10u);
        v /= 10u;
    } while (v != 0u);
    for (unsigned pad = n; pad < width; pad++) {
        *p++ = ' ';
    }
    while (n != 0u) {
        *p++ = tmp[--n];
    }
    return p;
}

static char *put_str(char *p, const char *s)
{
    while (*s != '\0') {
        *p++ = *s++;
    }
    return p;
}

static const char *can_status(uint32_t esr, int stale)
{
    if ((esr & CAN_ESR_BOFF) != 0u) {
        return "CAN BUS OFF";
    }
    if ((esr & CAN_ESR_EPVF) != 0u) {
        return "CAN PASSIVE";
    }
    if ((esr & CAN_ESR_EWGF) != 0u) {
        return "CAN WARNING";
    }
    return stale ? "NO RPM" : "CAN OK";
}

static void render(uint32_t esr)
{
    char line[24];
    char *p;
    uint32_t rpm = g_rpm;
    int stale = !g_have_rpm || (g_tick - g_last_rpm_tick) > STALE_TICKS;

    fb_clear();

    p = put_str(line, "RPM ");
    if (g_have_rpm) {
        p = put_uint_padded(p, rpm, 4u);
    } else {
        p = put_str(p, "----");
    }
    *p = '\0';
    draw_text(0u, 0u, line, 2u, 2u);

    uint32_t clamped = rpm < RPM_MIN ? RPM_MIN : (rpm > RPM_MAX ? RPM_MAX : rpm);
    draw_bar(g_have_rpm ? (clamped - RPM_MIN) * 126u / (RPM_MAX - RPM_MIN) : 0u);

    p = put_str(line, "TEMP ");
    if (g_have_temp) {
        int32_t t = g_temp_x10;
        if (t < 0) {
            *p++ = '-';
            t = -t;
        }
        p = put_uint_padded(p, (uint32_t) t / 10u, 1u);
        *p++ = '.';
        *p++ = (char) ('0' + (uint32_t) t % 10u);
        p = put_str(p, " C");
    } else {
        p = put_str(p, "--");
    }
    *p = '\0';
    draw_text(5u, 0u, line, 1u, 2u);

    draw_text(7u, 0u, can_status(esr, stale), 1u, 1u);
}

/* Send only the pages that differ from what the panel shows. */
static void flush(void)
{
    for (uint8_t page = 0u; page < 8u; page++) {
        int dirty = 0;
        for (unsigned c = 0u; c < 128u; c++) {
            if (g_fb[page][c] != g_shown[page][c]) {
                dirty = 1;
                break;
            }
        }
        if (!dirty) {
            continue;
        }
        const uint8_t window[] = {0x21, 0x00, 0x7F, 0x22, page, page};
        oled_cmds(window, sizeof window);
        if (i2c_begin(0x40u)) { /* control byte: data stream */
            for (unsigned c = 0u; c < 128u; c++) {
                i2c_put(g_fb[page][c]);
                g_shown[page][c] = g_fb[page][c];
            }
        }
        i2c_end();
    }
}

static void status_line(uint32_t esr)
{
    uart_puts("DASH rpm=");
    if (g_have_rpm) {
        uart_put_uint(g_rpm);
    } else {
        uart_puts("--");
    }
    uart_puts(" temp_x10=");
    if (g_have_temp) {
        uart_put_uint((uint32_t) g_temp_x10);
    } else {
        uart_puts("--");
    }
    uart_puts(" rx100=");
    uart_put_uint(g_rx_rpm_frames);
    uart_puts(" rx200=");
    uart_put_uint(g_rx_temp_frames);
    uart_puts(" can=");
    uart_puts(can_status(esr, 0) + 4); /* skip "CAN " */
    uart_puts(" tec=");
    uart_put_uint((esr >> 16) & 0xFFu);
    uart_puts(" rec=");
    uart_put_uint(esr >> 24);
    uart_puts("\r\n");
}

int main(void)
{
    clock_init_72mhz();
    uart_init_115200();
    /* Join the bus first: the OLED init below takes milliseconds, and a node
     * that is not yet on the bus cannot ACK the engine's first frames. */
    can_init(CAN_BTR_FOR(500000u), CAN_ID_RPM, CAN_ID_TEMP);
    CAN_IER = CAN_IER_FMPIE0;
    nvic_enable(IRQ_CAN1_RX0);

    i2c_init();
    oled_init();
    for (unsigned p = 0u; p < 8u; p++) { /* force a full first paint */
        for (unsigned c = 0u; c < 128u; c++) {
            g_shown[p][c] = 0xAAu;
        }
    }

    RCC_APB1ENR |= RCC_APB1ENR_TIM3EN;
    timer_start_100us(TIM3_BASE, 1000u); /* 100 ms */
    nvic_enable(IRQ_TIM3);

    for (;;) {
        if (g_refresh == 0u) {
            __WFI(); /* a CAN frame or the next tick wakes us */
            continue;
        }
        g_refresh = 0u;
        uint32_t esr = CAN_ESR;
        render(esr);
        flush();
        if (g_tick % 10u == 0u) {
            status_line(esr);
        }
    }
}
