# Adafruit ESP32-S3 Feather 8 MB w.FL (#5885)

**Demand rank:** ESP32 is rank **#1** in the standing demand evidence (38% of
reported MCU use). This adds board coverage for that family; the survey does
not rank this exact Feather SKU. Adafruit's exact 8 MB w.FL variant was listed
for sale when checked on 2026-11-06. See
[`evidence/vendor-status.md`](evidence/vendor-status.md).

This narrow board profile boots a bare-metal ESP32-S3 image and reads the
onboard MAX17048 battery monitor at 0x36 through I2C0 on SDA GPIO3 / SCL GPIO4.
It checks the voltage and state-of-charge register snapshots and reports them
on UART0.

## Run the local simulator smoke

From the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id adafruit-feather-esp32-s3-8mb-wfl \
  --firmware-build-script examples/adafruit-feather-esp32-s3-8mb-wfl/ci/build-firmware.sh \
  --script examples/adafruit-feather-esp32-s3-8mb-wfl/onboarding-smoke.yaml \
  --system configs/systems/adafruit-feather-esp32-s3-8mb-wfl.yaml \
  --out-dir examples/adafruit-feather-esp32-s3-8mb-wfl/evidence
```

The test expects `VCELL=0xC080` (3.85 V) and `SOC=0x4880` (72.5%). The
MAX17048 descriptor provides deterministic stimulus-driven snapshots. It does
not calculate the part's ModelGauge state-of-charge estimate. Build logs, the
simulation trace, and generated metrics are committed in `evidence/`; this is
local simulator evidence, not a physical-board run.

The descriptor models 8 MiB flash and no PSRAM. It does not claim USB, Wi-Fi,
BLE, battery charging, ADC, NeoPixel, buttons, or general FeatherWing behavior.
The ESP32-S3 SYSTEM register window is still a stub; its I2C clock/reset gating
effects are not modelled. See [`KNOWN_LIMITATIONS.md`](KNOWN_LIMITATIONS.md).
