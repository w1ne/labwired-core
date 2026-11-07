/* Adafruit ESP32-S3 Feather #5885: read the onboard MAX17048 over I2C0. */
#include <stdint.h>

#define REG32(addr) (*(volatile uint32_t *)(uintptr_t)(addr))

enum {
  UART0_FIFO = 0x60000000,
  UART0_STATUS = 0x6000001C,
  SYS_PERIP_CLK_EN0 = 0x600C0018,
  SYS_PERIP_RST_EN0 = 0x600C0020,
  GPIO_BASE = 0x60004000,
  GPIO_ENABLE_W1TS = GPIO_BASE + 0x24,
  GPIO_PIN3 = GPIO_BASE + 0x74 + 3 * 4,
  GPIO_PIN4 = GPIO_BASE + 0x74 + 4 * 4,
  GPIO_OUT_SEL3 = GPIO_BASE + 0x554 + 3 * 4,
  GPIO_OUT_SEL4 = GPIO_BASE + 0x554 + 4 * 4,
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
  SDA_PIN = 3,
  SCL_PIN = 4,
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
  MAX17048_ADDRESS = 0x36,
  UART_TX_FIFO_CAPACITY = 128,
};

static void uart_byte(uint8_t value) {
  for (unsigned spin = 0; spin < 100000; ++spin) {
    if (((REG32(UART0_STATUS) >> 16) & 0x3FF) < UART_TX_FIFO_CAPACITY) break;
  }
  REG32(UART0_FIFO) = value;
}

static void uart_str(const char *text) {
  while (*text) uart_byte((uint8_t)*text++);
}

static void uart_hex_byte(uint8_t value) {
  static const char hex[] = "0123456789ABCDEF";
  uart_byte((uint8_t)hex[value >> 4]);
  uart_byte((uint8_t)hex[value & 0x0F]);
}

static void uart_hex_word(uint16_t value) {
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

  /* GPIO-matrix I2C0: board SDA=GPIO3 and SCL=GPIO4, open-drain pads. */
  REG32(IO_MUX_BASE + 4 + 4 * SDA_PIN) = (1 << 12) | (2 << 10) | (1 << 9) | (1 << 8);
  REG32(IO_MUX_BASE + 4 + 4 * SCL_PIN) = (1 << 12) | (2 << 10) | (1 << 9) | (1 << 8);
  REG32(GPIO_PIN3) = 1 << 2;
  REG32(GPIO_PIN4) = 1 << 2;
  REG32(GPIO_OUT_SEL3) = I2C_SDA_SIGNAL;
  REG32(GPIO_OUT_SEL4) = I2C_SCL_SIGNAL;
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

/* Pointer write, repeated-start, then read a big-endian MAX17048 register. */
static int read_word(uint8_t reg, uint16_t *value) {
  REG32(I2C_FIFO_CONF) = FIFO_RESET;
  REG32(I2C_FIFO_CONF) = FIFO_RUN;
  REG32(I2C_INT_CLR) = 0xFFFFFFFF;
  REG32(I2C_DATA) = MAX17048_ADDRESS << 1;
  REG32(I2C_DATA) = reg;
  REG32(I2C_DATA) = (MAX17048_ADDRESS << 1) | 1;
  i2c_command(0, OP_RSTART, 0, 0);
  i2c_command(1, OP_WRITE, 2, ACK_CHECK);
  i2c_command(2, OP_RSTART, 0, 0);
  i2c_command(3, OP_WRITE, 1, ACK_CHECK);
  i2c_command(4, OP_READ, 2, NACK_LAST);
  i2c_command(5, OP_STOP, 0, 0);
  REG32(I2C_CTR) = I2C_MS_MODE | I2C_FORCE_OUT | I2C_CLK_EN | I2C_TRANS_START;

  for (unsigned i = 0; i < 500000; ++i) {
    uint32_t status = REG32(I2C_INT_RAW);
    if (status & INT_NACK) return -1;
    if (status & INT_TRANS_COMPLETE) {
      uint8_t msb = (uint8_t)REG32(I2C_DATA);
      uint8_t lsb = (uint8_t)REG32(I2C_DATA);
      *value = ((uint16_t)msb << 8) | lsb;
      return 0;
    }
  }
  return -2;
}

void _start(void) {
  uint16_t vcell = 0;
  uint16_t soc = 0;
  setup_i2c();

  if (read_word(0x02, &vcell) != 0 || read_word(0x04, &soc) != 0) {
    uart_str("MAX17048_I2C_ERROR\n");
    for (;;) {}
  }

  uart_str("MAX17048_VCELL=0x");
  uart_hex_word(vcell);
  uart_str("\nMAX17048_SOC=0x");
  uart_hex_word(soc);
  uart_str("\n");
  if (vcell != 0xC080 || soc != 0x4880) {
    uart_str("MAX17048_VALUE_ERROR\n");
    for (;;) {}
  }
  uart_str("ADAFRUIT_FEATHER_ESP32S3_MAX17048_OK\n");
  for (;;) {}
}
