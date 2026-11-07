#include <Arduino.h>

#define GPIO_OUT1_W1TS (*(volatile uint32_t *)0x60004014u)
#define GPIO_OUT1_W1TC (*(volatile uint32_t *)0x60004018u)
#define GPIO_ENABLE1_W1TS (*(volatile uint32_t *)0x60004030u)
#define GPIO_FUNC38_OUT_SEL_CFG (*(volatile uint32_t *)0x600045ECu)
#define GPIO38_BIT (1u << 6)

static inline void delay70() {
  __asm__ volatile(".rept 70\n nop\n .endr" ::: "memory");
}

static inline void delay170() {
  __asm__ volatile(".rept 170\n nop\n .endr" ::: "memory");
}

static inline void delay120() {
  __asm__ volatile(".rept 120\n nop\n .endr" ::: "memory");
}

static inline void delay220() {
  __asm__ volatile(".rept 220\n nop\n .endr" ::: "memory");
}

static inline void sendBit(bool one) {
  GPIO_OUT1_W1TS = GPIO38_BIT;
  if (one) {
    delay170();
  } else {
    delay70();
  }
  GPIO_OUT1_W1TC = GPIO38_BIT;
  if (one) {
    delay120();
  } else {
    delay220();
  }
}

static inline void sendByte(uint8_t value) {
  for (uint8_t mask = 0x80; mask; mask >>= 1) {
    sendBit((value & mask) != 0);
  }
}

void setup() {
  Serial0.begin(115200);
  GPIO_FUNC38_OUT_SEL_CFG = 256u;
  GPIO_ENABLE1_W1TS = GPIO38_BIT;
  GPIO_OUT1_W1TC = GPIO38_BIT;
  sendByte(0x00);  // Green
  sendByte(0xFF);  // Red
  sendByte(0x00);  // Blue
  for (volatile uint32_t i = 0; i < 30000; ++i) {
    __asm__ volatile("nop");
  }
  Serial0.println("ESP32_S3_DEVKITC1_V11_N16R8_OK");
}

void loop() {}
