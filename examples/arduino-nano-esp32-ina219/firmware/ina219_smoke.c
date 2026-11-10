/* Arduino Nano ESP32: read INA219 voltage/current/power on A4/A5 (GPIO11/12). */
#include <stdint.h>

#define REG32(addr) (*(volatile uint32_t *)(uintptr_t)(addr))

enum {
  UART0_FIFO = 0x60000000,
  SYS_PERIP_CLK_EN0 = 0x600C0018,
  SYS_PERIP_RST_EN0 = 0x600C0020,
  GPIO_BASE = 0x60004000,
  GPIO_ENABLE_W1TS = GPIO_BASE + 0x24,
  GPIO_PIN11 = GPIO_BASE + 0x74 + 11 * 4,
  GPIO_PIN12 = GPIO_BASE + 0x74 + 12 * 4,
  GPIO_OUT_SEL11 = GPIO_BASE + 0x554 + 11 * 4,
  GPIO_OUT_SEL12 = GPIO_BASE + 0x554 + 12 * 4,
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
  SDA_PIN = 11,
  SCL_PIN = 12,
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
  INA219_ADDRESS = 0x40,
};

static void uart_byte(uint8_t value) { REG32(UART0_FIFO) = value; }

static void uart_str(const char *text) {
  while (*text) uart_byte((uint8_t)*text++);
}

static void uart_hex16(uint16_t value) {
  static const char hex[] = "0123456789ABCDEF";
  uart_byte((uint8_t)hex[(value >> 12) & 0xF]);
  uart_byte((uint8_t)hex[(value >> 8) & 0xF]);
  uart_byte((uint8_t)hex[(value >> 4) & 0xF]);
  uart_byte((uint8_t)hex[value & 0xF]);
}

static void uart_u32(uint32_t value) {
  char digits[10];
  unsigned count = 0;
  do {
    digits[count++] = (char)('0' + (value % 10));
    value /= 10;
  } while (value != 0);
  while (count != 0) uart_byte((uint8_t)digits[--count]);
}

static void uart_i32(int32_t value) {
  if (value < 0) {
    uart_byte('-');
    uart_u32((uint32_t)(-value));
  } else {
    uart_u32((uint32_t)value);
  }
}

static void i2c_command(unsigned index, uint32_t opcode, uint32_t bytes,
                        uint32_t flags) {
  REG32(I2C_COMD0 + index * 4) = (opcode << 11) | (bytes & 0xFF) | flags;
}

static void setup_i2c(void) {
  REG32(SYS_PERIP_CLK_EN0) |= 1 << 7;
  REG32(SYS_PERIP_RST_EN0) = 0;
  REG32(IO_MUX_BASE + 4 + 4 * SDA_PIN) =
      (1 << 12) | (2 << 10) | (1 << 9) | (1 << 8);
  REG32(IO_MUX_BASE + 4 + 4 * SCL_PIN) =
      (1 << 12) | (2 << 10) | (1 << 9) | (1 << 8);
  REG32(GPIO_PIN11) = 1 << 2;
  REG32(GPIO_PIN12) = 1 << 2;
  REG32(GPIO_OUT_SEL11) = I2C_SDA_SIGNAL;
  REG32(GPIO_OUT_SEL12) = I2C_SCL_SIGNAL;
  REG32(GPIO_ENABLE_W1TS) = (1 << SDA_PIN) | (1 << SCL_PIN);
  REG32(GPIO_IN_SEL_SDA) = GPIO_INPUT_MATRIX | SDA_PIN;
  REG32(GPIO_IN_SEL_SCL) = GPIO_INPUT_MATRIX | SCL_PIN;
  REG32(I2C_CTR) = I2C_MS_MODE | I2C_FORCE_OUT | I2C_CLK_EN;
  REG32(I2C_CLK_CONF) = 1 << 21;
  REG32(I2C_SCL_LOW) = 199;
  REG32(I2C_SCL_HIGH) = 199;
  REG32(I2C_SDA_HOLD) = 39;
  REG32(I2C_SDA_SAMPLE) = 39;
  REG32(I2C_START_HOLD) = 199;
  REG32(I2C_RSTART_SETUP) = 199;
  REG32(I2C_STOP_HOLD) = 199;
  REG32(I2C_STOP_SETUP) = 199;
  REG32(I2C_CTR) =
      I2C_MS_MODE | I2C_FORCE_OUT | I2C_CLK_EN | I2C_CONF_UPGATE;
}

