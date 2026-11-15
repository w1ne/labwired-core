# CH32V005F6P6 (TSSOP20) GPIO blink and USART1 hello

The CH32V005F6P6 is a WCH QingKe V2C (RV32EmC) microcontroller: 48 MHz
maximum, 32 KB flash, 6 KB SRAM and 18 GPIOs in TSSOP20 (WCH CH32V006/005
DS0 V2.0). It is pin-compatible with the CH32V003F4P6 and has twice its
flash. Its RCC, GPIO and USART registers use the CH32V003 layout (WCH
`ch32v00X.h`), so the descriptor runs the `wch` RCC and GPIO profiles on the
RV32E core.

## Copy-paste bare-metal C example

```c
#include <stdint.h>

/* Register addresses and bits: WCH ch32v00X.h (openwch/ch32v002_004_005_006_007). */
#define RCC_APB2PCENR (*(volatile uint32_t *)0x40021018u)
#define GPIOC_CFGLR   (*(volatile uint32_t *)0x40011000u)
#define GPIOC_BSHR    (*(volatile uint32_t *)0x40011010u)
#define GPIOC_BCR     (*(volatile uint32_t *)0x40011014u)

static void delay(void) {
    for (volatile uint32_t i = 0; i < 500; ++i) {
        __asm__ volatile("nop");
    }
}

void main(void) {
    RCC_APB2PCENR |= 1u << 4; /* enable GPIOC */
    GPIOC_CFGLR = (GPIOC_CFGLR & ~0xFu) | 0x2u; /* PC0: push-pull output, 2 MHz */

    for (;;) {
        GPIOC_BSHR = 1u; /* set PC0 */
        delay();
        GPIOC_BCR = 1u;  /* reset PC0 */
        delay();
    }
}
```

Build both fixtures with a RISC-V GCC that supports `-march=rv32ec_zmmul
-mabi=ilp32e` (GCC 13 or later):

```sh
./examples/ch32v005f6p6/build.sh
```

The committed `tests/fixtures/ch32v005f6p6-blink.elf` and
`tests/fixtures/ch32v005f6p6-uart-hello.elf` were built with
`riscv32-esp-elf-gcc` 14.2.0 (crosstool-NG `esp-14.2.0_20260121`). Their
sha256 values are
`e734f818247aadf49bff283f9a9104a4db55960d582f327ff4747a95ffa58360` and
`694232fe516d9f7c594f8e600a409f39b3dda72c1ac50e25cc89ea0dc01cfa2d`. The
smokes check behaviour, not the hash.

Run the blink against the local model:

```sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v005f6p6/io-smoke.yaml \
  --output-dir out/boards/ch32v005f6p6/io-smoke --no-uart-stdout
```

The smoke requires at least two PC0 output edges. On a physical part, wire
PC0 (TSSOP20 pin 10) through a current-limiting resistor to an LED.

## USART1 hello

`uart_hello.c` maps USART1 TX to its default pin, PD5 (TSSOP20 pin 2), at 115200 baud from the
24 MHz HSI. Before it prints, it multiplies two volatile values with `mul`
(Zmmul) and compares the product with the compile-time result.

```sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v005f6p6/uart-smoke.yaml \
  --output-dir out/boards/ch32v005f6p6/uart-smoke --no-uart-stdout
```

The smoke asserts `Hello from CH32V005` on the UART stream.

## Scope

These smokes exercise the local simulator's RV32E core with compressed
instructions and MUL, the WCH RCC, GPIO ports A to D and polled USART1 TX. The die has GPIOB, but
no PB pin is bonded on TSSOP20.
The model has no PFIC, SysTick, timers, ADC, OPA, I2C, SPI, DMA or USART2.
The simulator does not trap on DIV/REM, which QingKe V2C does not implement.
No CH32V005 has been compared with the model over SDI, and CH32V005 is not a
hosted LabWired verifier target. This is local model evidence, not a hosted
`model_verified` result or hardware confirmation.
