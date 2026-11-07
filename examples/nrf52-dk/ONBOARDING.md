# nRF52 DK (PCA10040) onboarding

The nRF52 DK system profile covers the board's nRF52832, four active-low user
LEDs, four active-low buttons, and the UARTE0 transmit path to the J-Link
virtual COM port. The firmware smoke walks the LEDs and reports their output
state over UARTE0.

## Board map

| Board signal | MCU signal | Modeled behavior |
|---|---|---|
| LED1 | P0.17 | Active-low GPIO output; exercised by the smoke |
| LED2 | P0.18 | Active-low GPIO output; exercised by the smoke |
| LED3 | P0.19 | Active-low GPIO output; exercised by the smoke |
| LED4 | P0.20 | Active-low GPIO output; exercised by the smoke |
| Button 1–4 | P0.13–P0.16 | Active-low GPIO inputs with internal pull-ups; mapped, not exercised |
| J-Link VCOM TX | UARTE0 TXD, P0.06 | Host-captured serial output; exercised by the smoke |

The board descriptor is [`configs/systems/nrf52-dk.yaml`](../../configs/systems/nrf52-dk.yaml),
using [`configs/chips/nrf52832.yaml`](../../configs/chips/nrf52832.yaml). The
example firmware is [`crates/firmware-nrf52-dk-demo`](../../crates/firmware-nrf52-dk-demo).

## Reproduce

```bash
scripts/onboarding_smoke.sh \
  --target-id nrf52-dk \
  --crate firmware-nrf52-dk-demo \
  --target thumbv7em-none-eabi \
  --script examples/nrf52-dk/io-smoke.yaml \
  --system configs/systems/nrf52-dk.yaml \
  --out-dir examples/nrf52-dk/evidence

python3 scripts/generate_onboarding_scoreboard.py \
  --metrics-root examples/nrf52-dk/evidence \
  --markdown-out examples/nrf52-dk/evidence/onboarding-scoreboard.md \
  --json-out examples/nrf52-dk/evidence/onboarding-scoreboard.json
```

The committed `evidence/` directory contains the build and smoke logs, UART
transcript, simulation result, and scoreboard. The captured UART text is
normalized to LF line endings for the committed log. The scripted smoke checks
the ready banner and the LED on/off sequence. Firmware reads the GPIO OUT
register back after each write before reporting the LED state.

## Model boundary

The profile does not model Bluetooth/2.4 GHz radio behavior, NFC, the external
J-Link/interface MCU, USB enumeration, or power measurement. The UARTE0 host
console represents the board's VCOM link; it does not simulate that link's
interface MCU. This is local simulation evidence, not a physical-board capture.

See [`evidence/vendor-status.md`](evidence/vendor-status.md) for the live
product and buyability check.
