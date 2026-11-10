# ESP32-C6-DevKitC-1-N8 + BME280 availability check

Checked live on 2026-11-10.

- Espressif's current ESP32-C6-DevKitC-1 v1.2 user guide lists the exact
  `ESP32-C6-DevKitC-1-N8` ordering variant, based on ESP32-C6-WROOM-1 with
  8 MB SPI flash. The guide has no end-of-life notice for this board.
- The live Mouser Europe listing for manufacturer part
  `ESP32-C6-DevKitC-1-N8` showed 1,120 in stock, dispatch immediately, at
  €7.89 per unit.
- The Adafruit BME280 STEMMA QT breakout (product 2652) listing showed
  in stock at $14.95.

Sources: [Espressif current user guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c6/esp32-c6-devkitc-1/user_guide.html),
[Mouser exact-part listing](https://www.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-C6-DevKitC-1-N8?qs=8Wlm6%252BaMh8TjnOR8RwmaBw%3D%3D),
and [Adafruit BME280 listing](https://www.adafruit.com/product/2652).

## Scope and evidence

The system uses the repository's existing `configs/chips/esp32c6.yaml` descriptor,
which models the C6 CPU and peripherals, and connects a real modeled BME280 to
I2C0 on GPIO6/GPIO7. The local `scripts/onboarding_smoke.sh` run validates the
BME280 ID and raw-temperature register sample and captures UART0 output. UART0
is captured directly; the board's USB-to-UART bridge is not simulated. The
GPIO matrix routing is exercised by the smoke, while IO_MUX remains a
declarative register store rather than an electrically enforced model.

This evidence is from the local simulator only. The hosted LabWired catalog
marks the C6 DevKitC entry `design_only`; there is no hosted `labwired_verify`
model-verification verdict for this sensor system and no physical-board test.
The profile does not claim Wi-Fi, Bluetooth, 802.15.4, USB behavior, buttons,
RGB LED behavior, or LP-core behavior.
