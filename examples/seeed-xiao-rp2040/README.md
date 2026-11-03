# Seeed Studio XIAO RP2040

This board profile uses the board's Raspberry Pi RP2040 and exercises its
onboard red, green and blue indicator LEDs on GPIO17, GPIO16 and GPIO25. The
firmware sets the pins to SIO output, then toggles all three; the LEDs are
active-low.

Run the smoke from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id seeed-xiao-rp2040 \
  --crate firmware-xiao-rp2040-smoke \
  --target thumbv6m-none-eabi \
  --script examples/seeed-xiao-rp2040/io-smoke.yaml \
  --system configs/systems/seeed-xiao-rp2040.yaml \
  --out-dir examples/seeed-xiao-rp2040/evidence
```

The smoke checks GPIO transitions on all three LED pins. Its committed build
and simulation evidence is under [`evidence/`](evidence/).

## Scope

- Modeled and exercised: application startup from mapped flash, IO_BANK0
  function selection, SIO output enable/output state and the three LED loads.
- Not modeled or claimed: USB/UF2 boot, UART, I2C, SPI, ADC, PWM, battery
  management or the remaining header pins.
- No peripheral uses the engine's zero-answering stub path.

See [`evidence/vendor-status.md`](evidence/vendor-status.md) for the live
availability check and [`VALIDATION.md`](VALIDATION.md) for source references.
