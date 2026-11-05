typedef unsigned int uint32_t;
typedef unsigned char uint8_t;

#define UART0_FIFO (*(volatile uint32_t *)0x60000000u)
#define UART0_STATUS (*(volatile uint32_t *)0x6000001Cu)
#define GPIO_OUT1_W1TS (*(volatile uint32_t *)0x60004014u)
#define GPIO_OUT1_W1TC (*(volatile uint32_t *)0x60004018u)
#define GPIO_ENABLE1_W1TS (*(volatile uint32_t *)0x60004030u)
#define GPIO_FUNC38_OUT_SEL_CFG (*(volatile uint32_t *)0x600045ECu)
#define GPIO38_BIT (1u << 6)
#define TXFIFO_LEN 128u

static inline void delay_70_cycles(void) {
  __asm__ volatile(".rept 70\n nop\n .endr" ::: "memory");
}

static inline void delay_170_cycles(void) {
  __asm__ volatile(".rept 170\n nop\n .endr" ::: "memory");
}

static inline void delay_120_cycles(void) {
  __asm__ volatile(".rept 120\n nop\n .endr" ::: "memory");
}

static inline void delay_220_cycles(void) {
  __asm__ volatile(".rept 220\n nop\n .endr" ::: "memory");
}

static inline void send_bit(uint32_t one) {
  GPIO_OUT1_W1TS = GPIO38_BIT;
  if (one) {
    delay_170_cycles();
  } else {
    delay_70_cycles();
  }
  GPIO_OUT1_W1TC = GPIO38_BIT;
  if (one) {
    delay_120_cycles();
  } else {
    delay_220_cycles();
  }
}

static inline void send_byte(uint8_t value) {
  for (uint32_t mask = 0x80u; mask != 0; mask >>= 1) {
    send_bit((value & mask) != 0);
  }
}

__attribute__((noreturn, used, section(".entry")))
void _start(void) {
  static const uint8_t marker[] = "ESP32_S3_DEVKITC1_V11_N16R8_OK\n";

  // The v1.1 DevKitC-1 wires its addressable RGB LED to GPIO38. GPIO38 is in
  // the S3's second GPIO bank (bit 6); select the direct GPIO output path and
  // send a WS2812 GRB frame for solid red.
  GPIO_FUNC38_OUT_SEL_CFG = 256u;
  GPIO_ENABLE1_W1TS = GPIO38_BIT;
  GPIO_OUT1_W1TC = GPIO38_BIT;
  send_byte(0x00u);
  send_byte(0xFFu);
  send_byte(0x00u);
  for (uint32_t i = 0; i < 30000u; ++i) {
    __asm__ volatile("nop" ::: "memory");
  }

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
