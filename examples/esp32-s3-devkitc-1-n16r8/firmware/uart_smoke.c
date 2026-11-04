typedef unsigned int uint32_t;
typedef unsigned char uint8_t;

#define UART0_FIFO (*(volatile uint32_t *)0x60000000u)
#define UART0_STATUS (*(volatile uint32_t *)0x6000001Cu)
#define TXFIFO_LEN 128u

__attribute__((noreturn, used, section(".entry")))
void _start(void) {
  static const uint8_t marker[] = "ESP32_S3_DEVKITC1_N16R8_UART0_OK\r\n";

  for (const uint8_t *byte = marker; *byte != 0; ++byte) {
    // Bound the wait so a model gap cannot turn the smoke into a silent hang.
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
