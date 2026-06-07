# LabWired Firmware Scaffolds

This directory contains reference startup code and linker scripts so an agent (or a human) can compile a bare-metal ELF that boots correctly on the LabWired digital twin.

LabWired hosts the **run** side only: you compile in your own sandbox (or locally), then upload the ELF via `labwired_run`. LabWired returns the simulation result plus a hardware-level failure diagnosis.

---

## stm32l476

### Files

| File | Purpose |
|------|---------|
| `stm32l476/startup.c` | Minimal Cortex-M4 startup: copies `.data` flash→RAM, zeroes `.bss`, calls `main()`. |
| `stm32l476/stm32l476.ld` | Linker script: FLASH at `0x08000000` (1 MB), RAM at `0x20000000` (96 KB). |

### Compile command

```bash
arm-none-eabi-gcc \
  -mcpu=cortex-m4 -mthumb \
  -ffreestanding -nostdlib \
  -ffunction-sections -fdata-sections \
  -O1 -g \
  -T stm32l476/stm32l476.ld \
  stm32l476/startup.c your_firmware.c \
  -Wl,--gc-sections \
  -o firmware.elf
```

For C++:

```bash
arm-none-eabi-g++ \
  -mcpu=cortex-m4 -mthumb \
  -ffreestanding -nostdlib \
  -ffunction-sections -fdata-sections \
  -fno-exceptions -fno-rtti \
  -std=c++17 -O1 -g \
  -T stm32l476/stm32l476.ld \
  stm32l476/startup.c your_firmware.cpp \
  -Wl,--gc-sections \
  -o firmware.elf
```

### Clock / peripheral notes

The sim models a **subset** of the STM32L476 peripheral set. Firmware should run on the default MSI clock (~4 MHz after reset) — do **not** configure PLL or HSE, as the RCC peripheral model does not simulate clock-tree reconfiguration. Attempting to wait on an RCC flag (e.g. `RCC_CR_HSIRDY`) will busy-spin until the step limit is hit.

Modeled peripherals (v1): `rcc`, `gpioa`–`gpioh`, `systick`, `uart1`, `uart2`, `spi1`, `i2c1`.

Any memory access outside the modeled flash/RAM/peripheral range will produce a bus fault — `labwired_run` will include the faulting address in its `diagnosis` output.

### Minimal blink example

```c
// firmware.c — blink PA5 by toggling GPIOA ODR bit 5
// Compile with startup.c + stm32l476.ld (see above)

#include <stdint.h>

#define GPIOA_BASE  0x48000000U
#define GPIOA_MODER (*(volatile uint32_t *)(GPIOA_BASE + 0x00))
#define GPIOA_ODR   (*(volatile uint32_t *)(GPIOA_BASE + 0x14))

static void delay(volatile uint32_t n) { while (n--) {} }

int main(void) {
    // Set PA5 to output (MODER bits 11:10 = 01)
    GPIOA_MODER = (GPIOA_MODER & ~(3U << 10)) | (1U << 10);
    for (;;) {
        GPIOA_ODR ^= (1U << 5);
        delay(10000);
    }
}
```

### Encode the ELF for `labwired_run`

```bash
base64 -w 0 firmware.elf
```

Pass the resulting string as `elf_base64` to `labwired_run`.
