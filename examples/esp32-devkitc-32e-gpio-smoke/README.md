# ESP32-DevKitC-32E GPIO smoke (CLI proof)

Onboarding profile for `configs/chips/esp32-devkitc-32e-gpio-smoke.yaml`.
This directory is the chip's CLI coverage cell: `ci/test.sh` runs the
committed `tests/fixtures/tier1/esp32.elf` image (the same fixture parent
`esp32` uses) against `configs/systems/esp32-devkitc-32e-gpio-smoke.yaml`
and asserts `TIER1 clock PASS` and `TIER1 done`.

The PlatformIO deep/light-sleep sketches that produced the
`docs/receipts/2026-11-09-esp32-deep-sleep/` UART logs are not committed
ELFs; they stay local receipts.
