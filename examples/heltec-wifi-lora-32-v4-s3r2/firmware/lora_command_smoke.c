#include <stdint.h>

#define REG32(address) (*(volatile uint32_t *)(address))

#define GPIO_BASE 0x60004000u
#define GPIO_IN REG32(GPIO_BASE + 0x3Cu)
#define GPIO_OUT_W1TS REG32(GPIO_BASE + 0x08u)
#define GPIO_OUT_W1TC REG32(GPIO_BASE + 0x0Cu)
#define GPIO_ENABLE_W1TS REG32(GPIO_BASE + 0x20u)
#define GPIO_FUNC_IN_SEL(signal) REG32(GPIO_BASE + 0x154u + 4u * (signal))
#define GPIO_FUNC_OUT_SEL(pin) REG32(GPIO_BASE + 0x554u + 4u * (pin))
#define IO_MUX(pin) REG32(0x60009004u + 4u * (pin))

#define UART0_FIFO REG32(0x60000000u)
#define UART0_STATUS REG32(0x6000001Cu)

#define SPI3_BASE 0x60025000u
#define SPI_CMD REG32(SPI3_BASE + 0x00u)
#define SPI_CLOCK REG32(SPI3_BASE + 0x0Cu)
#define SPI_USER REG32(SPI3_BASE + 0x10u)
#define SPI_USER1 REG32(SPI3_BASE + 0x14u)
#define SPI_MS_DLEN REG32(SPI3_BASE + 0x1Cu)
#define SPI_MISC REG32(SPI3_BASE + 0x20u)
#define SPI_W(index) REG32(SPI3_BASE + 0x98u + 4u * (index))
#define SPI_SLAVE REG32(SPI3_BASE + 0xE0u)
#define SPI_CLK_GATE REG32(SPI3_BASE + 0xE8u)

#define GPIO_OUTPUT_FUNCTION 256u
#define GPIO_MATRIX_INPUT (1u << 7)
#define PAD_MATRIX ((1u << 12) | (2u << 10) | (1u << 9))
#define SPI_UPDATE (1u << 23)
#define SPI_USR (1u << 24)

static inline void gpio_output(uint32_t pin) {
  IO_MUX(pin) = PAD_MATRIX;
  GPIO_FUNC_OUT_SEL(pin) = GPIO_OUTPUT_FUNCTION;
  GPIO_ENABLE_W1TS = 1u << pin;
}

static inline void route_spi3_output(uint32_t pin, uint32_t signal) {
  IO_MUX(pin) = PAD_MATRIX;
  GPIO_FUNC_OUT_SEL(pin) = signal;
  GPIO_ENABLE_W1TS = 1u << pin;
}

static inline void route_spi3_input(uint32_t pin, uint32_t signal) {
  IO_MUX(pin) = PAD_MATRIX;
  GPIO_FUNC_IN_SEL(signal) = GPIO_MATRIX_INPUT | pin;
}

static void uart_write(const char *message) {
  for (; *message != '\0'; ++message) {
    for (uint32_t spin = 0u; spin < 100000u; ++spin) {
      if (((UART0_STATUS >> 16) & 0x3FFu) < 128u) {
        break;
      }
    }
    UART0_FIFO = (uint8_t)*message;
  }
}

// One low NSS transaction per SX1262 command. The controller's full-duplex W
// buffer supplies the status and command result bytes returned on MISO.
static void spi3_transfer(const uint8_t *tx, uint8_t *rx, uint32_t length) {
  uint32_t words = (length + 3u) / 4u;
  for (uint32_t word_index = 0u; word_index < words; ++word_index) {
    uint32_t word = 0u;
    for (uint32_t byte_index = 0u; byte_index < 4u; ++byte_index) {
      uint32_t offset = word_index * 4u + byte_index;
      if (offset < length) {
        word |= (uint32_t)tx[offset] << (byte_index * 8u);
      }
    }
    SPI_W(word_index) = word;
  }

  SPI_MS_DLEN = length * 8u - 1u;
  SPI_CMD = SPI_UPDATE;
  while ((SPI_CMD & SPI_UPDATE) != 0u) {
  }
  SPI_CMD = SPI_USR;
  while ((SPI_CMD & SPI_USR) != 0u) {
  }

  for (uint32_t byte_index = 0u; byte_index < length; ++byte_index) {
    uint32_t word = SPI_W(byte_index / 4u);
    rx[byte_index] = (uint8_t)(word >> ((byte_index % 4u) * 8u));
  }
}

