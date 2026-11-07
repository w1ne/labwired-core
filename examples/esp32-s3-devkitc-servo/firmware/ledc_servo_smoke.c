/* ESP32-S3-DevKitC-1 v1.0 N16R8: UART0, GPIO48 WS2812, and GPIO4 LEDC servo. */
typedef unsigned int uint32_t;
typedef unsigned char uint8_t;

#define UART0_FIFO (*(volatile uint32_t *)0x60000000u)
#define UART0_STATUS (*(volatile uint32_t *)0x6000001Cu)
#define GPIO_ENABLE_W1TS (*(volatile uint32_t *)0x60004020u)
#define GPIO_ENABLE1_W1TS (*(volatile uint32_t *)0x60004030u)
#define GPIO_FUNC4_OUT_SEL_CFG (*(volatile uint32_t *)0x60004564u)
#define GPIO_FUNC48_OUT_SEL_CFG (*(volatile uint32_t *)0x60004614u)
#define RMT_INT_RAW (*(volatile uint32_t *)0x60016070u)
#define RMT_INT_CLR (*(volatile uint32_t *)0x6001607Cu)
#define RMT_CH0_CONF0 (*(volatile uint32_t *)0x60016020u)
#define RMT_MEM ((volatile uint32_t *)0x60016400u)
#define LEDC_CH3_CONF0 (*(volatile uint32_t *)0x6001903Cu)
#define LEDC_CH3_HPOINT (*(volatile uint32_t *)0x60019040u)
#define LEDC_CH3_DUTY (*(volatile uint32_t *)0x60019044u)
#define LEDC_CH3_CONF1 (*(volatile uint32_t *)0x60019048u)
#define LEDC_CH3_DUTY_R (*(volatile uint32_t *)0x6001904Cu)
#define LEDC_TIMER2_CONF (*(volatile uint32_t *)0x600190B0u)

#define GPIO4_BIT (1u << 4)
#define GPIO48_BIT (1u << 16)
#define LEDC_SIG_OUT3 76u
#define RMT_SIG_OUT0 81u
#define RMT_TX_START (1u << 0)
#define RMT_CONF_UPDATE (1u << 24)
#define RMT_CH0_TX_END (1u << 0)
#define LEDC_DUTY_START (1u << 31)
#define TXFIFO_LEN 128u

static void write_ws2812_byte(uint8_t value, uint32_t *symbol_index) {
  for (uint32_t mask = 0x80u; mask != 0; mask >>= 1) {
    const uint32_t high = (value & mask) != 0 ? 56u : 28u;
    const uint32_t low = 100u - high;
    RMT_MEM[*symbol_index] = high | (1u << 15) | (low << 16);
    ++*symbol_index;
  }
}

static void uart_write(const uint8_t *bytes) {
  for (const uint8_t *byte = bytes; *byte != 0; ++byte) {
    for (uint32_t spin = 0; spin < 100000u; ++spin) {
      if (((UART0_STATUS >> 16) & 0x3FFu) < TXFIFO_LEN) {
        break;
      }
    }
    UART0_FIFO = *byte;
  }
}

__attribute__((noreturn, used, section(".entry")))
void _start(void) {
  static const uint8_t marker[] = "ESP32_S3_DEVKITC1_V10_LEDC_SERVO_OK\n";

  /* v1.0 DevKitC-1 routes its onboard WS2812 DIN to GPIO48. */
  GPIO_ENABLE1_W1TS = GPIO48_BIT;
  GPIO_FUNC48_OUT_SEL_CFG = RMT_SIG_OUT0;
  RMT_INT_CLR = RMT_CH0_TX_END;
  uint32_t symbol_index = 0;
  write_ws2812_byte(0x00u, &symbol_index);
  write_ws2812_byte(0xFFu, &symbol_index);
  write_ws2812_byte(0x00u, &symbol_index);
  RMT_MEM[symbol_index] = 0;
  const uint32_t rmt_conf = (1u << 8) | (1u << 16) | (1u << 6);
  RMT_CH0_CONF0 = rmt_conf | RMT_CONF_UPDATE;
  RMT_CH0_CONF0 = rmt_conf | RMT_TX_START;
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

  /*
   * 50 Hz, 14-bit LEDC on REF_TICK (1 MHz). Divider 313 gives 49.92 Hz:
   * 1 MHz * 256 / (313 * 2^14). GPIO4 is J1 pin 4; signal 76 is LEDC ch3.
   * 1187 / 16384 of the period is 1449 us, the SG90 model's 90-degree pulse.
   */
  GPIO_ENABLE_W1TS = GPIO4_BIT;
  GPIO_FUNC4_OUT_SEL_CFG = LEDC_SIG_OUT3;
  LEDC_TIMER2_CONF = 14u | (313u << 4) | (1u << 24);
  LEDC_CH3_CONF0 = 2u | (1u << 2); /* timer 2 and output enabled */
  LEDC_CH3_HPOINT = 0;
  LEDC_CH3_DUTY = 1187u << 4;
  LEDC_CH3_CONF1 = LEDC_DUTY_START;

  /* Do not report success unless the modeled LEDC path committed its duty. */
  if (LEDC_CH3_DUTY_R != (1187u << 4)) {
    for (;;) {
      __asm__ volatile("nop" ::: "memory");
    }
  }
  uart_write(marker);

  /* Leave the 49.92 Hz waveform running for the smoke's GPIO edge oracle. */
  for (;;) {
    __asm__ volatile("nop" ::: "memory");
  }
}
