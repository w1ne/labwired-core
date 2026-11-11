# ESP32-C6-DevKitC-1-N8 + BME280

This runnable board example targets Espressif's ESP32-C6-DevKitC-1-N8. It
builds an Arduino sketch, starts `Wire` on GPIO6/7, reads the BME280 chip ID
and raw temperature, and emits a UART0 marker.

## Run

```sh
examples/esp32c6-devkitc-1-n8-bme280/firmware/build-firmware.sh
cargo run -q -p labwired-cli -- test \
  --script examples/esp32c6-devkitc-1-n8-bme280/bme280-smoke.yaml \
  --output-dir out/esp32c6-devkitc-1-n8-bme280 \
  --no-uart-stdout
```

The sample config connects an Adafruit BME280 STEMMA QT model at `0x77` to
I²C0, with SDA on GPIO6 and SCL on GPIO7. The board's USB-to-UART bridge is
represented by the UART0 capture endpoint. The board's Wi-Fi, Bluetooth,
802.15.4, native USB, and WS2812 RGB LED are outside this smoke's modeled
scope; the WS2812 is not described as an ordinary LED.

See [`vendor-status.md`](vendor-status.md) for the live availability check and
[`VALIDATION.md`](VALIDATION.md) for the evidence boundary.
