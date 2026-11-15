# Seeed Studio XIAO ESP32-C6 onboard LED

## Demand position

ESP32 is rank 1 in the measured MCU list (38%). This is maintenance/cheap
board-depth work for that high-demand family, not a new-demand chip pick: the
ESP32-C6 silicon model already exists, so this adds the buyable XIAO board
profile and its onboard LED path rather than a new chip model.

The profile uses the exact ESP32-C6FH4 4 MB flash size. See the [live stock
receipt](vendor-status.md).

## Pin map

The XIAO exposes these digital header pins. The board profile binds the user
LED and BOOT button; the remaining pins are available as headers and use the
ESP32-C6 peripheral models listed below.

| XIAO pin | ESP32-C6 pin | Board use |
| --- | --- | --- |
| D0 | GPIO0 | ADC |
| D1 | GPIO1 | ADC |
| D2 | GPIO2 | ADC |
| D3 | GPIO21 | Digital I/O |
| D4 | GPIO22 | I2C SDA |
| D5 | GPIO23 | I2C SCL |
| D6 | GPIO16 | UART TX |
| D7 | GPIO17 | UART RX |
| D8 | GPIO19 | SPI SCK |
| D9 | GPIO20 | SPI MISO |
| D10 | GPIO18 | SPI MOSI |
| Onboard user LED | GPIO15 | Active-low output |
| BOOT button | GPIO9 | Active-low input; reset strap behavior is not modeled |

The C6 profile has modeled GPIO, UART, I2C, SPI, ADC, timer, and PWM blocks,
with the silicon limits recorded in [`KNOWN_LIMITATIONS.md`](KNOWN_LIMITATIONS.md).
This board smoke specifically exercises the GPIO pad output on GPIO15.

## Copy-paste Arduino example

Select **XIAO_ESP32C6** in Arduino IDE and upload:

```cpp
#include <Arduino.h>

constexpr uint8_t USER_LED = 15;

void setup() {
  pinMode(USER_LED, OUTPUT);
}

void loop() {
  digitalWrite(USER_LED, LOW);  // active-low LED on
  delay(500);
  digitalWrite(USER_LED, HIGH); // LED off
  delay(500);
}
```

Seeed's guide says the orange LED blinks with the stock Blink example and maps
the user light to GPIO15. The schematic wires the LED from 3V3 through a
resistor to GPIO15, confirming active-low behavior. The committed local smoke
captured 33 GPIO15 transitions and passed its 16-edge assertion. See
[`evidence/onboarding-summary.md`](evidence/onboarding-summary.md) and
[`evidence/logs/run_smoke.log`](evidence/logs/run_smoke.log). This is simulator
evidence, not a physical board test.

## Reproduce the local smoke

From the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id seeed-xiao-esp32c6 \
  --script examples/xiao-esp32c6-blinky/led-smoke.yaml \
  --system configs/systems/seeed-xiao-esp32c6.yaml \
  --firmware-build-script examples/xiao-esp32c6-blinky/ci/build-firmware.sh \
  --out-dir examples/xiao-esp32c6-blinky/evidence
```

The smoke boots the C6 high-performance RISC-V core and checks transitions on
the modeled GPIO15 pad. The GPIO mux selection is currently register-backed,
not electrically enforced, so the test proves the GPIO model's pad transitions
and the board's documented GPIO15-to-LED mapping, not IO_MUX enforcement.
