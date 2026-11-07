typedef unsigned int uint32_t;
typedef unsigned char uint8_t;

#define GPIO_OUT1_W1TS (*(volatile uint32_t *)0x60004014u)
#define GPIO_OUT1_W1TC (*(volatile uint32_t *)0x60004018u)
#define GPIO_ENABLE1_W1TS (*(volatile uint32_t *)0x60004030u)
#define GPIO_FUNC35_OUT_SEL_CFG (*(volatile uint32_t *)0x600045E0u)

#define GPIO35_BIT (1u << 3)

static inline void delay_70_cycles(void) {
  __asm__ volatile(".rept 70\n nop\n .endr" ::: "memory");
}

static inline void delay_120_cycles(void) {
  __asm__ volatile(".rept 120\n nop\n .endr" ::: "memory");
}

static inline void delay_170_cycles(void) {
  __asm__ volatile(".rept 170\n nop\n .endr" ::: "memory");
}

static inline void delay_220_cycles(void) {
  __asm__ volatile(".rept 220\n nop\n .endr" ::: "memory");
}

static inline void send_bit(uint32_t one) {
  GPIO_OUT1_W1TS = GPIO35_BIT;
  if (one) {
    delay_170_cycles();
  } else {
    delay_70_cycles();
  }
  GPIO_OUT1_W1TC = GPIO35_BIT;
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
  // GPIO35 is the AtomS3-Lite's onboard WS2812C-2020 data input. Select the
  // GPIO output path, enable bank 1, and send one red GRB pixel.
  GPIO_FUNC35_OUT_SEL_CFG = 256u;
  GPIO_ENABLE1_W1TS = GPIO35_BIT;
  GPIO_OUT1_W1TC = GPIO35_BIT;

  send_byte(0x00u);
  send_byte(0xFFu);
  send_byte(0x00u);

  // Hold low long enough for the WS2812 latch/reset interval.
  for (uint32_t i = 0; i < 40000u; ++i) {
    __asm__ volatile("nop" ::: "memory");
  }
  for (;;) {
    __asm__ volatile("nop" ::: "memory");
  }
}
