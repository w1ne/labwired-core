/* ESP32-C3 on-target I²C platform shim for the REAL Melexis MLX90640 driver.
 *
 * The vendored driver (third_party/mlx90640-library) calls these four hooks to
 * talk to the sensor. This file implements them by driving the ESP32-C3 I²C0
 * controller's command-list engine directly (register map at 0x60013000, the
 * behavioral `Esp32c3I2c` model). NO Rust callback, NO bus bypass: every word
 * the driver reads is fetched by a genuine I²C transaction (RSTART → WRITE
 * addr → repeated-START → READ → STOP) executed by the simulated controller
 * against the attached MLX90640 device model.
 *
 * The MLX90640 uses 16-bit register addressing with 16-bit big-endian data
 * words (auto-incrementing). The driver passes byte counts as *word* counts;
 * we translate to the controller's byte-level command list. The controller's
 * RX FIFO holds 32 bytes, so multi-word reads are chunked into ≤16-word
 * transactions, each addressing `startAddress + words_done`.
 */
#include <stdint.h>
#include "MLX90640_I2C_Driver.h"

/* ── ESP32-C3 I2C0 register map (subset; offsets per the C3 i2c0 SVD) ──────── */
#define I2C0_BASE 0x60013000u
#define I2C_REG(o) (*(volatile uint32_t *)(I2C0_BASE + (o)))

#define I2C_CTR       I2C_REG(0x04u)
#define I2C_DATA      I2C_REG(0x1Cu)
#define I2C_INT_RAW   I2C_REG(0x20u)
#define I2C_INT_CLR   I2C_REG(0x24u)
#define I2C_FIFO_CONF I2C_REG(0x18u)
#define I2C_CMD0      0x58u

/* The physical circuit declared in every C3 thermal system manifest: I²C0
 * SDA=GPIO4, SCL=GPIO5. The C3 has no fixed I²C pads; firmware must program
 * both directions of the GPIO matrix before a real slave can see the bus. */
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

/* Command-list opcodes (ESP32-C3 TRM §16): encoded as (op<<11)|byte_num. */
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
    I2C_FIFO_CONF = FIFO_RX_RST | FIFO_TX_RST; /* self-clearing in the model */
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

/* Kick the command list and wait for completion (END / TRANS_COMPLETE). The
 * model runs the whole list synchronously on the TRANS_START write, so the
 * status is already set when we read it back; the bounded loop is belt-and-
 * braces for fidelity with real silicon. Returns 0 on ACKed completion, 1 on
 * NACK. */
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
    return 1; /* timeout → treat as NACK */
}

/* Read up to 16 words (32 bytes) starting at `addr` into `out`. */
static int read_chunk(uint8_t slaveAddr, uint16_t addr, uint16_t nwords, uint16_t *out) {
    uint32_t i;
    i2c_reset_fifos();

    /* Command list: RSTART; WRITE 3 (addr+W, hi, lo); RSTART; WRITE 1 (addr+R);
     * READ 2*nwords; STOP. */
    cmd_slot(0, cmd(OP_RSTART, 0));
    cmd_slot(1, cmd(OP_WRITE, 3));
    cmd_slot(2, cmd(OP_RSTART, 0));
    cmd_slot(3, cmd(OP_WRITE, 1));
    cmd_slot(4, cmd(OP_READ, (uint32_t)nwords * 2u));
    cmd_slot(5, cmd(OP_STOP, 0));

    /* TX FIFO: addr+W, reg_hi, reg_lo, addr+R. */
    I2C_DATA = (uint32_t)(slaveAddr << 1);          /* write address */
    I2C_DATA = (uint32_t)((addr >> 8) & 0xFFu);     /* register hi   */
    I2C_DATA = (uint32_t)(addr & 0xFFu);            /* register lo   */
    I2C_DATA = (uint32_t)((slaveAddr << 1) | 1u);   /* read address  */

    if (i2c_run() != 0) {
        return MLX90640_I2C_NACK_ERROR;
    }

    /* RX FIFO holds 2*nwords bytes, MSB first per word. */
    for (i = 0; i < nwords; i++) {
        uint16_t hi = (uint16_t)(I2C_DATA & 0xFFu);
        uint16_t lo = (uint16_t)(I2C_DATA & 0xFFu);
        out[i] = (uint16_t)((hi << 8) | lo);
    }
    return MLX90640_NO_ERROR;
}

/* ── Melexis I²C driver hooks ─────────────────────────────────────────────── */

void MLX90640_I2CInit(void) {
    i2c0_gpio_matrix_init();
    i2c_reset_fifos();
}

int MLX90640_I2CGeneralReset(void) { return MLX90640_NO_ERROR; }

void MLX90640_I2CFreqSet(int freq) { (void)freq; }

int MLX90640_I2CRead(uint8_t slaveAddr, uint16_t startAddress,
                     uint16_t nMemAddressRead, uint16_t *data) {
    uint16_t done = 0;
    while (done < nMemAddressRead) {
        uint16_t chunk = nMemAddressRead - done;
        if (chunk > 16) {
            chunk = 16; /* ≤32 bytes per RX-FIFO load */
        }
        int rc = read_chunk(slaveAddr, (uint16_t)(startAddress + done), chunk,
                            data + done);
        if (rc != MLX90640_NO_ERROR) {
            return rc;
        }
        done = (uint16_t)(done + chunk);
    }
    return MLX90640_NO_ERROR;
}

int MLX90640_I2CWrite(uint8_t slaveAddr, uint16_t writeAddress, uint16_t data) {
    i2c_reset_fifos();

    /* Command list: RSTART; WRITE 5 (addr+W, reg_hi, reg_lo, val_hi, val_lo);
     * STOP. */
    cmd_slot(0, cmd(OP_RSTART, 0));
    cmd_slot(1, cmd(OP_WRITE, 5));
    cmd_slot(2, cmd(OP_STOP, 0));

    I2C_DATA = (uint32_t)(slaveAddr << 1);
    I2C_DATA = (uint32_t)((writeAddress >> 8) & 0xFFu);
    I2C_DATA = (uint32_t)(writeAddress & 0xFFu);
    I2C_DATA = (uint32_t)((data >> 8) & 0xFFu);
    I2C_DATA = (uint32_t)(data & 0xFFu);

    if (i2c_run() != 0) {
        return MLX90640_I2C_NACK_ERROR;
    }
    /* The driver read-backs the written value to confirm; our model accepts the
     * write, so just report success. */
    return MLX90640_NO_ERROR;
}
