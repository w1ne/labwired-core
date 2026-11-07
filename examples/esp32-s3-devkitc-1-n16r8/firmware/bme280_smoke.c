/* ESP32-S3-DevKitC-1 N16R8: read BME280 ID and temperature over I2C0. */
#include <stdint.h>

#define REG32(addr) (*(volatile uint32_t *)(uintptr_t)(addr))

enum {
  UART0_FIFO = 0x60000000,
  SYS_PERIP_CLK_EN0 = 0x600C0018,
  SYS_PERIP_RST_EN0 = 0x600C0020,
  GPIO_BASE = 0x60004000,
  GPIO_ENABLE_W1TS = GPIO_BASE + 0x24,
  GPIO_PIN8 = GPIO_BASE + 0x74 + 8 * 4,
  GPIO_PIN9 = GPIO_BASE + 0x74 + 9 * 4,
  GPIO_OUT_SEL8 = GPIO_BASE + 0x554 + 8 * 4,
  GPIO_OUT_SEL9 = GPIO_BASE + 0x554 + 9 * 4,
  GPIO_IN_SEL_SDA = GPIO_BASE + 0x154 + 90 * 4,
  GPIO_IN_SEL_SCL = GPIO_BASE + 0x154 + 89 * 4,
  IO_MUX_BASE = 0x60009000,
  I2C0 = 0x60013000,
  I2C_CTR = I2C0 + 0x04,
  I2C_FIFO_CONF = I2C0 + 0x18,
  I2C_DATA = I2C0 + 0x1C,
  I2C_INT_RAW = I2C0 + 0x20,
  I2C_INT_CLR = I2C0 + 0x24,
  I2C_CLK_CONF = I2C0 + 0x54,
  I2C_COMD0 = I2C0 + 0x58,
  I2C_SCL_LOW = I2C0 + 0x00,
  I2C_SDA_HOLD = I2C0 + 0x30,
  I2C_SDA_SAMPLE = I2C0 + 0x34,
  I2C_SCL_HIGH = I2C0 + 0x38,
  I2C_START_HOLD = I2C0 + 0x40,
  I2C_RSTART_SETUP = I2C0 + 0x44,
  I2C_STOP_HOLD = I2C0 + 0x48,
  I2C_STOP_SETUP = I2C0 + 0x4C,
  I2C_SDA_SIGNAL = 90,
  I2C_SCL_SIGNAL = 89,
  SDA_PIN = 8,
  SCL_PIN = 9,
  GPIO_INPUT_MATRIX = 1 << 7,
  I2C_FORCE_OUT = 3,
  I2C_MS_MODE = 1 << 4,
  I2C_CLK_EN = 1 << 8,
  I2C_CONF_UPGATE = 1 << 11,
  I2C_TRANS_START = 1 << 5,
  FIFO_RUN = (1 << 14) | (8 << 5) | 1,
  FIFO_RESET = FIFO_RUN | (1 << 13) | (1 << 12),
  INT_TRANS_COMPLETE = 1 << 7,
  INT_NACK = 1 << 10,
  OP_RSTART = 6,
  OP_WRITE = 1,
  OP_READ = 3,
  OP_STOP = 2,
  ACK_CHECK = 1 << 8,
  NACK_LAST = 1 << 10,
  BME280_ADDRESS = 0x77,
};

static void uart_byte(uint8_t value) { REG32(UART0_FIFO) = value; }

static void uart_str(const char *text) {
  while (*text) uart_byte((uint8_t)*text++);
}

static void uart_hex_byte(uint8_t value) {
  static const char hex[] = "0123456789ABCDEF";
  uart_byte((uint8_t)hex[value >> 4]);
  uart_byte((uint8_t)hex[value & 0x0F]);
}

static void uart_hex20(uint32_t value) {
  uart_hex_byte((uint8_t)(value >> 16));
  uart_hex_byte((uint8_t)(value >> 8));
  uart_hex_byte((uint8_t)value);
}

static void i2c_command(unsigned index, uint32_t opcode, uint32_t bytes,
                        uint32_t flags) {
  REG32(I2C_COMD0 + index * 4) = (opcode << 11) | (bytes & 0xFF) | flags;
}

