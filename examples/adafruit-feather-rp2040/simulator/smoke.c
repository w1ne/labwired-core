// Bare-metal RP2040 smoke image used by the local system simulator.
// The customer-facing Arduino example is in main.cpp; this image starts from
// the Cortex-M vector table so it can exercise I2C1 without Arduino's current
// timer-heavy startup path.

#include <stdint.h>

#define REG32(address) (*(volatile uint32_t *)(uintptr_t)(address))

enum {
  RESETS_BASE = 0x4000C000u,
  RESET_CLEAR = RESETS_BASE + 0x3000u,
  RESET_DONE = RESETS_BASE + 0x08u,
  RESET_I2C1 = 1u << 4,
  RESET_IO_BANK0 = 1u << 5,
  RESET_PADS_BANK0 = 1u << 8,
  BOARD_RESETS = RESET_I2C1 | RESET_IO_BANK0 | RESET_PADS_BANK0,
  IO_BANK0 = 0x40014000u,
  SIO = 0xD0000000u,
  I2C1 = 0x40048000u,
  GPIO_FUNC_I2C = 3u,
  GPIO_FUNC_SIO = 5u,
  GPIO_D13 = 1u << 13,
  I2C_READ = 1u << 8,
  I2C_STOP = 1u << 9,
  I2C_RESTART = 1u << 10,
  I2C_TX_ABORT = 1u << 6,
  I2C_STOP_DETECTED = 1u << 9,
};

static int read_bme280_chip_id(uint8_t *chip_id) {
  REG32(RESET_CLEAR) = BOARD_RESETS;
  uint32_t timeout;
  for (timeout = 0; timeout < 10000; ++timeout) {
    if ((REG32(RESET_DONE) & BOARD_RESETS) == BOARD_RESETS) {
      break;
    }
  }
  if (timeout == 10000) {
    return 0;
  }

  // Feather STEMMA QT uses I2C1 on GP2/GP3.
  REG32(IO_BANK0 + 0x04u + 8u * 2u) = GPIO_FUNC_I2C;
  REG32(IO_BANK0 + 0x04u + 8u * 3u) = GPIO_FUNC_I2C;

  // D13 is a header pin, not an onboard Feather LED. It rises only after the
  // sensor identity is confirmed, giving the smoke an observable GPIO result.
  REG32(IO_BANK0 + 0x04u + 8u * 13u) = GPIO_FUNC_SIO;
  REG32(SIO + 0x18u) = GPIO_D13; // GPIO_OUT_CLR
  REG32(SIO + 0x24u) = GPIO_D13; // GPIO_OE_SET

  // Standard-mode master, 7-bit target, repeated starts enabled.
  REG32(I2C1 + 0x6Cu) = 0;
  REG32(I2C1 + 0x00u) = 1u | (1u << 1) | (1u << 5) | (1u << 6) | (1u << 8);
  REG32(I2C1 + 0x14u) = 600;
  REG32(I2C1 + 0x18u) = 650;
  REG32(I2C1 + 0x04u) = 0x77;
  REG32(I2C1 + 0x6Cu) = 1;
  (void)REG32(I2C1 + 0x40u); // Clear any stale interrupt state.

  // Read one byte from BME280.CHIP_ID (0xD0) with repeated START and STOP.
  REG32(I2C1 + 0x10u) = 0xD0;
  REG32(I2C1 + 0x10u) = I2C_READ | I2C_RESTART | I2C_STOP;
  for (timeout = 0; timeout < 100000; ++timeout) {
    const uint32_t status = REG32(I2C1 + 0x34u);
    if ((status & I2C_TX_ABORT) != 0) {
      (void)REG32(I2C1 + 0x54u);
      return 0;
    }
    if ((status & I2C_STOP_DETECTED) != 0) {
      (void)REG32(I2C1 + 0x60u);
      *chip_id = (uint8_t)REG32(I2C1 + 0x10u);
      return 1;
    }
  }
  return 0;
}

void Reset_Handler(void) {
  uint8_t chip_id = 0;
  if (read_bme280_chip_id(&chip_id) && chip_id == 0x60) {
    REG32(SIO + 0x14u) = GPIO_D13; // GPIO_OUT_SET
  }
  for (;;) {
    __asm__ volatile("nop");
  }
}

__attribute__((section(".isr_vector"), used))
const uintptr_t vector_table[] = {
    0x20042000u,
    (uintptr_t)Reset_Handler,
};
