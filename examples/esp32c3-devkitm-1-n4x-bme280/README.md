# ESP32-C3-DevKitM-1-N4X + BME280

This runnable board example targets Espressif's currently listed
ESP32-C3-DevKitM-1-N4X. It builds an Arduino sketch, starts I²C0 on header
GPIO4/5, reads BME280 chip ID `0x60` and raw temperature, then emits a UART0
marker. The board guide identifies the USB-to-UART bridge and its RGB LED on
GPIO8; the smoke leaves the addressable LED out of the system descriptor.

## Run

```sh
LABWIRED_ESP32C3_FLASH=examples/esp32c3-devkitm-1-n4x-bme280/firmware/.pio/build/devkitm1_n4x/merged.bin \
  scripts/onboarding_smoke.sh \
    --target-id esp32c3-devkitm-1-n4x-bme280 \
    --firmware-build-script examples/esp32c3-devkitm-1-n4x-bme280/firmware/build-firmware.sh \
    --script examples/esp32c3-devkitm-1-n4x-bme280/bme280-smoke.yaml \
    --system configs/systems/esp32c3-devkitm-1-n4x-bme280.yaml \
    --out-dir examples/esp32c3-devkitm-1-n4x-bme280/evidence \
    --threshold-seconds 3600 \
    --rom-boot
```

The system connects an Adafruit BME280 STEMMA QT model at `0x77` to GPIO4/5.
The current N4X module has 4 MB flash. The evidence covers the modeled C3
mask-ROM boot, I²C controller, BME280 register responses, and UART0 output on
the local digital twin. Wi-Fi, Bluetooth LE, USB, the download/reset buttons,
and the WS2812 RGB LED are outside this smoke.

See [`vendor-status.md`](vendor-status.md) for live availability details,
[`EXTERNAL_COMPONENTS.md`](EXTERNAL_COMPONENTS.md) for wiring, and
[`VALIDATION.md`](VALIDATION.md) for the evidence boundary.
