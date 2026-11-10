# Raspberry Pi Pico (RP2040) BME280 smoke

This smoke boots a bare-metal Cortex-M0+ image on the RP2040 Pico profile. It
configures I2C0 for the Pico's GP4/GP5 pins, reads BME280 `CHIP_ID` (`0xD0`)
and raw temperature (`0xFA..0xFC`) from the modeled sensor at address `0x77`,
then toggles the onboard LED on GP25 twice. UART0 output is captured as
simulator diagnostics; the Pico has no onboard USB CDC bridge.

Build and run from the repository root:

```bash
rustup target add thumbv6m-none-eabi   # once
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id rp2040-pico-bme280 \
  --crate firmware-rp2040-pico-bme280-smoke \
  --target thumbv6m-none-eabi \
  --script examples/rp2040-pico-bme280/bme280-smoke.yaml \
  --system configs/systems/rp2040-pico-bme280.yaml \
  --out-dir examples/rp2040-pico-bme280/evidence
```

The committed evidence requires the sensor ID `0x60`, raw temperature
`0x082DC0`, at least four GP25 output edges, and the expected max-step stop.
This is local simulator evidence, not hardware confirmation. The board profile
uses the existing RP2040 descriptor; TBMAN remains a zero-answering stub, and
the smoke does not exercise USB, the second CPU, or PIO.
