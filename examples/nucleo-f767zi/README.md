# NUCLEO-F767ZI Onboarding Example

Run all commands from `core/`.

## Purpose

Deterministic bring-up for NUCLEO-F767ZI (STM32F767ZI, Cortex-M7, 2 MB flash,
512 KB SRAM) over the shared F4-profile peripheral models (RCC/GPIO/USART/
SysTick), with the F7-modern-I2C instances on the `stm32l4` profile.
ST listed the MCU as Active / in volume production and the NUCLEO-F767ZI as
In Stock on 2026-11-03; see the [live-status receipt](evidence/vendor-status.md).

## Quick Run

```bash
(cd examples/tier1-fixture/stm32f767 && cargo build -p tier1-fixture-stm32f767 --release --target thumbv7em-none-eabi)
cargo run -q -p labwired-cli -- test --script examples/nucleo-f767zi/onboarding-smoke.yaml --output-dir out/nucleo-f767zi/onboarding-smoke --no-uart-stdout
```

The part-specific firmware checks the USART3 VCP setup, board LED GPIO, clock,
timer, I2C, SPI, ADC, watchdog, and RTC behavior. The expected transcript is
asserted by [`onboarding-smoke.yaml`](onboarding-smoke.yaml).

## Firmware reuse note

The separate minimal UART example reuses `firmware-f401-demo`. The onboarding
smoke builds the F767-specific raw-register fixture from
[`examples/tier1-fixture/stm32f767`](../tier1-fixture/stm32f767/), so it checks
the F767 descriptor rather than relying on a sibling board's firmware.
Validation scope and unsupported peripherals are documented in
[`VALIDATION.md`](VALIDATION.md) and [`docs/boards/stm32f767.md`](../../docs/boards/stm32f767.md).

The smoke run, stage logs, metrics, and scoreboard are committed under
[`evidence/`](evidence/).

## Files

1. `system.yaml`: board mapping (LED1 on PB0, user button on PC13 per Nucleo-144).
2. `uart-smoke.yaml` / `io-smoke.yaml`: deterministic smoke assertions.
3. `REQUIRED_DOCS.md`: source-grounding references.
4. `EXTERNAL_COMPONENTS.md`: external component declaration.
