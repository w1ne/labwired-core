/* ESP32-S3-DevKitC-1 N16R8: read an SHT40 high-precision sample over I2C1. */
#include <stdint.h>

#define REG32(addr) (*(volatile uint32_t *)(uintptr_t)(addr))

enum {
  UART0_FIFO = 0x60000000,
  SYS_PERIP_CLK_EN0 = 0x600C0018,
  SYS_PERIP_RST_EN0 = 0x600C0020,
  GPIO_BASE = 0x60004000,
  GPIO_ENABLE1_W1TS = GPIO_BASE + 0x30,
  GPIO_PIN_SDA = GPIO_BASE + 0x74 + 41 * 4,
  GPIO_PIN_SCL = GPIO_BASE + 0x74 + 42 * 4,
  GPIO_OUT_SEL_SDA = GPIO_BASE + 0x554 + 41 * 4,
  GPIO_OUT_SEL_SCL = GPIO_BASE + 0x554 + 42 * 4,
  GPIO_IN_SEL_SDA = GPIO_BASE + 0x154 + 92 * 4,
  GPIO_IN_SEL_SCL = GPIO_BASE + 0x154 + 91 * 4,
  IO_MUX_BASE = 0x60009000,
  I2C_BASE = 0x60027000,
  I2C_CTR = I2C_BASE + 0x04,
  I2C_FIFO_CONF = I2C_BASE + 0x18,
  I2C_DATA = I2C_BASE + 0x1C,
  I2C_INT_RAW = I2C_BASE + 0x20,
  I2C_INT_CLR = I2C_BASE + 0x24,
  I2C_CLK_CONF = I2C_BASE + 0x54,
  I2C_COMD0 = I2C_BASE + 0x58,
  I2C_SCL_LOW = I2C_BASE + 0x00,
  I2C_SDA_HOLD = I2C_BASE + 0x30,
  I2C_SDA_SAMPLE = I2C_BASE + 0x34,
  I2C_SCL_HIGH = I2C_BASE + 0x38,
  I2C_START_HOLD = I2C_BASE + 0x40,
  I2C_RSTART_SETUP = I2C_BASE + 0x44,
  I2C_STOP_HOLD = I2C_BASE + 0x48,
  I2C_STOP_SETUP = I2C_BASE + 0x4C,
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
  SHT40_ADDRESS = 0x44,
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

static void uart_hex_word(uint16_t value) {
  uart_hex_byte((uint8_t)(value >> 8));
  uart_hex_byte((uint8_t)value);
}

static void i2c_command(unsigned index, uint32_t opcode, uint32_t bytes,
                        uint32_t flags) {
  REG32(I2C_COMD0 + index * 4) = (opcode << 11) | (bytes & 0xFF) | flags;
}

static void setup_i2c(void) {
  REG32(SYS_PERIP_CLK_EN0) |= 1U << 18;
  REG32(SYS_PERIP_RST_EN0) = 0;

  /* I2C1 routes through GPIO matrix signals 92/91 to board header GPIO41/42. */
  REG32(IO_MUX_BASE + 4 + 4 * 41) =
      (1 << 12) | (2 << 10) | (1 << 9) | (1 << 8);
  REG32(IO_MUX_BASE + 4 + 4 * 42) =
      (1 << 12) | (2 << 10) | (1 << 9) | (1 << 8);
  REG32(GPIO_PIN_SDA) = 1 << 2;
  REG32(GPIO_PIN_SCL) = 1 << 2;
  REG32(GPIO_OUT_SEL_SDA) = 92;
  REG32(GPIO_OUT_SEL_SCL) = 91;
  REG32(GPIO_ENABLE1_W1TS) = (1U << (41 - 32)) | (1U << (42 - 32));
  REG32(GPIO_IN_SEL_SDA) = GPIO_INPUT_MATRIX | 41;
  REG32(GPIO_IN_SEL_SCL) = GPIO_INPUT_MATRIX | 42;

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
  REG32(I2C_CTR) =
      I2C_MS_MODE | I2C_FORCE_OUT | I2C_CLK_EN | I2C_CONF_UPGATE;
}

static int wait_for_completion(void) {
  for (unsigned i = 0; i < 500000; ++i) {
    uint32_t status = REG32(I2C_INT_RAW);
    if (status & INT_NACK) return -1;
    if (status & INT_TRANS_COMPLETE) return 0;
  }
  return -2;
}

/* Send one SHT4x opcode as a complete I2C write transaction. */
static int write_command(uint8_t command) {
  REG32(I2C_FIFO_CONF) = FIFO_RESET;
  REG32(I2C_FIFO_CONF) = FIFO_RUN;
  REG32(I2C_INT_CLR) = 0xFFFFFFFF;
  REG32(I2C_DATA) = SHT40_ADDRESS << 1;
  REG32(I2C_DATA) = command;
  i2c_command(0, OP_RSTART, 0, 0);
  i2c_command(1, OP_WRITE, 2, ACK_CHECK);
  i2c_command(2, OP_STOP, 0, 0);
  REG32(I2C_CTR) = I2C_MS_MODE | I2C_FORCE_OUT | I2C_CLK_EN | I2C_TRANS_START;
  return wait_for_completion();
}

/* Read the SHT40's two CRC-framed words in a fresh I2C read transaction. */
static int read_measurement(uint8_t data[6]) {
  REG32(I2C_FIFO_CONF) = FIFO_RESET;
  REG32(I2C_FIFO_CONF) = FIFO_RUN;
  REG32(I2C_INT_CLR) = 0xFFFFFFFF;
  REG32(I2C_DATA) = (SHT40_ADDRESS << 1) | 1;
  i2c_command(0, OP_RSTART, 0, 0);
  i2c_command(1, OP_WRITE, 1, ACK_CHECK);
  i2c_command(2, OP_READ, 6, NACK_LAST);
  i2c_command(3, OP_STOP, 0, 0);
  REG32(I2C_CTR) = I2C_MS_MODE | I2C_FORCE_OUT | I2C_CLK_EN | I2C_TRANS_START;
  if (wait_for_completion() != 0) return -1;
  for (unsigned i = 0; i < 6; ++i) data[i] = (uint8_t)REG32(I2C_DATA);
  return 0;
}

static uint8_t sensirion_crc(const uint8_t *data) {
  uint8_t crc = 0xFF;
  for (unsigned i = 0; i < 2; ++i) {
    crc ^= data[i];
    for (unsigned bit = 0; bit < 8; ++bit) {
      crc = (crc & 0x80) ? (uint8_t)((crc << 1) ^ 0x31) : (uint8_t)(crc << 1);
    }
  }
  return crc;
}

static uint32_t read_ccount(void) {
  uint32_t count;
  __asm__ volatile("rsr.ccount %0" : "=a"(count));
  return count;
}

static void delay_us(uint32_t microseconds) {
  const uint32_t ticks = microseconds * 240U;
  const uint32_t start = read_ccount();
  while ((uint32_t)(read_ccount() - start) < ticks) {
  }
}

__attribute__((noreturn, used, section(".entry"))) void _start(void) {
  uint8_t sample[6];
  setup_i2c();

  /* The fixed serial-number command isolates I2C framing from conversion time. */
  if (write_command(0x89) != 0 || read_measurement(sample) != 0 ||
      sample[0] != 0x0F || sample[1] != 0x4E ||
      sample[3] != 0x2A || sample[4] != 0x71 ||
      sample[2] != sensirion_crc(sample) ||
      sample[5] != sensirion_crc(sample + 3)) {
    uart_str("SHT40_SERIAL_ERROR\n");
    for (;;) {}
  }
  uart_str("SHT40_SERIAL_OK\n");

  /* 0xFD is the SHT40 high-precision, no-heater measurement command. */
  if (write_command(0xFD) != 0) {
    uart_str("SHT40_COMMAND_ERROR\n");
    for (;;) {}
  }
  /* The datasheet maximum is 8.3 ms; allow 0.2 ms for the µs timer boundary. */
  delay_us(8500);
  if (read_measurement(sample) != 0) {
    uart_str("SHT40_READ_ERROR\n");
    for (;;) {}
  }

  uint16_t temperature = ((uint16_t)sample[0] << 8) | sample[1];
  uint16_t humidity = ((uint16_t)sample[3] << 8) | sample[4];
  uart_str("SHT40_TEMP_RAW=0x");
  uart_hex_word(temperature);
  uart_str("\nSHT40_RH_RAW=0x");
  uart_hex_word(humidity);
  uart_byte('\n');

  if (sample[2] == sensirion_crc(sample) &&
      sample[5] == sensirion_crc(sample + 3)) {
    uart_str("SHT40_CRC_OK\n");
  } else {
    uart_str("SHT40_CRC_ERROR\n");
  }
  for (;;) __asm__ volatile("nop" ::: "memory");
}
