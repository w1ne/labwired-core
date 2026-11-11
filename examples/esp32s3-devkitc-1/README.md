# ESP32-S3-DevKitC-1 v1.1

These profiles target the Espressif ESP32-S3-DevKitC-1 v1.1 with the
ESP32-S3-WROOM-1-N16R8 module. The base profile covers the UART0 console path;
the RGB profile adds the onboard one-pixel WS2812 on GPIO38.

## Pin map

| Board connection | MCU signal | LabWired connection |
| --- | --- | --- |
| J3 pin 2, TX | GPIO43 / U0TXD | UART0 → `host_console` |
| J3 pin 3, RX | GPIO44 / U0RXD | UART0 ← `host_console` |
| J3 pin 10, RGB LED | GPIO38 | RMT0 → GPIO matrix → WS2812 decoder |

The official [ESP32-S3-DevKitC-1 v1.1 guide](https://documentation.espressif.com/esp-dev-kits/en/latest/esp32s3/esp32-s3-devkitc-1/user_guide_v1.1.html)
lists the remaining header pin functions and the board schematic.

## UART smoke

Run `io-smoke.yaml` to boot the committed ESP32-S3 Tier-1 firmware and assert
its UART0 transcript. The USB-to-UART bridge is abstracted by the `host_console`
endpoint; the bridge IC itself is not a separate model.

## RGB LED smoke

Run `rgb-smoke.yaml` with `rgb-smoke/firmware/src/main.cpp` to route RMT0 to
GPIO38 and send one red WS2812 pixel. `setup()` is a copy-paste Arduino sketch
that uses the S3 RMT registers directly. The committed flash image contains
that normal Arduino app. The local CLI fixture uses the same ELF with its entry
point set to `labwired_entry()`, a small harness that skips Arduino/IDF startup
and executes the same RMT routine plus a raw UART marker. This keeps the local
smoke focused on the board's modeled RMT-to-LED path. The separate hosted
`labwired_verify` run compiles and verifies the Arduino entry point.

The smoke asserts the completion marker; `rgb-smoke/verify-rgb-evidence.py`
checks the captured device artifact for one decoded red pixel.

```bash
scripts/onboarding_smoke.sh \
  --target-id esp32s3-devkitc-1-v11-rgb \
  --script examples/esp32s3-devkitc-1/rgb-smoke.yaml \
  --system configs/systems/esp32s3-devkitc-1-v11-rgb.yaml \
  --out-dir examples/esp32s3-devkitc-1/evidence/rgb \
  --firmware-build-script examples/esp32s3-devkitc-1/rgb-smoke/build-firmware.sh
python3 examples/esp32s3-devkitc-1/rgb-smoke/verify-rgb-evidence.py \
  examples/esp32s3-devkitc-1/evidence/rgb/simulation-output/result.json
python3 examples/esp32s3-devkitc-1/rgb-smoke/verify-rgb-evidence.py \
  examples/esp32s3-devkitc-1/evidence/rgb/hosted-verify.json
```

## Limits

The profiles do not cover PSRAM, USB-OTG, Wi-Fi/Bluetooth, or the BOOT and
RESET buttons. The RGB smoke does not model LED current draw or board power
behavior. Neither run is a physical-board test.

Live vendor status and the selected module variant are recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md). UART smoke output is
committed under [`evidence/run/`](evidence/run/); the RGB run is under
[`evidence/rgb/`](evidence/rgb/).
