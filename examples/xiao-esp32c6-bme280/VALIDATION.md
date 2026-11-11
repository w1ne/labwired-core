# Validation record

## Demand and scope

ESP32 is demand rank **#1** in the 2025 embedded MCU survey (38%). This is a
board-level extension for the Seeed XIAO ESP32-C6; ESP32-C6 silicon and the
BME280 model already existed. It is not a new MCU core. The smoke uses the
ESP32-C6FH4's 4 MiB profile, native USB Serial/JTAG, I²C0 on D4/D5, and the
on-board active-low GPIO15 indicator.

## Reproduce

```sh
examples/xiao-esp32c6-bme280/firmware/build-firmware.sh
scripts/onboarding_smoke.sh \
  --target-id xiao-esp32c6-bme280 \
  --firmware-build-script examples/xiao-esp32c6-bme280/firmware/build-firmware.sh \
  --script examples/xiao-esp32c6-bme280/bme280-smoke.yaml \
  --system configs/systems/xiao-esp32c6-bme280.yaml \
  --out-dir examples/xiao-esp32c6-bme280/evidence
```

Committed build and smoke logs, the structured verdict, and metrics are under
[`evidence/`](evidence/).

## Claim boundary

The local LabWired smoke asserts BME280 `CHIP_ID=0x60`, raw temperature
`0x082DC0`, native USB Serial/JTAG output, and at least four GPIO15 edges. The
Arduino ELF is fast-booted by the local engine; it is not a mask-ROM flash boot
or a physical-board test. The LP core, Wi-Fi, Bluetooth, Thread/Zigbee, RF
switch, battery charging, and USB enumeration are outside the tested scope.
The boot button binding is declared but not exercised. The current hosted
LabWired catalog has no C6 compile-and-verify path, so this evidence is from
the repository's local engine and does not claim a hosted `labwired_verify`
verdict.