static void begin_transaction(void) {
  REG32(I2C_FIFO_CONF) = FIFO_RESET;
  REG32(I2C_FIFO_CONF) = FIFO_RUN;
  REG32(I2C_INT_CLR) = 0xFFFFFFFF;
}

static int wait_transaction(void) {
  for (unsigned i = 0; i < 500000; ++i) {
    uint32_t status = REG32(I2C_INT_RAW);
    if (status & INT_NACK) return -1;
    if (status & INT_TRANS_COMPLETE) return 0;
  }
  return -2;
}

static int read_register16(uint8_t reg, uint16_t *value) {
  begin_transaction();
  REG32(I2C_DATA) = INA219_ADDRESS << 1;
  REG32(I2C_DATA) = reg;
  REG32(I2C_DATA) = (INA219_ADDRESS << 1) | 1;
  i2c_command(0, OP_RSTART, 0, 0);
  i2c_command(1, OP_WRITE, 2, ACK_CHECK);
  i2c_command(2, OP_RSTART, 0, 0);
  i2c_command(3, OP_WRITE, 1, ACK_CHECK);
  i2c_command(4, OP_READ, 2, NACK_LAST);
  i2c_command(5, OP_STOP, 0, 0);
  REG32(I2C_CTR) =
      I2C_MS_MODE | I2C_FORCE_OUT | I2C_CLK_EN | I2C_TRANS_START;
  if (wait_transaction() != 0) return -1;
  uint8_t hi = (uint8_t)REG32(I2C_DATA);
  uint8_t lo = (uint8_t)REG32(I2C_DATA);
  *value = ((uint16_t)hi << 8) | lo;
  return 0;
}

static int write_register16(uint8_t reg, uint16_t value) {
  begin_transaction();
  REG32(I2C_DATA) = INA219_ADDRESS << 1;
  REG32(I2C_DATA) = reg;
  REG32(I2C_DATA) = (uint8_t)(value >> 8);
  REG32(I2C_DATA) = (uint8_t)value;
  i2c_command(0, OP_RSTART, 0, 0);
  i2c_command(1, OP_WRITE, 4, ACK_CHECK);
  i2c_command(2, OP_STOP, 0, 0);
  REG32(I2C_CTR) =
      I2C_MS_MODE | I2C_FORCE_OUT | I2C_CLK_EN | I2C_TRANS_START;
  return wait_transaction();
}

static void print_sample(void) {
  uint16_t bus = 0;
  uint16_t current = 0;
  uint16_t power = 0;
  if (read_register16(0x02, &bus) != 0 ||
      read_register16(0x04, &current) != 0 ||
      read_register16(0x03, &power) != 0) {
    uart_str("INA219_I2C_ERROR\n");
    return;
  }
  uart_str("INA219_VBUS_MV=");
  uart_u32((uint32_t)(bus >> 3) * 4);
  uart_str(" I_MA=");
  uart_i32((int16_t)current / 10);
  uart_str(" P_MW=");
  uart_u32((uint32_t)power * 2);
  uart_byte('\n');
}

void _start(void) {
  setup_i2c();
  uart_str("INA219_NANO_ESP32_READY\n");

  if (write_register16(0x00, 0x399F) != 0 ||
      write_register16(0x05, 4096) != 0) {
    uart_str("INA219_I2C_ERROR\n");
    for (;;) {}
  }
  uint16_t config = 0;
  if (read_register16(0x00, &config) != 0) {
    uart_str("INA219_I2C_ERROR\n");
    for (;;) {}
  }
  uart_str("INA219_CONFIG=0x");
  uart_hex16(config);
  uart_byte('\n');

  for (;;) {
    print_sample();
    for (volatile uint32_t delay = 0; delay < 200000; ++delay) {}
  }
}
