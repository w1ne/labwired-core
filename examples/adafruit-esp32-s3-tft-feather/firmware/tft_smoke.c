typedef unsigned int uint32_t;
typedef unsigned short uint16_t;
typedef unsigned char uint8_t;

#define REG32(addr) (*(volatile uint32_t *)(addr))

#define UART0_FIFO REG32(0x60000000u)
#define UART0_STATUS REG32(0x6000001Cu)

#define GPIO_BASE 0x60004000u
#define GPIO_OUT_W1TS REG32(GPIO_BASE + 0x08u)
#define GPIO_OUT_W1TC REG32(GPIO_BASE + 0x0Cu)
#define GPIO_OUT1_W1TS REG32(GPIO_BASE + 0x14u)
#define GPIO_OUT1_W1TC REG32(GPIO_BASE + 0x18u)
#define GPIO_ENABLE_W1TS REG32(GPIO_BASE + 0x24u)
#define GPIO_ENABLE1_W1TS REG32(GPIO_BASE + 0x30u)
#define GPIO_FUNC_OUT_SEL(pin) REG32(0x60004554u + 4u * (pin))
#define IO_MUX(pin) REG32(0x60009004u + 4u * (pin))

#define SPI2_BASE 0x60024000u
#define SPI_CMD REG32(SPI2_BASE + 0x00u)
#define SPI_CLOCK REG32(SPI2_BASE + 0x0Cu)
#define SPI_USER REG32(SPI2_BASE + 0x10u)
#define SPI_USER1 REG32(SPI2_BASE + 0x14u)
#define SPI_MS_DLEN REG32(SPI2_BASE + 0x1Cu)
#define SPI_MISC REG32(SPI2_BASE + 0x20u)
#define SPI_W(index) REG32(SPI2_BASE + 0x98u + 4u * (index))
#define SPI_SLAVE REG32(SPI2_BASE + 0xE0u)
#define SPI_CLK_GATE REG32(SPI2_BASE + 0xE8u)

#define GPIO_OUT_FUNC 256u
#define PAD_MATRIX ((1u << 12) | (2u << 10) | (1u << 9))
#define SPI_UPDATE (1u << 23)
#define SPI_USR (1u << 24)
#define TXFIFO_LEN 128u

static inline void gpio_write(uint32_t pin, uint32_t high) {
  if (pin < 32u) {
    const uint32_t mask = 1u << pin;
    if (high) {
      GPIO_OUT_W1TS = mask;
    } else {
      GPIO_OUT_W1TC = mask;
    }
  } else {
    const uint32_t mask = 1u << (pin - 32u);
    if (high) {
      GPIO_OUT1_W1TS = mask;
    } else {
      GPIO_OUT1_W1TC = mask;
    }
  }
}

static inline void gpio_enable(uint32_t pin) {
  if (pin < 32u) {
    GPIO_ENABLE_W1TS = 1u << pin;
  } else {
    GPIO_ENABLE1_W1TS = 1u << (pin - 32u);
  }
}

static inline void route_gpio_output(uint32_t pin, uint32_t signal) {
  IO_MUX(pin) = PAD_MATRIX;
  GPIO_FUNC_OUT_SEL(pin) = signal;
  gpio_enable(pin);
}

static inline void route_peripheral_output(uint32_t pin, uint32_t signal) {
  IO_MUX(pin) = PAD_MATRIX;
  GPIO_FUNC_OUT_SEL(pin) = signal;
  gpio_enable(pin);
}

static void spi_write(const uint8_t *bytes, uint32_t length) {
  while (length != 0u) {
    uint32_t chunk = length > 64u ? 64u : length;
    for (uint32_t word_index = 0; word_index < (chunk + 3u) / 4u; ++word_index) {
      uint32_t word = 0u;
      for (uint32_t byte_index = 0; byte_index < 4u; ++byte_index) {
        uint32_t offset = word_index * 4u + byte_index;
        if (offset < chunk) {
          word |= (uint32_t)bytes[offset] << (byte_index * 8u);
        }
      }
      SPI_W(word_index) = word;
    }

    SPI_MS_DLEN = chunk * 8u - 1u;
    SPI_CMD = SPI_UPDATE;
    while ((SPI_CMD & SPI_UPDATE) != 0u) {
    }
    gpio_write(7u, 0u); // TFT_CS, active low
    SPI_CMD = SPI_USR;
    while ((SPI_CMD & SPI_USR) != 0u) {
    }
    gpio_write(7u, 1u);
    bytes += chunk;
    length -= chunk;
  }
}

