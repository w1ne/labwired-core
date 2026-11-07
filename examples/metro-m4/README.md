# Adafruit Metro M4 Onboarding Example

Run all commands from `core/`.

## Purpose

This profile provides a deterministic bring-up for the Adafruit Metro M4 Express
(SAMD51J19A) using only the working path exercised by the firmware:

- OSCCTRL, MCLK, and GCLK provide the reset DFLL48M source and enable
  SERCOM3's core clock.
- SERCOM3 drives Serial1 on PA22/PA23 with the documented peripheral mux and
  115200-baud USART configuration.
- PORTA drives the on-board D13 LED on PA16.

The smoke keeps the MCU on its reset DFLL48M clock at 48 MHz; it does not
configure the higher-speed DPLL clock used by full Arduino sketches.

The focused chip descriptor omits QSPI flash, native USB, the NeoPixel, and
unused MCU peripherals. Serial output represents the header UART, not USB CDC.

**SIM-DERIVED.** Not silicon-verified. Bench silicon may be ItsyBitsy M4 (SAMD51G19A); Playground pinout is Metro M4.

## Quick Run

```bash
scripts/onboarding_smoke.sh \
  --target-id metro-m4 \
  --crate firmware-atsamd51-demo \
  --target thumbv7em-none-eabi \
  --script examples/metro-m4/onboarding-smoke.yaml \
  --system configs/systems/metro-m4.yaml \
  --out-dir examples/metro-m4/evidence
```

Expected result:
1. the test passes with `METRO_M4_OK` on SERCOM3
2. the PA16 LED output records at least one edge

The committed stage logs, simulation output, metrics, and scoreboard are in
[`evidence/`](evidence/). The live board and MCU status receipt is
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Files

1. `system.yaml`: local system manifest referencing the focused chip profile.
2. `onboarding-smoke.yaml`: UART and GPIO evidence assertions.
3. `uart-smoke.yaml`: shorter compatibility smoke.
4. `REQUIRED_DOCS.md`: source-grounding references (DS60001507, pinout).
5. `EXTERNAL_COMPONENTS.md`: external component declaration.
6. `VALIDATION.md`: reproducible validation/audit commands.
