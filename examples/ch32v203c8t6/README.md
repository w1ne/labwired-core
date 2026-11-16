# CH32V203C8T6 (LQFP48) GPIO blink and USART1 hello

The CH32V203C8T6 is a WCH QingKe V4B (RV32IMAC) microcontroller: 144 MHz
maximum, 64 KB zero-wait flash, 20 KB SRAM and 37 GPIOs in LQFP48
(WCH CH32V203DS0 V3.0, Table 2-1). Its RCC, GPIO and USART registers use the
STM32F1 layout (WCH CH32FV2x_V3xRM), so the descriptor runs the `stm32f1`
RCC and GPIO profiles on the standard RV32 core.

## Copy-paste bare-metal C example

```c
#include <stdint.h>

#define RCC_APB2PCENR (*(volatile uint32_t *)0x40021018u)
#define GPIOC_CFGHR   (*(volatile uint32_t *)0x40011004u)
#define GPIOC_BSHR    (*(volatile uint32_t *)0x40011010u)
#define GPIOC_BCR     (*(volatile uint32_t *)0x40011014u)

#define LED_BIT 13u

static void delay(void) {
    for (volatile uint32_t i = 0; i < 500; ++i) {
        __asm__ volatile("nop");
    }
}

void main(void) {
    RCC_APB2PCENR |= 1u << 4; /* enable GPIOC */
    /* PC13 is in CFGHR (pins 8..15): push-pull output, 2 MHz. */
    GPIOC_CFGHR = (GPIOC_CFGHR & ~(0xFu << ((LED_BIT - 8u) * 4u)))
                | (0x2u << ((LED_BIT - 8u) * 4u));

    for (;;) {
        GPIOC_BSHR = 1u << LED_BIT; /* set PC13 */
        delay();
        GPIOC_BCR = 1u << LED_BIT;  /* reset PC13 */
        delay();
    }
}
```

Build both fixtures with any RISC-V GCC that supports `-march=rv32imac`:

```sh
./examples/ch32v203c8t6/build.sh
```

The committed `tests/fixtures/ch32v203c8t6-blink.elf` and
`tests/fixtures/ch32v203c8t6-uart-hello.elf` were built with
`riscv32-esp-elf-gcc` 14.2.0 (crosstool-NG `esp-14.2.0_20260121`). Their
sha256 values are
`07be2450677fc694805bddefa16050ec5a66f7c4ee75fb55c7c5a92e2c50c584` and
`5d0a1fcec9b6423ba0d51746c1f720e5a878bc416c402c78428cbcd4da63c45f`. The
smokes check behaviour, not the hash.

Run the blink against the local model:

```sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v203c8t6/io-smoke.yaml \
  --output-dir out/boards/ch32v203c8t6/io-smoke --no-uart-stdout
```

The smoke requires at least two PC13 output edges. On a physical part, wire
PC13 (pin 2) through a current-limiting resistor to an LED.

## USART1 hello

`uart_hello.c` maps USART1 TX to its default pin, PA9, at 115200 baud from the
8 MHz HSI reset clock. Before it prints, it computes a value with `mul`,
`divu` and `amoadd.w` and compares it with the compile-time result, so the
string also shows that the M and A extensions ran correctly.

```sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v203c8t6/uart-smoke.yaml \
  --output-dir out/boards/ch32v203c8t6/uart-smoke --no-uart-stdout
```

The smoke asserts `Hello from CH32V203` on the UART stream.

## Scope

These smokes exercise the local simulator's RV32IMAC core, the F1-layout RCC,
GPIO ports A to D and polled USART1/USART2 TX. The model has no PFIC,
SysTick, timers, ADC, I2C, SPI, CAN or USB. The RCC decodes PLLMUL with the
STM32F1 table, so a PLLMUL setting of x18 reads as x16 in the model. No
CH32V203 has been compared with the model over SDI, and CH32V203 is not a
hosted LabWired verifier target. This is local model evidence, not a hosted
`model_verified` result or hardware confirmation.
