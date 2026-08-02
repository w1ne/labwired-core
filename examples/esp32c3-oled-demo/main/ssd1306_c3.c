/* SSD1306 OLED driver for the ESP32-C3 sim — see ssd1306_c3.h. */
#include "ssd1306_c3.h"

#define I2C0_BASE 0x60013000u
#define I2C_REG(o) (*(volatile uint32_t *)(I2C0_BASE + (o)))

#define I2C_CTR       I2C_REG(0x04u)
#define I2C_DATA      I2C_REG(0x1Cu)
#define I2C_INT_RAW   I2C_REG(0x20u)
#define I2C_INT_CLR   I2C_REG(0x24u)
#define I2C_FIFO_CONF I2C_REG(0x18u)
#define I2C_CMD0      0x58u

/* The OLED lab's physical route is SDA=GPIO4 / SCL=GPIO5.  ESP32-C3 I²C0
 * reaches those pads through the GPIO matrix; both directions must be wired
 * before the command-list controller can see the panel. */
#define GPIO_BASE 0x60004000u
#define GPIO_REG(o) (*(volatile uint32_t *)(GPIO_BASE + (o)))
#define GPIO_ENABLE_W1TS GPIO_REG(0x24u)
#define GPIO_FUNC_IN_SEL 0x154u
#define GPIO_FUNC_OUT_SEL 0x554u
#define GPIO_MATRIX_INPUT_SELECT (1u << 6)
#define I2C0_SCL_SIGNAL 53u
#define I2C0_SDA_SIGNAL 54u
#define I2C0_SDA_PIN 4u
#define I2C0_SCL_PIN 5u

#define CTR_TRANS_START (1u << 5)
#define INT_END_DETECT     (1u << 3)
#define INT_TRANS_COMPLETE (1u << 7)
#define INT_NACK           (1u << 10)
#define FIFO_RX_RST (1u << 12)
#define FIFO_TX_RST (1u << 13)

#define OP_RSTART 6u
#define OP_WRITE  1u
#define OP_STOP   2u

static inline uint32_t cmd(uint32_t op, uint32_t bytes) {
    return (op << 11) | (bytes & 0xFFu);
}
static inline void cmd_slot(uint32_t idx, uint32_t word) {
    I2C_REG(I2C_CMD0 + (idx * 4u)) = word;
}
static void i2c_reset_fifos(void) {
    I2C_FIFO_CONF = FIFO_RX_RST | FIFO_TX_RST;
}

static void i2c0_gpio_matrix_init(void) {
    GPIO_ENABLE_W1TS = (1u << I2C0_SDA_PIN) | (1u << I2C0_SCL_PIN);
    GPIO_REG(GPIO_FUNC_OUT_SEL + I2C0_SDA_PIN * 4u) = I2C0_SDA_SIGNAL;
    GPIO_REG(GPIO_FUNC_OUT_SEL + I2C0_SCL_PIN * 4u) = I2C0_SCL_SIGNAL;
    GPIO_REG(GPIO_FUNC_IN_SEL + I2C0_SDA_SIGNAL * 4u) =
        GPIO_MATRIX_INPUT_SELECT | I2C0_SDA_PIN;
    GPIO_REG(GPIO_FUNC_IN_SEL + I2C0_SCL_SIGNAL * 4u) =
        GPIO_MATRIX_INPUT_SELECT | I2C0_SCL_PIN;
}

static int i2c_run(void) {
    I2C_INT_CLR = 0xFFFFFFFFu;
    I2C_CTR |= CTR_TRANS_START;
    for (uint32_t spin = 0; spin < 100000u; spin++) {
        uint32_t raw = I2C_INT_RAW;
        if (raw & INT_NACK) {
            return 1;
        }
        if (raw & (INT_TRANS_COMPLETE | INT_END_DETECT)) {
            return 0;
        }
    }
    return 1;
}

/* One I²C write transaction: RSTART; WRITE n+1 (addr + payload); STOP.
 * `payload[0]` is the SSD1306 control byte (0x00 = commands, 0x40 = data).
 * n must be <= 31 so addr+payload fits the 32-byte TX FIFO. */
static void oled_write(const uint8_t *payload, int n) {
    i2c_reset_fifos();
    cmd_slot(0, cmd(OP_RSTART, 0));
    cmd_slot(1, cmd(OP_WRITE, (uint32_t)n + 1u));
    cmd_slot(2, cmd(OP_STOP, 0));
    I2C_DATA = (uint32_t)(SSD1306_I2C_ADDR << 1);
    for (int i = 0; i < n; i++) {
        I2C_DATA = (uint32_t)payload[i];
    }
    (void)i2c_run();
}

void ssd1306_init(void) {
    /* Canonical SSD1306 power-on sequence as one command stream (control 0x00).
     * 28 bytes incl. control byte → fits one 32-byte FIFO transaction. */
    static const uint8_t seq[] = {
        0x00,       /* control: command stream */
        0xAE,       /* display off */
        0xD5, 0x80, /* clock divide / osc freq */
        0xA8, 0x3F, /* multiplex ratio 64 */
        0xD3, 0x00, /* display offset 0 */
        0x40,       /* start line 0 */
        0x8D, 0x14, /* charge pump on */
        0x20, 0x00, /* horizontal addressing mode */
        0xA1,       /* segment remap */
        0xC8,       /* COM scan direction reversed */
        0xDA, 0x12, /* COM pins config */
        0x81, 0xCF, /* contrast */
        0xD9, 0xF1, /* pre-charge */
        0xDB, 0x40, /* VCOMH deselect */
        0xA4,       /* display follows RAM */
        0xA6,       /* normal (non-inverted) */
        0xAF,       /* display on */
    };
    i2c0_gpio_matrix_init();
    oled_write(seq, (int)sizeof(seq));
}

void ssd1306_flush(const uint8_t *fb) {
    /* Address the whole panel: horizontal mode, columns 0..127, pages 0..7. */
    static const uint8_t set_horiz[] = {0x00, 0x20, 0x00};
    static const uint8_t set_cols[] = {0x00, 0x21, 0x00, 0x7F};
    static const uint8_t set_pages[] = {0x00, 0x22, 0x00, 0x07};
    oled_write(set_horiz, 3);
    oled_write(set_cols, 4);
    oled_write(set_pages, 4);

    /* Stream the framebuffer as data (control 0x40), 30 bytes per transaction. */
    uint8_t chunk[31];
    chunk[0] = 0x40;
    int sent = 0;
    while (sent < SSD1306_FB_SIZE) {
        int k = SSD1306_FB_SIZE - sent;
        if (k > 30) {
            k = 30;
        }
        for (int i = 0; i < k; i++) {
            chunk[1 + i] = fb[sent + i];
        }
        oled_write(chunk, k + 1);
        sent += k;
    }
}
