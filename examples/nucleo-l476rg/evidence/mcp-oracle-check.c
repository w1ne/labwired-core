#include <stdint.h>
#define REG32(a) (*(volatile uint32_t *)(a))
int main(void) {
  REG32(0x4002104C) |= 1u;
  REG32(0x40021058) |= (1u << 17);
  REG32(0x48000000) = (REG32(0x48000000) & ~((3u << 4) | (3u << 10))) | (2u << 4) | (1u << 10);
  REG32(0x48000020) = (REG32(0x48000020) & ~(15u << 8)) | (7u << 8);
  REG32(0x4000440C) = 35u;
  REG32(0x40004400) = 9u;
  const char *s = "L476_BOARD_PATH_OK\\r\\n";
  for (; *s; ++s) {
    while ((REG32(0x4000441C) & (1u << 7)) == 0) {}
    REG32(0x40004428) = (uint8_t)*s;
  }
  REG32(0x48000014) |= (1u << 5);
  for (volatile uint32_t i = 0; i < 1000; ++i) {}
  REG32(0x48000014) &= ~(1u << 5);
  for (;;) { __asm volatile ("wfi"); }
}
