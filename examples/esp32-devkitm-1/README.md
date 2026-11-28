# Espressif ESP32-DevKitM-1

**Demand rank:** ESP32 is #1 in the measured MCU-use list at 38%. This adds a
system fixture for Espressif's breadboard-friendly ESP32-DevKitM-1. It uses
the existing [classic ESP32 descriptor](../../configs/chips/esp32.yaml) and
adds coverage for this board's UART0 and external GPIO23 indicator.

The exact board uses an ESP32-MINI-1 module with 4 MB flash and no PSRAM. The
system profile covers UART0 and an external indicator LED attached to header
GPIO23 through a 330 Ω series resistor. Espressif maps UART0 TX/RX to GPIO1/GPIO3
and connects those lines to the onboard USB-to-UART bridge. The simulator
captures UART0 output at the MCU model; the USB bridge and USB connection
themselves are not simulated. GPIO23 is not an onboard LED.

## Reproduce

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id esp32-devkitm-1 \
  --firmware-build-script examples/esp32-devkitm-1/build-firmware.sh \
  --script examples/esp32-devkitm-1/onboarding-smoke.yaml \
  --system configs/systems/esp32-devkitm-1.yaml \
  --out-dir examples/esp32-devkitm-1/evidence
```

The smoke requires the `ESP32_DEVKITM1_UART0_OK` UART0 marker and at least 16
GPIO23 edges. Connect a physical LED anode to GPIO23 through a 330 Ω resistor
and its cathode to GND to reproduce the external indicator wiring. The run log
and full simulator result are committed under `evidence/`. The hosted verifier
separately model-verifies the shared ESP32 GPIO23 path; its catalog does not
contain this exact DevKitM-1 profile and assumes an ideal 3.3 V rail.

## Model boundary

The system reuses the existing classic ESP32 model and memory map. This smoke
validates UART0 and GPIO23 for the serial path and external indicator. It does
not validate Wi-Fi, Bluetooth, the physical USB-to-UART bridge, USB,
reset/download buttons, regulator, power LED, or other peripherals.

## Analog inputs

`analog-ntc-diode-system.yaml` drives a generic Beta-parameter NTC divider and
a Shockley diode load line into `board.analog.gpio34_volts` and
`board.analog.gpio35_volts`. The classic ESP32 descriptor maps the eight ADC1
pads (GPIO36-GPIO39 and GPIO32-GPIO35) to SENS SAR1 channels 0-7, so a deck
names a header pad and never a channel. The map is ADC1-only on purpose: the
Wi-Fi driver owns ADC2, so firmware cannot use ADC2 while Wi-Fi is enabled.
ADC2 and every other SAR channel stay reachable through the explicit
`adc.sens_sar_adc.<channel>_volts` route. A pad outside the map, including the
ADC2 pad GPIO25, is refused at binding time.

## Evidence provenance

The committed receipts in `evidence/` were captured on 2026-11-10 with the
original narrow `esp32-devkitm-1-smoke` descriptor. Their logs remain unchanged
to preserve that provenance. The current system uses the canonical `esp32.yaml`
descriptor, which has the same CPU, flash, RAM, UART0, and GPIO map needed by
this fixture.

Live status and purchase evidence is recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## BME280 profile

The [DevKitM-1 + BME280 profile](../../configs/systems/esp32-devkitm-1-bme280.yaml)
adds an I2C sensor at `0x77` on GPIO21 (SDA) and GPIO22 (SCL), plus the existing
GPIO23 external indicator. This is maintenance/board-depth work for demand-rank
1 ESP32, not a new-demand chip pick. The [copy-paste Arduino sketch and smoke
instructions](#bme280-copy-paste-example) exercise the sensor's `CHIP_ID`
register; exact live buyability is recorded in
[`evidence/vendor-status-bme280.md`](evidence/vendor-status-bme280.md).

### BME280 copy-paste example

Wire the breakout's SDA/SCL to GPIO21/GPIO22, power it from 3V3, connect GND,
and attach an external LED to GPIO23 through a 330 Ω resistor. The BME280's
default address is `0x77`. Provide I2C pull-ups to 3V3 if the breakout does not
include them; the simulator checks digital transactions and does not model
pull-up resistance.

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t BME280_ADDRESS = 0x77;
constexpr uint8_t BME280_CHIP_ID_REGISTER = 0xD0;
constexpr uint8_t BME280_CHIP_ID = 0x60;
constexpr uint8_t INDICATOR_PIN = 23;

void setup() {
  Serial.begin(115200);
  pinMode(INDICATOR_PIN, OUTPUT);
  digitalWrite(INDICATOR_PIN, LOW);
  Wire.begin(21, 22, 100000);  // SDA, SCL, 100 kHz
  Wire.beginTransmission(BME280_ADDRESS);
  Wire.write(BME280_CHIP_ID_REGISTER);
  if (Wire.endTransmission(false) != 0 ||
      Wire.requestFrom(BME280_ADDRESS, static_cast<uint8_t>(1)) != 1) {
    Serial.println("DEVKITM1_BME280_I2C_ERROR");
    return;
  }
  const uint8_t id = Wire.read();
  if (id == BME280_CHIP_ID) {
    digitalWrite(INDICATOR_PIN, HIGH);
    Serial.println("DEVKITM1_BME280_OK ID=0x60");
  } else {
    Serial.printf("DEVKITM1_BME280_BAD_ID=0x%02X\n", id);
  }
}

void loop() { delay(100); }
```

Reproduce the simulator run from the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id esp32-devkitm-1-bme280 \
  --firmware-build-script examples/esp32-devkitm-1/build-bme280-firmware.sh \
  --script examples/esp32-devkitm-1/bme280-smoke.yaml \
  --system configs/systems/esp32-devkitm-1-bme280.yaml \
  --out-dir examples/esp32-devkitm-1/evidence/bme280
```

The committed fast-boot fixture executes a register-level read of BME280
`CHIP_ID` and raises GPIO23 only when the returned value is `0x60`. The profile
does not claim Wi-Fi/Bluetooth, USB bridge behavior, reset/download buttons,
power switching, or physical-board measurements. The hosted catalog has no
exact DevKitM-1 board entry, so this is a repository-local smoke result rather
than a hosted `model_verified` or hardware-confirmed result.
