# ESP32-C6-DevKitC-1-N8 + BME280

ESP32 family depth pick (**rank 1, 38% survey usage**). This system reuses the
existing ESP32-C6 chip model at its exact 8 MB flash size and connects an
Adafruit BME280 breakout. It tests I2C0 on GPIO6/GPIO7 by reading ID `0x60`
and raw-temperature sample `0x082DC0`, with the results captured on UART0.

The system descriptor is
[`configs/systems/esp32c6-devkitc-1-n8-bme280.yaml`](../../configs/systems/esp32c6-devkitc-1-n8-bme280.yaml).
The smoke firmware is in
[`crates/firmware-esp32c6-devkitc-bme280-smoke/`](../../crates/firmware-esp32c6-devkitc-bme280-smoke/).
Committed local simulator evidence is in
[`examples/esp32c6-devkitc-1-n8-bme280/evidence/bme280/`](../../examples/esp32c6-devkitc-1-n8-bme280/evidence/bme280/).

The profile claims UART0, the GPIO matrix, I2C0, required PCR clock gates,
and the BME280. It does not claim Wi-Fi, Bluetooth, 802.15.4, USB behavior,
buttons, RGB LED behavior, LP-core behavior, or physical-board validation.
UART0 is captured directly; the board's USB-to-UART bridge is not simulated.
The GPIO matrix route is exercised. IO_MUX remains a declarative register
store rather than an electrically enforced model.

The hosted catalog lists the C6 DevKitC as `design_only`, so the evidence is
the local CLI smoke rather than a hosted `labwired_verify` verdict. See the
example's [live availability receipt](../../examples/esp32c6-devkitc-1-n8-bme280/evidence/bme280/vendor-status.md)
and [wiring and copy-paste Arduino example](../../examples/esp32c6-devkitc-1-n8-bme280/README.md).