static void tft_command(uint8_t command) {
  gpio_write(39u, 0u); // TFT D/C, command
  spi_write(&command, 1u);
}

static void tft_data(const uint8_t *bytes, uint32_t length) {
  gpio_write(39u, 1u); // TFT D/C, data
  spi_write(bytes, length);
}

static void delay_cycles(uint32_t count) {
  for (volatile uint32_t i = 0u; i < count; ++i) {
    __asm__ volatile("nop" ::: "memory");
  }
}

static void tft_window(uint16_t x0, uint16_t y0, uint16_t x1, uint16_t y1) {
  const uint8_t column[] = {
      (uint8_t)(x0 >> 8), (uint8_t)x0, (uint8_t)(x1 >> 8), (uint8_t)x1};
  const uint8_t row[] = {
      (uint8_t)(y0 >> 8), (uint8_t)y0, (uint8_t)(y1 >> 8), (uint8_t)y1};
  tft_command(0x2Au); // CASET
  tft_data(column, sizeof(column));
  tft_command(0x2Bu); // RASET
  tft_data(row, sizeof(row));
}

__attribute__((noreturn, used, section(".entry")))
void _start(void) {
  static const uint8_t marker[] =
      "ADAFRUIT_ESP32S3_TFT_FEATHER_ST7789_OK\n";
  static const uint8_t madctl = 0xA0u; // MV|MY, Adafruit rotation 1
  static const uint8_t colmod = 0x55u; // RGB565
  static const uint8_t red_patch[8u * 8u * 2u] = {
      0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u,
      0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u,
      0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u,
      0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u,
      0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u,
      0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u,
      0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u,
      0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u,
      0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u,
      0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u,
      0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u,
      0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u,
      0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u,
      0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u,
      0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u,
      0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u, 0xF8u, 0x00u,
  };

  // Leave the S3 SYSTEM clock/reset registers at their documented reset
  // values; this profile does not model dynamic SYSTEM clock gating.
  // Route the Feather's actual SCK/MOSI pads.
  route_peripheral_output(36u, 101u); // FSPICLK -> GPIO36 / SCK
  route_peripheral_output(35u, 103u); // FSPID -> GPIO35 / MOSI
  route_gpio_output(7u, GPIO_OUT_FUNC);
  route_gpio_output(21u, GPIO_OUT_FUNC);
  route_gpio_output(39u, GPIO_OUT_FUNC);
  route_gpio_output(40u, GPIO_OUT_FUNC);
  route_gpio_output(45u, GPIO_OUT_FUNC);

  gpio_write(7u, 1u);  // deselect panel
  gpio_write(21u, 0u); // panel/STEMMA rail off
  gpio_write(39u, 0u); // command level
  gpio_write(40u, 0u); // assert reset
  gpio_write(45u, 0u); // backlight off

  SPI_CLK_GATE = 7u;
  SPI_SLAVE = 0u;
  SPI_CLOCK = (15u << 18) | (49u << 12) | (24u << 6) | 49u;
  SPI_USER = (1u << 6) | (1u << 7) | (1u << 27); // CS hold/setup, MOSI
  SPI_USER1 = 0u;
  SPI_MISC = 0x3Eu;

  gpio_write(21u, 1u); // TFT_I2C_POWER, active high
  delay_cycles(5000u);
  gpio_write(40u, 1u); // release active-low reset
  delay_cycles(5000u);

  tft_command(0x01u); // SWRESET
  delay_cycles(5000u);
  tft_command(0x11u); // SLPOUT
  delay_cycles(5000u);
  tft_command(0x36u); // MADCTL
  tft_data(&madctl, 1u);
  tft_command(0x3Au); // COLMOD
  tft_data(&colmod, 1u);
  tft_window(52u, 40u, 59u, 47u);
  tft_command(0x2Cu); // RAMWR
  tft_data(red_patch, sizeof(red_patch));
  tft_command(0x29u); // DISPON
  gpio_write(45u, 1u); // TFT backlight, active high

  for (const uint8_t *byte = marker; *byte != 0u; ++byte) {
    for (uint32_t spin = 0u; spin < 100000u; ++spin) {
      if (((UART0_STATUS >> 16) & 0x3FFu) < TXFIFO_LEN) {
        break;
      }
    }
    UART0_FIFO = *byte;
  }

  for (;;) {
    __asm__ volatile("nop" ::: "memory");
  }
}
