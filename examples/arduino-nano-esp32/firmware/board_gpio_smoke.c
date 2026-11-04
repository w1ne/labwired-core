typedef unsigned int uint32_t;

#define GPIO_BASE 0x60004000u
#define GPIO_OUT1_W1TS (*(volatile uint32_t *)(GPIO_BASE + 0x14u))
#define GPIO_OUT1_W1TC (*(volatile uint32_t *)(GPIO_BASE + 0x18u))
#define GPIO_ENABLE1_W1TS (*(volatile uint32_t *)(GPIO_BASE + 0x30u))

// GPIO46 is bit 14 in the ESP32-S3 GPIO bank 1 and drives the Nano ESP32's
// active-low red RGB LED channel.
__attribute__((noreturn, used, section(".entry")))
void _start(void) {
  const uint32_t gpio46 = 1u << (46u - 32u);
  GPIO_ENABLE1_W1TS = gpio46;
  GPIO_OUT1_W1TS = gpio46;

  for (;;) {
    GPIO_OUT1_W1TC = gpio46;
    __asm__ volatile("nop; nop; nop" ::: "memory");
    GPIO_OUT1_W1TS = gpio46;
  }
}
