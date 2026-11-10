/* ESP32-S3-DevKitC-1 v1.0 N16R8: UART0, GPIO48 WS2812, and GPIO4 MCPWM servo. */
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
#define MCPWM0_CLK_CFG (*(volatile uint32_t *)0x6001E000u)
#define MCPWM0_TIMER0_CFG0 (*(volatile uint32_t *)0x6001E004u)
#define MCPWM0_TIMER0_CFG1 (*(volatile uint32_t *)0x6001E008u)
#define MCPWM0_GEN0_TSTMP_A (*(volatile uint32_t *)0x6001E040u)
#define MCPWM0_GEN0_A (*(volatile uint32_t *)0x6001E050u)

#define GPIO4_BIT (1u << 4)
#define GPIO48_BIT (1u << 16)
#define MCPWM0_OUT0A 160u
#define RMT_SIG_OUT0 81u
#define RMT_TX_START (1u << 0)
#define RMT_CONF_UPDATE (1u << 24)
#define RMT_CH0_TX_END (1u << 0)
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
  static const uint8_t marker[] = "ESP32_S3_DEVKITC1_V10_MCPWM_SERVO_OK\n";

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
   * MCPWM uses CLK_160M / 16 / 10 = 1 MHz; up-count period 19999 is 50 Hz.
   * GPIO4 is J1 pin 4; GPIO matrix selector 160 is MCPWM0 PWM0A. The
   * generator drives high at TEZ and low at compare A=1500 for a 1.5 ms pulse.
   */
  GPIO_ENABLE_W1TS = GPIO4_BIT;
  GPIO_FUNC4_OUT_SEL_CFG = MCPWM0_OUT0A;
  MCPWM0_CLK_CFG = 15u; /* CLK_160M / 16 = 10 MHz */
  MCPWM0_GEN0_TSTMP_A = 1500u;
  MCPWM0_GEN0_A = 0x12u; /* up-count TEZ: high; TEA: low */
  MCPWM0_TIMER0_CFG0 = 9u | (19999u << 8); /* 10 MHz / 10, period=20 ms */
  MCPWM0_TIMER0_CFG1 = 1u | (2u << 3); /* up-count, start continuous */

  /* Do not report success unless the modeled MCPWM committed compare A. */
  if (MCPWM0_GEN0_TSTMP_A != 1500u) {
    for (;;) {
      __asm__ volatile("nop" ::: "memory");
    }
  }
  uart_write(marker);

  /* Leave the 50 Hz waveform running for the smoke's GPIO edge oracle. */
  for (;;) {
    __asm__ volatile("nop" ::: "memory");
  }
}