static void setup_i2c(void) {
  REG32(SYS_PERIP_CLK_EN0) |= 1 << 7; /* I2C_EXT0 clock */
  REG32(SYS_PERIP_RST_EN0) = 0;       /* release peripheral reset */

  /* GPIO-matrix I2C0: SDA=GPIO8, SCL=GPIO9, with open-drain pads. */
  REG32(IO_MUX_BASE + 4 + 4 * SDA_PIN) = (1 << 12) | (2 << 10) | (1 << 9) | (1 << 8);
  REG32(IO_MUX_BASE + 4 + 4 * SCL_PIN) = (1 << 12) | (2 << 10) | (1 << 9) | (1 << 8);
  REG32(GPIO_PIN8) = 1 << 2;
  REG32(GPIO_PIN9) = 1 << 2;
  REG32(GPIO_OUT_SEL8) = I2C_SDA_SIGNAL;
  REG32(GPIO_OUT_SEL9) = I2C_SCL_SIGNAL;
  REG32(GPIO_ENABLE_W1TS) = (1 << SDA_PIN) | (1 << SCL_PIN);
  REG32(GPIO_IN_SEL_SDA) = GPIO_INPUT_MATRIX | SDA_PIN;
  REG32(GPIO_IN_SEL_SCL) = GPIO_INPUT_MATRIX | SCL_PIN;

  REG32(I2C_CTR) = I2C_MS_MODE | I2C_FORCE_OUT | I2C_CLK_EN;
  REG32(I2C_CLK_CONF) = 1 << 21; /* active XTAL source, divide by one */
  REG32(I2C_SCL_LOW) = 199;
  REG32(I2C_SCL_HIGH) = 199;
  REG32(I2C_SDA_HOLD) = 39;
  REG32(I2C_SDA_SAMPLE) = 39;
  REG32(I2C_START_HOLD) = 199;
  REG32(I2C_RSTART_SETUP) = 199;
  REG32(I2C_STOP_HOLD) = 199;
  REG32(I2C_STOP_SETUP) = 199;
  REG32(I2C_CTR) = I2C_MS_MODE | I2C_FORCE_OUT | I2C_CLK_EN | I2C_CONF_UPGATE;
}

/* Read one BME280 register using address-write / repeated-start / address-read. */
static int read_register(uint8_t reg, uint8_t *value) {
  REG32(I2C_FIFO_CONF) = FIFO_RESET;
  REG32(I2C_FIFO_CONF) = FIFO_RUN;
  REG32(I2C_INT_CLR) = 0xFFFFFFFF;
  REG32(I2C_DATA) = BME280_ADDRESS << 1;
  REG32(I2C_DATA) = reg;
  REG32(I2C_DATA) = (BME280_ADDRESS << 1) | 1;
  i2c_command(0, OP_RSTART, 0, 0);
  i2c_command(1, OP_WRITE, 2, ACK_CHECK);
  i2c_command(2, OP_RSTART, 0, 0);
  i2c_command(3, OP_WRITE, 1, ACK_CHECK);
  i2c_command(4, OP_READ, 1, NACK_LAST);
  i2c_command(5, OP_STOP, 0, 0);
  REG32(I2C_CTR) = I2C_MS_MODE | I2C_FORCE_OUT | I2C_CLK_EN | I2C_TRANS_START;

  for (unsigned i = 0; i < 500000; ++i) {
    uint32_t status = REG32(I2C_INT_RAW);
    if (status & INT_NACK) return -1;
    if (status & INT_TRANS_COMPLETE) {
      *value = (uint8_t)REG32(I2C_DATA);
      return 0;
    }
  }
  return -2;
}

void _start(void) {
  uint8_t id = 0;
  uint8_t msb = 0;
  uint8_t lsb = 0;
  uint8_t xlsb = 0;
  setup_i2c();

  if (read_register(0xD0, &id) != 0) {
    uart_str("BME280_I2C_ERROR\n");
    for (;;) {}
  }
  uart_str("BME280_ID=0x");
  uart_hex_byte(id);
  uart_str("\n");
  if (id != 0x60) {
    uart_str("BME280_ID_ERROR\n");
    for (;;) {}
  }

  if (read_register(0xFA, &msb) != 0 || read_register(0xFB, &lsb) != 0 ||
      read_register(0xFC, &xlsb) != 0) {
    uart_str("BME280_I2C_ERROR\n");
    for (;;) {}
  }
  uint32_t raw = ((uint32_t)msb << 12) | ((uint32_t)lsb << 4) | (xlsb >> 4);
  uart_str("BME280_TEMP_RAW=0x");
  uart_hex20(raw);
  uart_str("\n");
  for (;;) {}
}
