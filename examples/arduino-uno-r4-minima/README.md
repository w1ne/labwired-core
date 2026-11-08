# Arduino Uno R4 Minima Onboarding Example

Run all commands from `core/`.

## Purpose

This example provides deterministic bring-up for Arduino Uno R4 Minima (R7FA4M1AB) using the minimal supported subset:
1. `ra_sysc` (HOCO / OSCSF)
2. `port1` / `port3` (`ra_port`)
3. `sci2` UART (header Serial on P301/P302)
4. `systick`

USB CDC Serial is intentionally out of scope (USBFS is a stub window only).

**SIM-DERIVED.** Not silicon-verified. The onboarding smoke uses the shipped
`configs/systems/arduino-uno-r4-minima.yaml` descriptor and asserts both SCI2
output (`OK`) and a transition on the built-in D13 LED (P111). USB CDC remains
out of scope; its `usbfs` window is an explicit stub and this firmware does not
access it.

## Quick Run

```bash
cargo build -p firmware-ra4m1-demo --release --target thumbv7em-none-eabi
scripts/onboarding_smoke.sh \
  --target-id arduino-uno-r4-minima \
  --crate firmware-ra4m1-demo \
  --target thumbv7em-none-eabi \
  --script examples/arduino-uno-r4-minima/io-smoke.yaml \
  --system configs/systems/arduino-uno-r4-minima.yaml \
  --out-dir examples/arduino-uno-r4-minima/evidence
```

Expected result:
1. smoke test passes
2. SCI2 output contains `OK`
3. PORT1/P111 records at least one GPIO edge

## Files

1. `system.yaml`: local board mapping for the existing focused example tests.
2. `io-smoke.yaml`: deterministic onboarding assertion for SCI2 and D13 GPIO.
3. `REQUIRED_DOCS.md`: source-grounding references (R01UH0887, pinout; FSP PAC as address cross-check only).
4. `EXTERNAL_COMPONENTS.md`: external component declaration.
5. `VALIDATION.md`: reproducible validation/audit commands.

## Vendor status checked 2026-11-03

Arduino's [live UNO R4 Minima shop page](https://store-usa.arduino.cc/collections/boards-modules/products/uno-r4-minima)
listed SKU ABX00080 at **$20.00** with **Add to cart**. The board uses
Renesas R7FA4M1AB3CFM#AA0; Renesas's [live part page](https://www.renesas.com/en/products/ra4m1/part-details/r7fa4m1ab3cfm-aa0)
states **Active**, and its [RA4M1 order table](https://www.renesas.com/en/products/ra4m1?tab=software-tools)
listed that package **In Stock**.

The modeled/claimed board surface for this smoke is limited to RA4M1 reset and
clock readiness, SCI2 on D0/D1, PORT1/P111 on D13, and SysTick. USB CDC,
analog input, DAC, CAN, USB HID, and other peripherals are not exercised or
claimed. The board has no routed power net in this simulation, so VDD is
assumed ideal and power-on/brown-out reset is not simulated. No silicon capture
is available for this board.
