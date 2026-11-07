# Adafruit Feather nRF52840 Express

This focused profile boots nRF52840 firmware and sends
`FEATHER_NRF52840_UART_OK` over UARTE0 on Feather D0/TX (P0.25). The smoke
asserts the captured UART output.

## Reproduce

From the repository root:

```bash
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id adafruit-feather-nrf52840-express \
  --crate firmware-feather-nrf52840-express-smoke \
  --target thumbv7em-none-eabi \
  --script examples/adafruit-feather-nrf52840-express/io-smoke.yaml \
  --system configs/systems/adafruit-feather-nrf52840-express.yaml \
  --out-dir examples/adafruit-feather-nrf52840-express/evidence
```

The committed local smoke logs and simulation output are stored under
`evidence/`. The purchase check is in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Model boundary

The board profile includes the nRF52840 memory map, P0 GPIO configuration for
P0.25, and UARTE0. It claims only UART TX on D0/P0.25. The board's USB CDC,
Bluetooth/radio, user LEDs, NeoPixel, QSPI flash, battery charger, buttons, and
other MCU peripherals are omitted from this focused profile.

Adafruit's board variant maps D0/TX to P0.25 and D1/RX to P0.24. The firmware
selects P0.25 for UARTE0 TX and configures the pad as an output before starting
the transfer. See the official [variant source][variant] and [variant header][header].

[variant]: https://github.com/adafruit/Adafruit_nRF52_Arduino/blob/master/variants/feather_nrf52840_express/variant.cpp
[header]: https://github.com/adafruit/Adafruit_nRF52_Arduino/blob/master/variants/feather_nrf52840_express/variant.h
