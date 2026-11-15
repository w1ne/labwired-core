# WCH CH32V003EVT

**Demand position:** the measured MCU survey does not rank WCH. CH32V003 is the
standing brief's Tier C vendor bet: an inexpensive WCH QingKe RV32EC part with
active hobbyist and Arduino-core use. This is board coverage for the exact
`WCH-CH32V003EVT`, not another WCH silicon model.

The board uses a CH32V003F4P6. Its LED1/LED2 signals are on connector P4; they
are not fixed to a GPIO. This profile represents a jumper from LED1 (P4-1) to
PC0. LED1 is active low because its series resistor and anode connect to VCC.
USART1 TX (PD5) is exposed on the board header. The USB-C connector supplies
power only, so `debug_uart` is a LabWired capture route and does not represent
an on-board USB-UART bridge.

## Copy-paste bare-metal C example

This blink configures GPIOC and toggles PC0. Connect P4-1 (LED1) to PC0 with a
jumper lead on the evaluation board.

```c
#include <stdint.h>

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
    GPIOC_CFGLR = 0x2u;       /* PC0: push-pull output, 2 MHz */
    for (;;) {
        GPIOC_BSHR = 1u;      /* LED1 off */
        delay();
        GPIOC_BCR = 1u;       /* LED1 on */
        delay();
    }
}
```

The tested source is [`main.c`](../ch32v003f4p6/main.c). A second compiled
example writes `Hello from CH32V003` through polled USART1 TX on PD5; connect a
3.3 V USB-serial adapter to PD5 and GND to observe it on a physical board.

## Reproduce

The repository includes compiled firmware fixtures. To rebuild them, use a
RISC-V GCC that supports `-march=rv32e -mabi=ilp32e`:

```sh
./examples/ch32v003f4p6/build.sh
```

Run the board LED smoke and UART smoke from the repository root:

```sh
cargo run --locked -q -p labwired-cli -- test \
  --script examples/wch-ch32v003evt/io-smoke.yaml \
  --output-dir out/boards/wch-ch32v003evt/io-smoke --no-uart-stdout

cargo run --locked -q -p labwired-cli -- test \
  --script examples/wch-ch32v003evt/uart-smoke.yaml \
  --output-dir out/boards/wch-ch32v003evt/uart-smoke --no-uart-stdout
```

The committed evidence covers GPIOC/PC0 and polled USART1 TX only. The model
does not include WCH-Link/SDI debug, PFIC interrupts, ADC, I2C, SPI, timers, or
USB data. The board has no USB data function. These are local simulator smokes,
not hosted `model_verified` results or physical hardware confirmation.

See [`vendor-status.md`](vendor-status.md) for current lifecycle and stock
evidence.
