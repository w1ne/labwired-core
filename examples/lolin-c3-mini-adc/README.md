# LOLIN C3 Mini V2.1.0 ADC quickstart

This smoke exercises the ESP32-C3 ADC on a buyable WEMOS board. ESP32 ranks
**#1** in the JetBrains State of Embedded 2025 survey (38%); this adds ADC
peripheral depth to the already-covered LOLIN C3 Mini, not another board SKU.

## Connect a potentiometer

Use a 10 kΩ linear potentiometer with its outer terminals on **3V3** and **GND**,
and its wiper on **GPIO0 / ADC1_CH0**. Keep the wiper at or below 3.3 V. At the
default 50% position the simulator supplies 1.65 V and firmware checks for a
12-bit sample in the 1900–2200 range.

The example's copy-paste firmware is in [`firmware/src/main.cpp`](firmware/src/main.cpp):

```cpp
#include <Arduino.h>

void setup() {
  Serial.begin(115200);
  analogReadResolution(12);
  analogSetPinAttenuation(0, ADC_11db);
  delay(100);
  const int raw = analogRead(0);  // GPIO0 / ADC1_CH0
  if (raw >= 1900 && raw <= 2200) {
    Serial.printf("LOLIN_C3_ADC_OK GPIO0=%d\n", raw);
  } else {
    Serial.printf("LOLIN_C3_ADC_FAIL GPIO0=%d\n", raw);
  }
}

void loop() { delay(1000); }
```

The system descriptor injects the potentiometer voltage into ADC channel 0. It
does not solve analog wires or enforce the GPIO0 electrical connection. The
profile omits Wi-Fi, Bluetooth, USB behavior, and the onboard WS2812B on GPIO7.
This is simulator evidence, not a physical-board test.

See [`VALIDATION.md`](VALIDATION.md) for the reproducible smoke and its committed
evidence.
