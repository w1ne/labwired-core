/* VEML7700 ambient-light driver for the ESP32-C3 sim — see veml7700_c3.h. */
#include "veml7700_c3.h"

#define I2C0_BASE 0x60013000u
#define I2C_REG(o) (*(volatile uint32_t *)(I2C0_BASE + (o)))

#define I2C_CTR       I2C_REG(0x04u)
#define I2C_DATA      I2C_REG(0x1Cu)
#define I2C_INT_RAW   I2C_REG(0x20u)
#define I2C_INT_CLR   I2C_REG(0x24u)
#define I2C_FIFO_CONF I2C_REG(0x18u)
#define I2C_CMD0      0x58u

#define CTR_TRANS_START (1u << 5)
#define INT_END_DETECT     (1u << 3)
#define INT_TRANS_COMPLETE (1u << 7)
#define INT_NACK           (1u << 10)
#define FIFO_RX_RST (1u << 12)
#define FIFO_TX_RST (1u << 13)

#define OP_RSTART 6u
#define OP_WRITE  1u
#define OP_READ   3u
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

void veml7700_init(void) {
    i2c_reset_fifos();
    /* Write ALS_CONF = 0x0000 (power on, gain ×1, IT 100 ms). The config is a
     * 16-bit little-endian word, so the device must receive reg + low + high =
     * 3 data bytes. The C3 WRITE byte count INCLUDES the address byte, so this
     * is WRITE 4: addr+W, reg, lo, hi — one transaction. */
    cmd_slot(0, cmd(OP_RSTART, 0));
    cmd_slot(1, cmd(OP_WRITE, 4));
    cmd_slot(2, cmd(OP_STOP, 0));
    I2C_DATA = (uint32_t)(VEML7700_I2C_ADDR << 1);
    I2C_DATA = (uint32_t)VEML7700_REG_ALS_CONF;
    I2C_DATA = 0x00u; /* config low byte  */
    I2C_DATA = 0x00u; /* config high byte */
    (void)i2c_run();
}

int veml7700_read_als(uint16_t *out_counts) {
    i2c_reset_fifos();
    /* Register read: RSTART; WRITE 2 (addr+W, reg); RSTART; WRITE 1 (addr+R);
     * READ 2; STOP. Output is 16-bit little-endian (low byte first). */
    cmd_slot(0, cmd(OP_RSTART, 0));
    cmd_slot(1, cmd(OP_WRITE, 2));
    cmd_slot(2, cmd(OP_RSTART, 0));
    cmd_slot(3, cmd(OP_WRITE, 1));
    cmd_slot(4, cmd(OP_READ, 2));
    cmd_slot(5, cmd(OP_STOP, 0));

    I2C_DATA = (uint32_t)(VEML7700_I2C_ADDR << 1);
    I2C_DATA = (uint32_t)VEML7700_REG_ALS;
    I2C_DATA = (uint32_t)((VEML7700_I2C_ADDR << 1) | 1u);

    if (i2c_run() != 0) {
        return -1;
    }
    uint16_t lo = (uint16_t)(I2C_DATA & 0xFFu);
    uint16_t hi = (uint16_t)(I2C_DATA & 0xFFu);
    *out_counts = (uint16_t)((hi << 8) | lo);
    return 0;
}

uint32_t veml7700_counts_to_lux(uint16_t counts) {
    /* Resolution at gain ×1 / IT 100 ms is 0.0576 lux/count. Integer-scale to
     * avoid floating point: lux = counts * 576 / 10000. */
    return ((uint32_t)counts * 576u) / 10000u;
}
