# NUCLEO-H533RE

**Demand position:** `other STM32` is rank 4 in the MCU survey (13%); Nucleo /
Discovery kits rank 4 among kits (23%). H533RE-specific usage was not measured.
This is a new, current STM32H5 Nucleo board profile, not another variant of an
already modeled H533 board. ST's live lifecycle and availability evidence is in
[`vendor-status.md`](vendor-status.md).

## Board wiring and model scope

| Board signal | MCU pin | Modeled path |
|---|---|---|
| Green user LED LD2 | PA5 | active-high GPIO output |
| ST-LINK virtual COM TX/RX | PA2 / PA3 | USART2, alternate function 7 |

The profile exposes RCC, GPIOA, USART2, and the three pins above. Other H533
peripherals and pins remain unmodeled. The routes follow ST's
[MB1814 C-02 schematic](https://www.st.com/resource/en/schematic_pack/mb1814-h533re-c02-schematic.pdf)
and [STM32H533RE datasheet](https://www.st.com/resource/en/datasheet/stm32h533re.pdf).

## Copy-paste register example

This CMSIS-style C example configures the default 32 MHz reset clock, prints a
UART marker, and blinks LD2. It uses the modelled RCC, USART2, and GPIOA blocks.

```c
#include <stdint.h>

#define REG32(addr) (*(volatile uint32_t *)(addr))
#define RCC_AHB2ENR  REG32(0x44020C8Cu)
#define RCC_APB1LENR REG32(0x44020C9Cu)
#define GPIOA_MODER  REG32(0x42020000u)
#define GPIOA_BSRR   REG32(0x42020018u)
#define GPIOA_AFRL   REG32(0x42020020u)
#define USART2_CR1   REG32(0x40004400u)
#define USART2_BRR   REG32(0x4000440Cu)
#define USART2_ISR   REG32(0x4000441Cu)
#define USART2_TDR   REG32(0x40004428u)

static void putc_uart(char c) {
  while ((USART2_ISR & (1u << 7)) == 0) {}
  USART2_TDR = (uint8_t)c;
}

int main(void) {
  RCC_AHB2ENR |= 1u;           // GPIOA clock
  RCC_APB1LENR |= 1u << 17;    // USART2 clock
  GPIOA_MODER = (GPIOA_MODER & ~((3u << 4) | (3u << 6) | (3u << 10)))
              | (2u << 4) | (2u << 6) | (1u << 10);
  GPIOA_AFRL = (GPIOA_AFRL & ~0x0000FF00u) | 0x00007700u; // PA2/PA3 AF7
  USART2_BRR = 278u;            // 32 MHz / 115200
  USART2_CR1 = (1u << 0) | (1u << 3); // USART enable, transmitter enable

  const char marker[] = "NUCLEO_H533RE_OK\n";
  for (unsigned i = 0; marker[i]; ++i) putc_uart(marker[i]);
  for (;;) {
    GPIOA_BSRR = 1u << 5;       // LD2 on
    for (volatile unsigned i = 0; i < 100000; ++i) __asm volatile("nop");
    GPIOA_BSRR = 1u << 21;      // LD2 off
    for (volatile unsigned i = 0; i < 100000; ++i) __asm volatile("nop");
  }
}
```

## Smoke evidence

The smoke asserts the USART2 marker and at least four PA5 transitions. Its
committed run logs and structured result are in [`evidence/run/`](evidence/run/).

```sh
rustup target add thumbv8m.main-none-eabi
./examples/nucleo-h533re/ci/build.sh
ONBOARDING_EVIDENCE_DIR=examples/nucleo-h533re/evidence/run \
  ./examples/nucleo-h533re/ci/test.sh
```

This is simulator evidence only; see [`VALIDATION.md`](VALIDATION.md) for the
scope and model limitations.
