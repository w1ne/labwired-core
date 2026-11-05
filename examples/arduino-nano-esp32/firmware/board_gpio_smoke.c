typedef unsigned int uint32_t;

#define GPIO_BASE 0x60004000u
#define GPIO_OUT_W1TS (*(volatile uint32_t *)(GPIO_BASE + 0x08u))
#define GPIO_OUT_W1TC (*(volatile uint32_t *)(GPIO_BASE + 0x0Cu))
#define GPIO_OUT1_W1TS (*(volatile uint32_t *)(GPIO_BASE + 0x14u))
#define GPIO_OUT1_W1TC (*(volatile uint32_t *)(GPIO_BASE + 0x18u))
#define GPIO_ENABLE_W1TS (*(volatile uint32_t *)(GPIO_BASE + 0x24u))
#define GPIO_ENABLE1_W1TS (*(volatile uint32_t *)(GPIO_BASE + 0x30u))
#define GPIO_FUNC0_OUT_SEL_CFG (*(volatile uint32_t *)0x60004554u)
#define GPIO_FUNC45_OUT_SEL_CFG (*(volatile uint32_t *)0x60004608u)
#define GPIO_FUNC46_OUT_SEL_CFG (*(volatile uint32_t *)0x6000460Cu)
#define GPIO_OUT_FUNC 256u

static inline void gpio_set(uint32_t pin, uint32_t high) {
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

static inline void pause(void) {
  __asm__ volatile("nop; nop; nop; nop" ::: "memory");
}

// Arduino's standard Nano ESP32 mapping exposes the active-low RGB contacts
// on GPIO46, GPIO0 and GPIO45. Some early units swap the green/blue contacts,
// so this smoke checks the electrical channels by GPIO number.
__attribute__((noreturn, used, section(".entry")))
void _start(void) {
  GPIO_FUNC0_OUT_SEL_CFG = GPIO_OUT_FUNC;
  GPIO_FUNC45_OUT_SEL_CFG = GPIO_OUT_FUNC;
  GPIO_FUNC46_OUT_SEL_CFG = GPIO_OUT_FUNC;
  gpio_enable(0u);
  gpio_enable(45u);
  gpio_enable(46u);

  for (;;) {
    // Active-low channels on.
    gpio_set(46u, 0u);
    gpio_set(0u, 0u);
    gpio_set(45u, 0u);
    pause();
    // Active-low channels off.
    gpio_set(46u, 1u);
    gpio_set(0u, 1u);
    gpio_set(45u, 1u);
    pause();
  }
}
