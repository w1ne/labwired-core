# STM32F103C8T6 Blue Pill

**Demand position:** STM32F103 ranks #3 in the measured MCU list (17%). This
adds a board-level profile for the popular Blue Pill using the existing
STM32F103C8 model; it does not add a new chip family.

The smoke boots a bare-metal Cortex-M3 image, sends a marker through USART1,
and toggles the board's active-low PC13 user LED. The profile covers only
those board connections. It does not model the USB device, USB bootloader,
onboard debugger, or every Blue Pill clone's board differences.

## Pin map

| Board signal | MCU pin | Notes |
| --- | --- | --- |
| User LED | PC13 | Active low on the reference schematic |
| USART1 TX | PA9 | Header pin; connect to a 3.3 V adapter's RX |
| USART1 RX | PA10 | Header pin; connect to adapter TX if needed |
| SWDIO / SWCLK | PA13 / PA14 | External SWD programmer; not exercised here |

The board has no USB-UART bridge. For a physical serial view, connect a 3.3 V
USB-serial adapter to PA9 and GND. The smoke's captured UART is a simulator
console route, not a simulated USB bridge.

## Copy-paste Arduino example

Select the STM32F103C8T6 Blue Pill board in STM32duino and use USART1 on PA9:

```cpp
#include <Arduino.h>

void setup() {
  pinMode(PC13, OUTPUT);
  Serial1.begin(115200);  // USART1 TX=PA9, RX=PA10
  Serial1.println("BLUEPILL_F103C8_OK");
}

void loop() {
  digitalWrite(PC13, LOW);   // active-low user LED on
  delay(500);
  digitalWrite(PC13, HIGH);  // user LED off
  delay(500);
}
```

## Reproduce the committed local simulator smoke

```sh
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id stm32f103-blue-pill \
  --crate firmware-stm32f103-bluepill-smoke \
  --target thumbv7m-none-eabi \
  --script examples/stm32f103-blue-pill/io-smoke.yaml \
  --system configs/systems/stm32f103-blue-pill.yaml \
  --out-dir examples/stm32f103-blue-pill/evidence
```

The smoke asserts the USART1 marker, at least four PC13 transitions, and a
bounded stop at the configured step limit. Evidence and the onboarding metric
are committed under `evidence/`. The board is not hardware-confirmed and is
not a hosted LabWired `model_verified` target.

See [`vendor-status.md`](vendor-status.md) for the live lifecycle and board
availability check.
