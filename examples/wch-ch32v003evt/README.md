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

## I2C1 sensor read (stock openwch `Wire`)

I2C1 is the legacy F1-layout controller at `0x4000_5400`, clocked by
`APB1PCENR` bit 21, with `I2C1_EV` 30 and `I2C1_ER` 31 wired into the PFIC.
The default (non-remapped) pads are SDA = PC1 and SCL = PC2 — exactly the pads
the openwch core selects from `PIN_WIRE_SDA` / `PIN_WIRE_SCL`. Tie the BME280
breakout's SDO low for address `0x76`, and connect SDA/SCL to PC1/PC2 with
pull-ups.

```cpp
#include <Arduino.h>
#include <Wire.h>

void setup() {
  Serial.begin(115200);
  Wire.begin();

  Wire.beginTransmission(0x76);
  Wire.write(0xD0);                  // CHIP_ID register
  uint8_t err = Wire.endTransmission();
  uint8_t count = Wire.requestFrom(0x76, (uint8_t)1);
  uint8_t id = (count == 1) ? Wire.read() : 0;

  Serial.println((err == 0 && count == 1 && id == 0x60) ? "I2C_PASS" : "I2C_FAIL");
}
```

The tested sketch is
[`tests/fixtures/ch32v003f4p6-arduino-openwch-i2c/src/main.cpp`](../../tests/fixtures/ch32v003f4p6-arduino-openwch-i2c/src/main.cpp).
The board profile with the sensor is
[`configs/systems/wch-ch32v003evt-bme280.yaml`](../../configs/systems/wch-ch32v003evt-bme280.yaml).
The vendor test
[`crates/core/tests/vendor/ch32v003_compiled_firmware.rs`](../../crates/core/tests/vendor/ch32v003_compiled_firmware.rs)
runs it twice: address `0x76` returns `id=0x60` and the sketch prints
`CH32V003_I2C_PASS`; the negative control places the sensor at `0x77`, so the
address phase NACKs and the same sketch prints `CH32V003_I2C_FAIL`.

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

cargo run --locked -q -p labwired-cli -- test \
  --script examples/wch-ch32v003evt/i2c-smoke.yaml \
  --output-dir out/boards/wch-ch32v003evt/i2c-smoke --no-uart-stdout
```

The committed evidence covers GPIOC/PC0, polled USART1 TX and RX-interrupt
input, the STK SysTick interrupt, EXTI line 3, ADC1 (10-bit), TIM1/TIM2 PWM and
the I2C1 master read above. What is still not modelled: WCH-Link/SDI debug, the
SPI1 controller, USB data, the AFIO I2C1 partial/full remaps (PD0/PD1 and
PC6/PC5), and the TIM1 complementary outputs (PD0/PA2/PD1). ADC1 is exercised
end to end against the generic `analog-ntc-diode-system.yaml` decks; see
[`crates/core/tests/analog/cosim_ch32v003_analog.rs`](../../crates/core/tests/analog/cosim_ch32v003_analog.rs).
The board has no USB data function. These are local simulator smokes, not
hosted `model_verified` results or physical hardware confirmation.

See [`vendor-status.md`](vendor-status.md) for current lifecycle and stock
evidence.
