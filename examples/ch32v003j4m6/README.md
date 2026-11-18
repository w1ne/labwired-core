# CH32V003J4M6 (SOP-8) GPIO blink and USART1 hello

The CH32V003J4M6 is the SOP-8 package of the WCH CH32V003 (QingKe RV32EC,
48 MHz, 16 KB flash, 2 KB SRAM). It is the same die as the CH32V003F4P6 in
`examples/ch32v003f4p6`, but it bonds only 9 GPIO signals onto 6 I/O pads
(WCH CH32V003DS0 V1.8, Table 2-1):

| Pin | Signals |
|---|---|
| 1 | PD6, PA1 |
| 2 | VSS |
| 3 | PA2 |
| 4 | VDD |
| 5 | PC1 |
| 6 | PC2 |
| 7 | PC4 |
| 8 | PD1 (SWIO), PD4, PD5 |

PC0, which the F4P6 example blinks, has no pad on this package. This example
blinks PC4 (pin 7) instead.

## Copy-paste bare-metal C example

```c
#include <stdint.h>

#define RCC_APB2PCENR (*(volatile uint32_t *)0x40021018u)
#define GPIOC_CFGLR   (*(volatile uint32_t *)0x40011000u)
#define GPIOC_BSHR    (*(volatile uint32_t *)0x40011010u)
#define GPIOC_BCR     (*(volatile uint32_t *)0x40011014u)

#define LED_BIT 4u

static void delay(void) {
    for (volatile uint32_t i = 0; i < 500; ++i) {
        __asm__ volatile("nop");
    }
}

void main(void) {
    RCC_APB2PCENR |= 1u << 4; /* enable GPIOC */
    GPIOC_CFGLR = (GPIOC_CFGLR & ~(0xFu << (LED_BIT * 4u))) | (0x2u << (LED_BIT * 4u));

    for (;;) {
        GPIOC_BSHR = 1u << LED_BIT; /* set PC4 */
        delay();
        GPIOC_BCR = 1u << LED_BIT;  /* reset PC4 */
        delay();
    }
}
```

Build it with the F4P6 example's `start.S` and `link.x`:

```sh
./examples/ch32v003j4m6/build.sh
```

The committed `tests/fixtures/ch32v003j4m6-blink-pc4.elf` was built with
`riscv32-esp-elf-gcc` 14.2.0 (crosstool-NG `esp-14.2.0_20260121`). Its sha256
is `2bcdae579c10d73b081e6f2ce4ef0bdccd5bdfd13661e0182ee36ae6a007788e`. The
smoke checks behaviour, not the hash.

Run it against the local model:

```sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003j4m6/io-smoke.yaml \
  --output-dir out/boards/ch32v003j4m6/io-smoke --no-uart-stdout
```

The smoke requires at least two PC4 output edges. On a physical part, wire
pin 7 through a current-limiting resistor to an LED and ground.

## USART1 hello

USART1 TX keeps its default mapping, PD5, which the SOP-8 bonds to pin 8
together with PD1 (SWIO) and PD4. The F4P6 hello firmware therefore runs
unchanged:

```sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v003j4m6/uart-smoke.yaml \
  --output-dir out/boards/ch32v003j4m6/uart-smoke --no-uart-stdout
```

The smoke asserts `Hello from CH32V003` on the UART stream. Pin 8 is also the
single-wire debug pin (SWIO).

## Scope

These smokes exercise the local simulator's RV32EC core, RCC, GPIO and
polled USART1 TX. The model does not short the signals that share a pad. No
CH32V003 has been compared with the model over SDI, and CH32V003 is not a
hosted LabWired verifier target. This is local model evidence, not a hosted
`model_verified` result or hardware confirmation.
