# Raspberry Pi Pico 2 W (RP2350) BME280 smoke

The smoke boots a bare-metal Cortex-M33 image, configures I2C0 on GP4/GP5,
reads BME280 `CHIP_ID` (`0xD0`) and raw temperature (`0xFA..0xFC`) at address
`0x77`, then reports the values over the simulator's UART0 diagnostic capture.
It does not drive or assert the Pico 2 W's CYW43439-controlled WL_GPIO0 LED.

Run from the repository root:

```bash
rustup target add thumbv8m.main-none-eabi  # once
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id rp2350-pico2w-bme280 \
  --crate firmware-rp2350-pico2w-bme280-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/rp2350-pico2w-bme280/bme280-smoke.yaml \
  --system configs/systems/rp2350-pico2w-bme280.yaml \
  --out-dir examples/rp2350-pico2w-bme280/evidence
```

The acceptance checks are the BME280 ID `0x60`, temperature sample `0x082DC0`,
and the expected max-step stop. This is simulator evidence, not physical board
confirmation. The descriptor omits the integrated Infineon CYW43439 Wi-Fi and
Bluetooth device; it also omits the onboard LED that the CYW43439 drives through
WL_GPIO0. TBMAN in the RP2350 chip descriptor remains a zero-answering stub.

The separate hosted LabWired catalog has no Pico 2 W board ID (its `rp2350`
alias resolves to the Waveshare RP2350-Zero). I did not use that different board
for an oracle check. The pass recorded here is from this repository's exact
RP2350 chip descriptor and system profile.
