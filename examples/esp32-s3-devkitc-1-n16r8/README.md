# ESP32-S3-DevKitC-1 v1.1 N16R8 ADC input

This narrow board profile boots register-level ESP32-S3 firmware and reads a
centered potentiometer on GPIO4 / ADC1_CH3 (J1 pin 4). The system model drives
1650 mV and the firmware requires ADC code 2047. The committed repository smoke
is deterministic model evidence; it does not measure a physical board or ADC
accuracy.

Connect a 10 kΩ potentiometer between 3V3 and GND, with its wiper on J1 pin 4.
Copy-paste Arduino example for a 12-bit reading:

```cpp
#include <Arduino.h>

void setup() {
  Serial.begin(115200);
  analogReadResolution(12);
  Serial.printf("GPIO4_ADC1_CH3=%d\n", analogRead(4));
}

void loop() {}
```

The local register-level firmware and board profile are in
[`firmware/adc_smoke.c`](firmware/adc_smoke.c),
[`configs/chips/esp32s3-devkitc-1-n16r8-adc.yaml`](../../../configs/chips/esp32s3-devkitc-1-n16r8-adc.yaml),
and [`configs/systems/esp32-s3-devkitc-1-n16r8-adc.yaml`](../../../configs/systems/esp32-s3-devkitc-1-n16r8-adc.yaml).
Run the smoke from the repository root:

```sh
examples/esp32-s3-devkitc-1-n16r8/ci/build-adc-firmware.sh
RUSTC_WRAPPER= cargo run -q -p labwired-cli -- test \
  --script examples/esp32-s3-devkitc-1-n16r8/adc-smoke.yaml \
  --output-dir examples/esp32-s3-devkitc-1-n16r8/evidence/adc/simulation-output \
  --no-uart-stdout
```

The passing local logs and structured result are under
[`evidence/adc/`](evidence/adc/). Espressif's [live DevKits catalog](https://www.espressif.com/products/devkits)
lists the N16R8 SKU; the [current v1.1 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32s3/esp32-s3-devkitc-1/user_guide_v1.1.html)
maps GPIO4 / J1 pin 4 to ADC1_CH3 and marks older N8/N8R2 SKUs EOL.
