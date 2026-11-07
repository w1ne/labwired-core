#include <Arduino.h>

typedef unsigned int u32;
typedef unsigned char u8;
#define REG32(a) (*(volatile u32 *)(a))
#define GPIO_OUT1_W1TC REG32(0x60004018u)
#define GPIO_ENABLE1_W1TS REG32(0x60004030u)
#define GPIO_FUNC48_OUT_SEL_CFG REG32(0x60004614u)
#define RMT_INT_RAW REG32(0x60016070u)
#define RMT_INT_CLR REG32(0x6001607Cu)
#define RMT_CH0_CONF0 REG32(0x60016020u)
#define RMT_MEM ((volatile u32 *)0x60016400u)
#define GPIO48_BIT (1u << 16)
#define RMT_SIG_OUT0 81u
#define RMT_TX_START (1u << 0)
#define RMT_CONF_UPDATE (1u << 24)
#define RMT_CH0_TX_END (1u << 0)

static void write_byte(u8 value, u32 *index) {
  for (u32 mask = 0x80u; mask != 0; mask >>= 1) {
    const u32 high = (value & mask) ? 56u : 28u;
    const u32 low = 100u - high;
    RMT_MEM[*index] = high | (1u << 15) | (low << 16);
    ++*index;
  }
}

void setup() {
  Serial0.begin(115200);
  GPIO_ENABLE1_W1TS = GPIO48_BIT;
  GPIO_OUT1_W1TC = GPIO48_BIT;
  GPIO_FUNC48_OUT_SEL_CFG = RMT_SIG_OUT0;
  RMT_INT_CLR = RMT_CH0_TX_END;

  u32 index = 0;
  write_byte(0x00u, &index); // Green
  write_byte(0xFFu, &index); // Red
  write_byte(0x00u, &index); // Blue
  RMT_MEM[index] = 0;

  const u32 conf = (1u << 8) | (1u << 16) | (1u << 6);
  RMT_CH0_CONF0 = conf | RMT_CONF_UPDATE;
  RMT_CH0_CONF0 = conf | RMT_TX_START;
  for (u32 spin = 0; spin < 100000u; ++spin) {
    if ((RMT_INT_RAW & RMT_CH0_TX_END) != 0) {
      RMT_INT_CLR = RMT_CH0_TX_END;
      delayMicroseconds(100);
      Serial0.println("ESP32_S3_DEVKITC1_V10_RMT_OK");
      return;
    }
  }
  Serial0.println("ESP32_S3_DEVKITC1_RMT_TIMEOUT");
}

void loop() { delay(1000); }
