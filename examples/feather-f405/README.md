# Feather-STM32F405 Onboarding Example

Run all commands from `core/`.

## Purpose

This example provides deterministic bring-up for a Feather-class STM32F405RG
board (Cortex-M4F, 1 MB flash, 192 KB SRAM — 128 KB main declared) using the
F4 shared peripheral profiles:
1. `rcc`
2. `gpio`
3. `uart`
4. `systick`

## Quick Run

```bash
cargo build -p firmware-f401-demo --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test --script examples/feather-f405/uart-smoke.yaml --output-dir out/feather-f405/uart-smoke --no-uart-stdout
```

Expected result:
1. smoke test passes
2. UART contains `OK`

## Firmware reuse note

The smoke firmware is the existing `firmware-f401-demo` crate, reused
unchanged: same Cortex-M4F core, same flash base (`0x08000000`), same SRAM
base (`0x20000000`), same USART2/PA5/PC13 mapping. The F405 descriptor is a
strict superset for everything this firmware touches. See `VALIDATION.md`
for what would invalidate the reuse.

## Files

1. `system.yaml`: local board mapping for simulation runs.
2. `uart-smoke.yaml`: deterministic UART smoke assertion.
3. `io-smoke.yaml`: strict onboarding smoke path.
4. `REQUIRED_DOCS.md`: source-grounding references.
5. `EXTERNAL_COMPONENTS.md`: external component declaration.

## Mixed-signal ADC example

[`analog-ntc-diode-system.yaml`](analog-ntc-diode-system.yaml) routes a generic
Beta NTC divider and a separate Shockley diode load line through PA0 and PA1 to
ADC1 channels 0 and 1. The integration test checks the solved voltages,
channel isolation, and firmware-visible one-shot conversions:

```sh
cargo test -p labwired-core --test cosim_feather_f405_analog
```
