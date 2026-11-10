/* ESP32-S3-DevKitC-1 N16R8: sample the centered GPIO4 / ADC1_CH3 input. */
#include <stdint.h>

#define REG32(addr) (*(volatile uint32_t *)(uintptr_t)(addr))

enum {
  UART0_FIFO = 0x60000000u,
  SENS_SAR_MEAS1_CTRL2 = 0x6000880Cu,
  SAR_MEAS1_DONE = 1u << 16,
  SAR_MEAS1_START = 1u << 17,
  SAR1_EN_PAD_CH3 = 1u << (19 + 3),
};

static void uart_str(const char *text) {
  while (*text) REG32(UART0_FIFO) = (uint8_t)*text++;
}

__attribute__((noreturn, used, section(".entry")))
void _start(void) {
  /* Select ADC1 channel 3 (GPIO4) and start its SENS oneshot conversion. */
  REG32(SENS_SAR_MEAS1_CTRL2) = SAR1_EN_PAD_CH3 | SAR_MEAS1_START;
  const uint32_t result = REG32(SENS_SAR_MEAS1_CTRL2);
  const uint16_t raw = (uint16_t)(result & 0x0FFFu);

  if ((result & SAR_MEAS1_DONE) && raw == 2047u) {
    uart_str("ESP32_S3_DEVKITC1_ADC1_CH3_2047_OK\n");
  } else {
    uart_str("ESP32_S3_DEVKITC1_ADC1_CH3_FAIL\n");
  }

  for (;;) __asm__ volatile("nop" ::: "memory");
}
