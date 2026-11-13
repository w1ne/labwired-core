# Arduino Nano classic (ATmega328P) maintenance validation

This is maintenance for the existing Nano firmware path, not new hardware
onboarding. The demand survey ranks ATmega328 #2 (30%) and Arduino kits #3
(37%); Microchip's live page still marks ATmega328P **Not Recommended for new
designs**, checked 2026-11-13 with the real Chrome profile. Do not use this
record as a recommendation for a new design.

## Smoke

Run from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id arduino-nano-atmega328p-maintenance \
  --script examples/arduino-nano-blinky/io-smoke.yaml \
  --system configs/systems/arduino-nano.yaml \
  --out-dir examples/arduino-nano-blinky/evidence \
  --firmware-build-script examples/arduino-nano-blinky/build-firmware.sh
```

The smoke builds the stock Arduino sketch with PlatformIO and runs it through
the Nano system manifest. It requires the USART0 `nano-ok` marker and execution
to the full 2,000,000-step budget without a fault. The sketch also toggles the
board's PB5/D13 LED. `cargo test -p labwired-core --test avr_nano_live_run`
checks that PB5 reaches both levels and toggles while `nano-ok` is emitted.

Evidence is committed under `evidence/`: the build and run logs, structured
simulation result, and maintenance metrics. This is digital-twin evidence; no
physical Nano, USB-serial bridge, bootloader, or silicon register capture was
tested. The model uses the ideal configured supply and does not establish
brown-out behavior.

## Hosted model verification

The same Arduino sketch was also submitted to the hosted LabWired verifier for
`arduino-nano`. Its deterministic oracle returned `status: model_verified` for
both the `nano-ok` serial marker and D13 toggling. The result is saved in
[`evidence/model-verification.json`](evidence/model-verification.json). The
verifier reports one non-blocking approximation: no power net is routed, so the
MCU uses an ideal 5 V rail and power-on/brown-out reset behavior is not covered.

## Model boundary

The smoke exercises the modeled ATmega328P USART0 transmit path and PB5 GPIO.
The USB-serial adapter is outside this system profile. Other unsupported
peripherals are outside the smoke's claims.
