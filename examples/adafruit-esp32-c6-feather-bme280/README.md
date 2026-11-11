# Adafruit ESP32-C6 Feather + BME280

This smoke covers Adafruit PID 5933, an ESP32-family board (demand rank **#1**;
ESP32 is used by 38% of respondents in the JetBrains State of Embedded survey).
It adds board wiring for an existing C6 model; it does not add a new chip core.

Connect an Adafruit BME280 STEMMA QT breakout to the Feather's STEMMA QT port.
The firmware raises GPIO20 to power the connector, reads the BME280 chip ID
and temperature over I²C0 (SDA GPIO19, SCL GPIO18), then blinks the red GPIO15
LED. The smoke checks the sensor result and GPIO edges.

## Run

```sh
examples/adafruit-esp32-c6-feather-bme280/firmware/build-firmware.sh
scripts/onboarding_smoke.sh \
  --target-id adafruit-esp32-c6-feather-bme280 \
  --firmware-build-script examples/adafruit-esp32-c6-feather-bme280/firmware/build-firmware.sh \
  --script examples/adafruit-esp32-c6-feather-bme280/bme280-smoke.yaml \
  --system configs/systems/adafruit-esp32-c6-feather-bme280.yaml \
  --out-dir examples/adafruit-esp32-c6-feather-bme280/evidence
```

The profile covers the C6 GPIO and I²C paths, external BME280 and red user
LED. It omits the on-board fuel gauge and environmental sensor, charging and
power circuitry, NeoPixel, radio, low-power core, and electrical power gating.
The BME280 is directly attached in the model, so the smoke does not validate
that GPIO20 electrically switches connector power. See [validation](VALIDATION.md)
for the model boundary and evidence.

For a physical example, connect the BME280 breakout to STEMMA QT and upload the
same sketch with PlatformIO:

```sh
pio run -d examples/adafruit-esp32-c6-feather-bme280/firmware \
  -e feather_c6 -t upload
```

The expected serial line starts with
`ADAFRUIT_ESP32C6_FEATHER_BME280_OK ID=0x60 TEMP_RAW=0x082DC0`.
The board's USB-C connection uses the C6 USB Serial/JTAG controller; it does
not provide native USB HID/storage.
