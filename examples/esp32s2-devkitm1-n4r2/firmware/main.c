typedef unsigned int uint32_t;
typedef unsigned char uint8_t;

#define UART0_FIFO (*(volatile uint32_t *)0x60000000u)
#define GPIO_OUT_W1TS (*(volatile uint32_t *)0x60004008u)
#define GPIO_OUT_W1TC (*(volatile uint32_t *)0x6000400Cu)
#define GPIO_ENABLE_W1TS (*(volatile uint32_t *)0x60004024u)
#define GPIO_FUNC18_OUT_SEL_CFG (*(volatile uint32_t *)0x6000459Cu)
#define GPIO18_MASK (1u << 18)

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
  GPIO_OUT_W1TS = GPIO18_MASK;
  if (one) {
    delay_170_cycles();
  } else {
    delay_70_cycles();
  }
  GPIO_OUT_W1TC = GPIO18_MASK;
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
  static const uint8_t marker[] = "ESP32_S2_DEVKITM1_N4R2_OK\n";

  // ESP32-S2 and S3 share this GPIO output register head. Select direct GPIO
  // output for pad 18, enable it, and send GRB for a solid red WS2812 pixel.
  GPIO_FUNC18_OUT_SEL_CFG = 256u;
  GPIO_ENABLE_W1TS = GPIO18_MASK;
  GPIO_OUT_W1TC = GPIO18_MASK;
  send_byte(0x00u);
  send_byte(0xFFu);
  send_byte(0x00u);
  for (uint32_t i = 0; i < 10000u; ++i) {
    __asm__ volatile("nop" ::: "memory");
  }

  for (const uint8_t *p = marker; *p != 0; ++p) {
    UART0_FIFO = *p;
  }
  for (;;) {
    __asm__ volatile("nop" ::: "memory");
  }
}
