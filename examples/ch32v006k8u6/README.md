# CH32V006K8U6 (QFN32) GPIO blink and USART1 hello

The CH32V006K8U6 is a WCH QingKe V2C (RV32EmC) microcontroller: 48 MHz
maximum, 62 KB flash, 8 KB SRAM and 31 GPIOs in QFN32 (WCH CH32V006DS0 V2.0).
It is the larger successor of the CH32V003. Its RCC, GPIO and USART registers
use the CH32V003 layout (WCH `ch32v00X.h`), with one more port, GPIOB, so the
descriptor runs the `wch` RCC and GPIO profiles on the RV32E core.

## Copy-paste bare-metal C example

```c
#include <stdint.h>

#define RCC_APB2PCENR (*(volatile uint32_t *)0x40021018u)
#define GPIOB_CFGLR   (*(volatile uint32_t *)0x40010C00u)
#define GPIOB_BSHR    (*(volatile uint32_t *)0x40010C10u)
#define GPIOB_BCR     (*(volatile uint32_t *)0x40010C14u)

static void delay(void) {
    for (volatile uint32_t i = 0; i < 500; ++i) {
        __asm__ volatile("nop");
    }
}

void main(void) {
    RCC_APB2PCENR |= 1u << 3; /* enable GPIOB */
    GPIOB_CFGLR = (GPIOB_CFGLR & ~0xFu) | 0x2u; /* PB0: push-pull output, 2 MHz */

    for (;;) {
        GPIOB_BSHR = 1u; /* set PB0 */
        delay();
        GPIOB_BCR = 1u;  /* reset PB0 */
        delay();
    }
}
```

Build both fixtures with a RISC-V GCC that supports `-march=rv32ec_zmmul
-mabi=ilp32e` (GCC 13 or later):

```sh
./examples/ch32v006k8u6/build.sh
```

The committed `tests/fixtures/ch32v006k8u6-blink.elf` and
`tests/fixtures/ch32v006k8u6-uart-hello.elf` were built with
`riscv32-esp-elf-gcc` 14.2.0 (crosstool-NG `esp-14.2.0_20260121`). Their
sha256 values are
`62481d720c9e485adc7bf8a41149b651a6f0b03713a6a50c9f6b94a67c2c40f6` and
`bd9abd4af37c31cf5b437b21297d2020ebad71073b7f88d56d418a022c37924a`. The
smokes check behaviour, not the hash.

Run the blink against the local model:

```sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v006k8u6/io-smoke.yaml \
  --output-dir out/boards/ch32v006k8u6/io-smoke --no-uart-stdout
```

The smoke requires at least two PB0 output edges. On a physical part, wire
PB0 (QFN32 pin 10) through a current-limiting resistor to an LED.

## USART1 hello

`uart_hello.c` maps USART1 TX to its default pin, PD5, at 115200 baud from the
24 MHz HSI. Before it prints, it multiplies two volatile values with `mul`
(Zmmul) and compares the product with the compile-time result.

```sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v006k8u6/uart-smoke.yaml \
  --output-dir out/boards/ch32v006k8u6/uart-smoke --no-uart-stdout
```

The smoke asserts `Hello from CH32V006` on the UART stream.

## Scope

These smokes exercise the local simulator's RV32E core with compressed
instructions and MUL, the WCH RCC, GPIO ports A to D and polled USART1 TX.
The model has no PFIC, SysTick, timers, ADC, OPA, I2C, SPI, DMA or USART2.
The simulator does not trap on DIV/REM, which QingKe V2C does not implement.
No CH32V006 has been compared with the model over SDI, and CH32V006 is not a
hosted LabWired verifier target. This is local model evidence, not a hosted
`model_verified` result or hardware confirmation.
