# Adafruit ESP32 Feather V2

The board system is [`configs/systems/adafruit-feather-esp32-v2.yaml`](../../configs/systems/adafruit-feather-esp32-v2.yaml)
and uses the limited
[`esp32-feather-v2-smoke` chip profile](../../configs/chips/esp32-feather-v2-smoke.yaml).
Its current onboarding smoke exercises the discrete active-high red user LED
on GPIO13 through the classic ESP32 GPIO model. A second profile exercises the
STEMMA QT BME280 path, including its GPIO2-controlled rail and I2C0.

## Verified scope

- **Boot and GPIO smoke:** `examples/adafruit-feather-esp32-v2/io-smoke.yaml`
  runs the PlatformIO firmware and requires 16 GPIO13 edges.
- **BME280 smoke:** `examples/adafruit-feather-esp32-v2/bme280-smoke.yaml`
  accounts for Arduino initialization enabling GPIO2, switches it low to
  verify an address NACK, then raises it and reads the sensor ID and raw
  temperature on SDA=GPIO22 and SCL=GPIO20. Its local evidence is under
  `examples/adafruit-feather-esp32-v2/evidence/bme280/`.
- **Physical board map:** the user LED is GPIO13. GPIO2 is the STEMMA QT and
  NeoPixel power gate, not an LED. I2C uses SDA=GPIO22 and SCL=GPIO20.
- **Outside these profiles:** PSRAM data behavior, wireless radios, the
  USB-to-UART bridge, and NeoPixel signaling.
- The twin treats PSRAM as a probe-only stub, so startup logs a PSRAM ID error
  and continues without external memory. It also assumes an ideal 3.3 V rail;
  power-on and brown-out behavior are untested.

The hosted LabWired `labwired_verify` endpoint currently reports this classic
ESP32 run as `unsupported`: it observes the GPIO oracle, then faults at an
unmapped `0x3fc9b2b4` access. That address is outside the classic ESP32 map.
The committed local CLI run is the board-profile acceptance evidence; the
hosted endpoint's limitation is recorded in the example evidence.

Vendor lifecycle and stock evidence: [example evidence](../../examples/adafruit-feather-esp32-v2/evidence/vendor-status.md).
Pin assignments follow Adafruit's [official Feather V2 pinout guide](https://learn.adafruit.com/adafruit-esp32-feather-v2/pinouts).
