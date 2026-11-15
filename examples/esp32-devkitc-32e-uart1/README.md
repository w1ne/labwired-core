# ESP32-DevKitC-32E UART1 TX smoke

**Demand rank:** ESP32 is rank 1 in the standing target list (38% of reported
MCU use). This adds peripheral depth to the existing DevKitC-32E model; it is
not a new MCU family.

This profile targets the Espressif ESP32-DevKitC-32E with an ESP32-WROOM-32E
module and 4 MB flash. It exposes UART0 console output, UART1 TX routed to
J3 pin 12 / GPIO16, and only the GPIO pad needed by this smoke. UART1 RX,
Wi-Fi/Bluetooth, SPI, I2C, ADC, and onboard LEDs are outside this profile.
GPIO16 is available on the WROOM module variant used on the 32E board.

## Copy-paste Arduino example

Connect the receiving device's RX to J3 pin 12 / GPIO16, connect grounds, and
use 3.3 V logic. This profile exercises TX only.

```cpp
#include <Arduino.h>

void setup() {
  Serial1.begin(115200, SERIAL_8N1, -1, 16);  // RX unused, TX=GPIO16
  Serial1.println("sensor request");
}

void loop() {}
```

The acceptance firmware sends repeated `U` bytes and prints a UART0 marker.
The smoke requires the marker and at least 32 transitions on GPIO16, confirming
the routed UART1 TX waveform. The hosted verifier independently reports 87
GPIO16 transitions. Neither check is a physical-board measurement.

The checked-in vendor availability and hosted verification notes are under
[`evidence/`](evidence/).

## Reproduce

From the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-devkitc-32e-uart1 \
  --script examples/esp32-devkitc-32e-uart1/uart1-smoke.yaml \
  --system configs/systems/esp32-devkitc-32e-uart1.yaml \
  --out-dir examples/esp32-devkitc-32e-uart1/evidence/run \
  --firmware-build-script scripts/build-esp32-devkitc-32e-uart1-smoke.sh
```
