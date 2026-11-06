# Seeed Studio XIAO ESP32-C6 GPIO smoke

This narrow board profile runs a bare-metal ESP32-C6 image and toggles the
onboard yellow USER LED through GPIO15. The pin is active-low. The chip profile
loads the real ESP32-C6 GPIO model and only the memory needed to execute this
smoke.

The profile does not claim wireless, USB, UART, I2C, SPI, ADC, battery,
charging, RF-switch, or low-power-core behavior. The broad C6 catalog
descriptor has additional peripherals and authored placeholders; this board
profile omits those windows entirely.

The smoke loads a bare-metal ELF directly at its entry point in the simulator.
It exercises the GPIO model and does not claim a mask-ROM or Arduino framework
startup path.

Run from the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id seeed-xiao-esp32c6 \
  --script examples/seeed-xiao-esp32c6/onboarding-smoke.yaml \
  --system configs/systems/seeed-xiao-esp32c6.yaml \
  --firmware-build-script examples/seeed-xiao-esp32c6/ci/build-firmware.sh \
  --out-dir examples/seeed-xiao-esp32c6/evidence
```

The committed `evidence/` directory contains the firmware build log and the
local deterministic engine smoke output. This is not physical-board testing.
Vendor availability and lifecycle sources are recorded in
[`vendor-status.md`](vendor-status.md).

The separate [BME280 profile](../seeed-xiao-esp32c6-bme280/README.md) covers
the board's documented D4/D5 I2C0 path.
