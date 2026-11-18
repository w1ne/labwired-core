# NUCLEO-F767ZI Onboarding Example

Run all commands from `core/`.

## Purpose

Deterministic bring-up for NUCLEO-F767ZI (STM32F767ZI, Cortex-M7, 2 MB flash,
512 KB SRAM) over the shared F4-profile peripheral models (RCC/GPIO/USART/
SysTick), with the F7-modern-I2C instances on the `stm32l4` profile.

## Quick Run

```bash
cargo run -q -p labwired-cli -- test --script examples/nucleo-f767zi/io-smoke.yaml --output-dir out/nucleo-f767zi/io-smoke --no-uart-stdout
```

The pinned STM32duino fixture prints `F767_OK` and toggles PB0 in simulation.
Its source and build provenance are in `tests/fixtures/stm32f767-stock-arduino/`.
This does not establish physical-board behavior.

## Files

1. `system.yaml`: board mapping (LED1 on PB0, user button on PC13 per Nucleo-144).
2. `uart-smoke.yaml` / `io-smoke.yaml`: deterministic smoke assertions.
3. `REQUIRED_DOCS.md`: source-grounding references.
4. `EXTERNAL_COMPONENTS.md`: external component declaration.