static void spi3_init(void) {
  // Heltec WiFi LoRa 32 V4.3: NSS=8, SCK=9, MOSI=10, MISO=11.
  route_spi3_output(8u, 71u);  // SPI3_CS0
  route_spi3_output(9u, 66u);  // SPI3_CLK
  route_spi3_output(10u, 68u); // SPI3_D (MOSI)
  route_spi3_input(11u, 67u);  // SPI3_Q (MISO)

  SPI_CLK_GATE = 7u;
  SPI_SLAVE = 0u;
  SPI_CLOCK = (15u << 18) | (49u << 12) | (24u << 6) | 49u;
  SPI_USER = (1u << 0) | (1u << 6) | (1u << 7) | (1u << 27) | (1u << 28);
  SPI_USER1 = 0u;
  SPI_MISC = 0x3Eu;
}

__attribute__((noreturn, used, section(".entry")))
void _start(void) {
  uint8_t rx[9];

  // The V4 board's SX1262 is on a dedicated GP-SPI3 bus. GPIO13 and GPIO14
  // are the radio BUSY and DIO1 pins; reset sequencing is outside this profile.
  gpio_output(8u);
  gpio_output(9u);
  gpio_output(10u);
  spi3_init();

  static const uint8_t set_lora_packet_type[] = {0x8Au, 0x01u};
  spi3_transfer(set_lora_packet_type, rx, sizeof(set_lora_packet_type));

  // GetPacketType: response byte 2 must be LoRa (0x01).
  static const uint8_t get_packet_type[] = {0x11u, 0x00u, 0x00u};
  spi3_transfer(get_packet_type, rx, sizeof(get_packet_type));
  uint8_t packet_type = rx[2];

  // Enable TxDone on DIO1 (IRQ bit 0), then ask the behavioral radio model to
  // complete SetTx. The twin reports TxDone immediately; it has no RF link.
  static const uint8_t enable_tx_done_irq[] = {
      0x08u, 0x00u, 0x01u, 0x00u, 0x01u, 0x00u, 0x00u, 0x00u, 0x00u};
  spi3_transfer(enable_tx_done_irq, rx, sizeof(enable_tx_done_irq));
  static const uint8_t set_tx[] = {0x83u, 0x00u, 0x00u, 0x00u};
  spi3_transfer(set_tx, rx, sizeof(set_tx));

  // GetIrqStatus: IRQ[15:8] and IRQ[7:0] arrive in response bytes 2 and 3.
  static const uint8_t get_irq_status[] = {0x12u, 0x00u, 0x00u, 0x00u};
  spi3_transfer(get_irq_status, rx, sizeof(get_irq_status));
  uint16_t irq_status = ((uint16_t)rx[2] << 8) | rx[3];
  uint32_t dio1_high = (GPIO_IN & (1u << 14)) != 0u;

  if (packet_type == 0x01u && (irq_status & 0x0001u) != 0u && dio1_high) {
    uart_write("HELTEC_SX1262_OK\n");
  } else if (packet_type != 0x01u) {
    uart_write("SX1262_PACKET_FAIL\n");
  } else if ((irq_status & 0x0001u) == 0u) {
    uart_write("SX1262_IRQ_FAIL\n");
  } else {
    uart_write("SX1262_DIO1_FAIL\n");
  }

  for (;;) {
    __asm__ volatile("nop" ::: "memory");
  }
}
