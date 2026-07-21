/* ESP32-C3 I²C platform shim for the REAL Sensirion embedded-i2c drivers.
 *
 * The unmodified vendor drivers (third_party/embedded-i2c-{scd4x,sgp41,sps30})
 * call these six hooks through their shared sensirion_i2c.c layer. This file
 * implements them by driving the ESP32-C3 I²C0 controller's command-list engine
 * directly (register map at 0x60013000, the behavioral `Esp32c3I2c` model). NO
 * Rust callback, NO bus bypass: every byte the drivers read is fetched by a
 * genuine I²C transaction (RSTART → WRITE addr → READ → STOP) executed by the
 * simulated controller against the attached sensor device models.
 *
 * Sensirion's wire protocol is plain byte streams (not register-addressed): a
 * command write is one transaction, a data read is a separate transaction. The
 * C3 controller clocks each command list bit-by-bit into a 32-byte FIFO, so a
 * single read is capped at 32 bytes — every Sensirion read on this board
 * (data-ready 3 B, SCD4x measurement 9 B, SGP41 raw 6 B, SPS30 uint16 30 B)
 * fits inside one transaction.
 */
#include "sensirion_i2c_hal.h"
#include "sensirion_common.h"
#include "sensirion_config.h"

/* ── ESP32-C3 I2C0 register map (subset; offsets per the C3 i2c0 SVD) ──────── */
#define I2C0_BASE 0x60013000u
#define I2C_REG(o) (*(volatile uint32_t *)(I2C0_BASE + (o)))

#define I2C_CTR       I2C_REG(0x04u)
#define I2C_DATA      I2C_REG(0x1Cu)
#define I2C_INT_RAW   I2C_REG(0x20u)
#define I2C_INT_CLR   I2C_REG(0x24u)
#define I2C_FIFO_CONF I2C_REG(0x18u)
#define I2C_CMD0      0x58u

/* Physical I²C0 wiring declared by the Leo system manifests. ESP32-C3 uses a
 * GPIO matrix, so a controller transaction reaches these pads only after both
 * its output and input signal paths are programmed. */
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

/* Kick the command list and busy-wait for completion (END / TRANS_COMPLETE),
 * exactly as on real silicon: the controller clocks the transaction on the
 * wire at the rate its timing registers dictate, so the spin runs for the
 * real wire time. Returns 0 on ACKed completion, 1 on NACK/timeout. */
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

/* ── Sensirion I²C HAL hooks ──────────────────────────────────────────────── */

void sensirion_i2c_hal_init(void) {
    i2c0_gpio_matrix_init();
    i2c_reset_fifos();
}

void sensirion_i2c_hal_free(void) {
    /* Nothing to release in the simulated controller. */
}

int16_t sensirion_i2c_hal_select_bus(uint8_t bus_idx) {
    (void)bus_idx; /* single I²C0 bus on this board */
    return NO_ERROR;
}

/* Write `count` bytes to a 7-bit `address`:
 *   RSTART; WRITE (1 + count) [addr<<1, data...]; STOP. */
int8_t sensirion_i2c_hal_write(uint8_t address, const uint8_t *data,
                               uint8_t count) {
    i2c_reset_fifos();

    cmd_slot(0, cmd(OP_RSTART, 0));
    cmd_slot(1, cmd(OP_WRITE, (uint32_t)count + 1u));
    cmd_slot(2, cmd(OP_STOP, 0));

    I2C_DATA = (uint32_t)(address << 1); /* addr + W */
    for (uint8_t i = 0; i < count; i++) {
        I2C_DATA = (uint32_t)data[i];
    }

    return i2c_run() == 0 ? 0 : -1;
}

/* Read `count` bytes from a 7-bit `address`:
 *   RSTART; WRITE 1 [addr<<1 | R]; READ count; STOP. */
int8_t sensirion_i2c_hal_read(uint8_t address, uint8_t *data, uint8_t count) {
    i2c_reset_fifos();

    cmd_slot(0, cmd(OP_RSTART, 0));
    cmd_slot(1, cmd(OP_WRITE, 1));
    cmd_slot(2, cmd(OP_READ, (uint32_t)count));
    cmd_slot(3, cmd(OP_STOP, 0));

    I2C_DATA = (uint32_t)((address << 1) | 1u); /* addr + R */

    if (i2c_run() != 0) {
        return -1;
    }

    for (uint8_t i = 0; i < count; i++) {
        data[i] = (uint8_t)(I2C_DATA & 0xFFu);
    }
    return 0;
}

/* The simulator advances the sensor scene on each measurement command, not on
 * elapsed time, so a real sleep would only waste sim steps. Burn a tiny bounded
 * spin to keep the driver's timing-loop structure intact. */
void sensirion_i2c_hal_sleep_usec(uint32_t useconds) {
    volatile uint32_t spin = useconds & 0xFFu;
    while (spin--) {
    }
}
