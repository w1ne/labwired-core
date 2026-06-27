/* MLX90614 IR-thermometer driver for the ESP32-C3 sim — see mlx90614_c3.h.
 *
 * Drives the same behavioral C3 I²C0 command-list engine the VEML7700 and the
 * Sensirion HAL use, but speaks the MLX90614's SMBus "read word" framing:
 *   RSTART; WRITE 2 (addr+W, cmd); RSTART; WRITE 1 (addr+R); READ 3; STOP
 * returning LSB, MSB, PEC. Temperature is raw × 0.02 K, and the PEC is an SMBus
 * CRC-8 (poly 0x07) over [addr·W, cmd, addr·R, LSB, MSB] which we verify. */
#include "mlx90614_c3.h"

#define I2C0_BASE 0x60013000u
#define I2C_REG(o) (*(volatile uint32_t *)(I2C0_BASE + (o)))

#define I2C_DATA      I2C_REG(0x1Cu)
#define I2C_CTR       I2C_REG(0x04u)
#define I2C_INT_RAW   I2C_REG(0x20u)
#define I2C_INT_CLR   I2C_REG(0x24u)
#define I2C_FIFO_CONF I2C_REG(0x18u)
#define I2C_CMD0      0x58u

#define CTR_TRANS_START    (1u << 5)
#define INT_END_DETECT     (1u << 3)
#define INT_TRANS_COMPLETE (1u << 7)
#define INT_NACK           (1u << 10)
#define FIFO_RX_RST        (1u << 12)
#define FIFO_TX_RST        (1u << 13)

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

/* SMBus PEC: CRC-8, polynomial 0x07, initial 0. */
static uint8_t smbus_pec(const uint8_t *bytes, int n) {
    uint8_t crc = 0;
    for (int i = 0; i < n; i++) {
        crc ^= bytes[i];
        for (int b = 0; b < 8; b++) {
            crc = (crc & 0x80u) ? (uint8_t)((crc << 1) ^ 0x07u) : (uint8_t)(crc << 1);
        }
    }
    return crc;
}

int mlx90614_read_centi_c(uint8_t ram_cmd, int32_t *out_centi_c) {
    I2C_FIFO_CONF = FIFO_RX_RST | FIFO_TX_RST;

    cmd_slot(0, cmd(OP_RSTART, 0));
    cmd_slot(1, cmd(OP_WRITE, 2)); /* addr+W, ram_cmd */
    cmd_slot(2, cmd(OP_RSTART, 0));
    cmd_slot(3, cmd(OP_WRITE, 1)); /* addr+R */
    cmd_slot(4, cmd(OP_READ, 3));  /* LSB, MSB, PEC */
    cmd_slot(5, cmd(OP_STOP, 0));

    I2C_DATA = (uint32_t)(MLX90614_I2C_ADDR << 1);
    I2C_DATA = (uint32_t)ram_cmd;
    I2C_DATA = (uint32_t)((MLX90614_I2C_ADDR << 1) | 1u);

    if (i2c_run() != 0) {
        return -1;
    }
    uint8_t lsb = (uint8_t)(I2C_DATA & 0xFFu);
    uint8_t msb = (uint8_t)(I2C_DATA & 0xFFu);
    uint8_t pec = (uint8_t)(I2C_DATA & 0xFFu);

    uint8_t frame[5] = {(uint8_t)(MLX90614_I2C_ADDR << 1), ram_cmd,
                        (uint8_t)((MLX90614_I2C_ADDR << 1) | 1u), lsb, msb};
    if (smbus_pec(frame, 5) != pec) {
        return -2;
    }

    uint16_t raw = (uint16_t)((msb << 8) | lsb);
    /* T[°C]×100 = raw × 0.02 × 100 − 273.15 × 100 = raw×2 − 27315. */
    *out_centi_c = (int32_t)raw * 2 - 27315;
    return 0;
}

int mlx90614_read_surface_centi_c(int32_t *out_centi_c) {
    return mlx90614_read_centi_c(MLX90614_RAM_TOBJ1, out_centi_c);
}
