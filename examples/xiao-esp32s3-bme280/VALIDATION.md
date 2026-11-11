# Validation record

Demand rank: **ESP32 #1 (38%)**. This is a capability expansion for the
existing XIAO ESP32-S3 board profile: it adds a real BME280 I2C transaction on
D4/GPIO5 and D5/GPIO6 while retaining a check of the active-low USER_LED on
GPIO21. It is not a new chip-family model.

The local onboarding smoke builds Seeed's `seeed_xiao_esp32s3` Arduino target,
reads BME280 `CHIP_ID` (`0xD0`) and raw temperature (`0xFA`), and checks at least
six GPIO21 edges. The BME280 model returns a deterministic ID and temperature
vector; conversion timing and changing environmental values are outside this
smoke. The XIAO's 8 MiB flash is represented, while its physical 8 MiB OPI PSRAM
is excluded and firmware is built QSPI-only. Wi-Fi, BLE, USB electrical
enumeration, battery circuitry, and environmental accuracy are not tested.
The existing ESP32-S3 `system_stub` window is not read by this firmware. The
hosted verification catalog has no exact XIAO ESP32-S3 entry, so the evidence
below is from the repository's exact-board onboarding simulation; no separate
hosted-verification result is claimed.

## Reproduce

```sh
scripts/onboarding_smoke.sh \
  --target-id xiao-esp32s3-bme280 \
  --firmware-build-script examples/xiao-esp32s3-bme280/firmware/build-firmware.sh \
  --script examples/xiao-esp32s3-bme280/bme280-smoke.yaml \
  --system configs/systems/xiao-esp32s3-bme280.yaml \
  --out-dir examples/xiao-esp32s3-bme280/evidence
```

The committed `evidence/` directory contains the build logs and simulator
assertions. This evidence is for the digital twin and does not confirm a
physical board or its electrical I2C bus. See [vendor-status.md](vendor-status.md)
for the checked product availability and [README.md](README.md) for wiring and
a copy-paste Arduino example.
