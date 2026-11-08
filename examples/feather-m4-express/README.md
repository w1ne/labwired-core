# Adafruit Feather M4 Express

This profile uses the ATSAMD51J19A and covers the on-board D13 red LED on
PA23. The smoke boots bare-metal Cortex-M4 firmware and checks repeated PORTA
output transitions on that pin.

## Reproduce

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id feather-m4-express \
  --crate firmware-feather-m4-express-smoke \
  --target thumbv7em-none-eabi \
  --script examples/feather-m4-express/io-smoke.yaml \
  --system configs/systems/feather-m4-express.yaml \
  --out-dir examples/feather-m4-express/evidence
```

The committed logs and simulation output are in [`evidence/`](evidence/).
Current board and MCU lifecycle checks are recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Model boundary

The chip descriptor exposes only PORTA and SysTick. It maps PA23 as Feather
D13 and the on-board red LED. USB/CDC, SERCOM/UART, QSPI flash, the NeoPixel,
charging, ADC/DAC, and other SAMD51 blocks are omitted and not claimed.
