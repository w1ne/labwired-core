# Arduino UNO R4 WiFi

This focused board profile exercises the RA4M1's SCI2 UART on D0/D1 and the
on-board D13 LED at P102. The smoke prints `UNO_R4_WIFI_OK` and toggles the
LED four times. It uses the repo's RA4M1 peripheral models, with a board-specific
pin map for SKU ABX00087.

Run from the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id arduino-uno-r4-wifi \
  --crate firmware-uno-r4-wifi-smoke \
  --target thumbv7em-none-eabi \
  --script examples/arduino-uno-r4-wifi/io-smoke.yaml \
  --system configs/systems/arduino-uno-r4-wifi.yaml \
  --out-dir examples/arduino-uno-r4-wifi/evidence
```

The committed smoke log and UART/GPIO simulator output are under `evidence/`.
The profile is intentionally limited to RA4M1 SCI2, P102 GPIO, its pin function
select block, and reset-clock readiness. The UNO R4 WiFi's ESP32-S3
coprocessor, Wi-Fi/Bluetooth, 12x8 LED matrix, Qwiic connector, USBFS/HID, and
other MCU peripherals are not described as working here. See
[`VALIDATION.md`](VALIDATION.md) for the model boundary and sources.
