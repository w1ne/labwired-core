# Community hardware quickstarts — draft

This is a review draft for the community coverage page. It is intentionally not
listed in `mkdocs.yml` and has not been published. The current draft covers the
ESP32 family entry; additional demand-ranked entries need to be added and
reviewed before publication.

## ESP32 — rank 1, 38% of reported MCU use

### ESP32-DevKitC-32E / ESP32-WROOM-32E

The board profile boots Arduino firmware and exercises UART0, GPIO, I2C with a
BME280, RMT output, ADC1 channel 0, and LEDC PWM. The preceding ADC smoke
checks `analogRead(36)` on GPIO36 / ADC1_CH0 with a potentiometer fixture; a
1650 mV input returns 2047 at 12-bit resolution. This round adds stock
`ledcAttach` / `ledcWrite` output on GPIO23 / J3 pin 2 for an external
SG90-class servo fixture.

Espressif's [current DevKitC V4 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32/esp32-devkitc/user_guide.html)
lists the exact 32E board and maps J2 pin 3 / VP to GPIO36 / ADC1_CH0. The
[Mouser listing](https://www.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-DevKitC-32E?mgh=1&qs=GedFDFLaBXFpgD0kAZWDrQ%3D%3D)
reported immediate stock during the current check; DigiKey lists the SKU as
Active, though its stock view differed. Full lifecycle evidence is in
[`vendor-status.md`](../../examples/esp32-devkitc-32e/evidence/adc/vendor-status.md).

Copy-paste Arduino example for a potentiometer wired from 3V3 to GND with its
wiper connected to J2 pin 3 / GPIO36:

```cpp
#include <Arduino.h>

void setup() {
  Serial.begin(115200);
  analogReadResolution(12);
  const int raw = analogRead(36);  // ADC1_CH0 / VP
  Serial.printf("ADC1_CH0=%d\n", raw);
}

void loop() {}
```

Copy-paste Arduino example for a servo signal on J3 pin 2 / GPIO23. Power the
physical servo separately from 5 V and connect its ground to the board ground:

```cpp
#include <Arduino.h>

void setup() {
  constexpr uint8_t servo_pin = 23;
  if (!ledcAttach(servo_pin, 50, 14)) return;
  ledcWrite(servo_pin, 1187);  // about 1.45 ms per 49.92 Hz frame
}

void loop() {}
```

Run it against the committed board profile with:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-devkitc-32e-ledc-servo \
  --script examples/esp32-devkitc-32e/ledc-smoke.yaml \
  --system configs/systems/esp32-devkitc-32e-ledc-servo.yaml \
  --out-dir examples/esp32-devkitc-32e/evidence/ledc \
  --firmware-build-script scripts/build-esp32-devkitc-32e-ledc-smoke.sh
```

The live board status and in-stock check is in
[`vendor-status.md`](../../examples/esp32-devkitc-32e/evidence/ledc/vendor-status.md).
The smoke asserts the channel-0 duty shadow and GPIO23 edges. The SG90 twin is
not evidence of physical servo torque, current draw, or mechanical motion.

Run it against the committed board profile with:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-devkitc-32e-adc \
  --script examples/esp32-devkitc-32e/adc-smoke.yaml \
  --system configs/systems/esp32-devkitc-32e-adc.yaml \
  --out-dir examples/esp32-devkitc-32e/evidence/adc \
  --firmware-build-script scripts/build-esp32-devkitc-32e-adc-smoke.sh
```

The run log is [`run_smoke.log`](../../examples/esp32-devkitc-32e/evidence/adc/logs/run_smoke.log);
the complete board scope and other copy-paste examples are in the
[DevKitC-32E README](../../examples/esp32-devkitc-32e/README.md).

The deterministic ADC fixture models an injected 1650 mV level. It does not
model ADC noise, calibration, or the analog attenuation curve, and the
potentiometer is an external fixture rather than an onboard board component.

## Remaining demand-ranked entries

Before this page is ready for publication, add reviewed coverage and a
copy-paste example for RP2040/RP2350, the supported Arduino AVR path, STM32F103
and current Nucleo/Discovery boards, and the WCH CH32V003 community target. Each
entry must link to its exact board or part evidence and clearly identify any
legacy lifecycle status.
