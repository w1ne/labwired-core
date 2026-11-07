typedef unsigned int uint32_t;
typedef unsigned char uint8_t;

#define UART0_FIFO (*(volatile uint32_t *)0x60000000u)
#define UART0_STATUS (*(volatile uint32_t *)0x6000001Cu)
#define GPIO_OUT1_W1TC (*(volatile uint32_t *)0x60004018u)
#define GPIO_ENABLE1_W1TS (*(volatile uint32_t *)0x60004030u)
#define GPIO_FUNC38_OUT_SEL_CFG (*(volatile uint32_t *)0x600045ECu)
#define RMT_INT_RAW (*(volatile uint32_t *)0x60016070u)
#define RMT_INT_CLR (*(volatile uint32_t *)0x6001607Cu)
#define RMT_CH0_CONF0 (*(volatile uint32_t *)0x60016020u)
#define RMT_MEM ((volatile uint32_t *)0x60016400u)

#define GPIO38_BIT (1u << 6)
#define RMT_SIG_OUT0 81u
#define RMT_TX_START (1u << 0)
#define RMT_CONF_UPDATE (1u << 24)
#define RMT_CH0_TX_END (1u << 0)
#define TXFIFO_LEN 128u

static void write_ws2812_byte(uint8_t value, uint32_t *symbol_index) {
  for (uint32_t mask = 0x80u; mask != 0; mask >>= 1) {
    // At the S3's default 80 MHz RMT source, 28/56 ticks give the WS2812
    // short/long HIGH pulse; each bit lasts 100 ticks (1.25 us).
    const uint32_t high = (value & mask) != 0 ? 56u : 28u;
    const uint32_t low = 100u - high;
    RMT_MEM[*symbol_index] = high | (1u << 15) | (low << 16);
    ++*symbol_index;
  }
}

static inline void delay_reset_gap(void) {
  // >40 us at the 240 MHz simulated CPU clock, so the LED model sees a reset
  // gap after the RMT playback completes.
  for (uint32_t i = 0; i < 30000u; ++i) {
    __asm__ volatile("nop" ::: "memory");
  }
}

__attribute__((noreturn, used, section(".entry")))
void _start(void) {
  static const uint8_t marker[] = "ESP32_S3_DEVKITC1_V11_RMT_OK\n";

  // The v1.1 board profile attaches its WS2812 model to GPIO38. Route RMT TX
  // channel 0 to that pad, enable the output driver, and prepare a red GRB
  // frame in the channel's direct symbol-RAM window.
  GPIO_ENABLE1_W1TS = GPIO38_BIT;
  GPIO_OUT1_W1TC = GPIO38_BIT;
  GPIO_FUNC38_OUT_SEL_CFG = RMT_SIG_OUT0;
  RMT_INT_CLR = RMT_CH0_TX_END;

  uint32_t symbol_index = 0;
  write_ws2812_byte(0x00u, &symbol_index); // Green
  write_ws2812_byte(0xFFu, &symbol_index); // Red
  write_ws2812_byte(0x00u, &symbol_index); // Blue
  RMT_MEM[symbol_index] = 0;               // End marker

  // Divider 1, one memory block, and an enabled low idle output.
  const uint32_t ch0_conf = (1u << 8) | (1u << 16) | (1u << 6);
  RMT_CH0_CONF0 = ch0_conf | RMT_CONF_UPDATE;
  RMT_CH0_CONF0 = ch0_conf | RMT_TX_START;
  uint32_t tx_complete = 0;
  for (uint32_t spin = 0; spin < 100000u; ++spin) {
    if ((RMT_INT_RAW & RMT_CH0_TX_END) != 0) {
      tx_complete = 1;
      break;
    }
  }
  if (tx_complete == 0) {
    for (;;) {
      __asm__ volatile("nop" ::: "memory");
    }
  }
  RMT_INT_CLR = RMT_CH0_TX_END;

  delay_reset_gap();

  for (const uint8_t *byte = marker; *byte != 0; ++byte) {
    for (uint32_t spin = 0; spin < 100000u; ++spin) {
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
