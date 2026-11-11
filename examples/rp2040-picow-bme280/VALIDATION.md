# Pico W + BME280 validation

This smoke builds RP2040 firmware, reads BME280 register `0xD0` (expected ID
`0x60`) and raw temperature registers (`0x082DC0` in the deterministic model),
then reports the result through UART0. I2C0 is configured on GP4/GP5 and the
sensor is attached to the modeled I2C controller at address `0x77`.

Run from the repository root:

```bash
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id rp2040-picow-bme280 \
  --crate firmware-rp2040-picow-bme280-smoke \
  --target thumbv6m-none-eabi \
  --script examples/rp2040-picow-bme280/bme280-smoke.yaml \
  --system configs/systems/rp2040-picow-bme280.yaml \
  --out-dir examples/rp2040-picow-bme280/evidence
```

Committed simulator output and stage logs are stored in `evidence/`.

The shared RP2040 I2C0-to-BME280 behavior was also checked with LabWired's
hosted `labwired_verify` oracle against its generic `rpi-pico` catalog target.
It returned `status: model_verified` for the UART marker. This verifies the
shared RP2040 sensor path, not a Pico W-specific radio, LED, or board model;
the twin reported non-blocking timing and ideal-rail approximations. The
verification details are in
[`evidence/hosted-model-verification.md`](evidence/hosted-model-verification.md).

The smoke does not exercise Wi-Fi, Bluetooth, CYW43439, the WL_GPIO0 onboard
LED, USB, or physical board pads. The BME280 is attached directly to the
modeled I2C controller; this does not validate electrical pull-ups or pad
routing. The RP2040 descriptor also retains its TBMAN zero-answering stub, which
this firmware does not access.
