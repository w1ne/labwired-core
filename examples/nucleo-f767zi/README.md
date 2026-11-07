# NUCLEO-F767ZI Onboarding Example

Run all commands from `core/`.

## Purpose

Deterministic bring-up for NUCLEO-F767ZI (STM32F767ZI, Cortex-M7, 2 MB flash,
512 KB SRAM) over the shared F4-profile peripheral models (RCC/GPIO/USART/
SysTick), with the F7-modern-I2C instances on the `stm32l4` profile.

## Quick Run

```bash
cargo build -p firmware-f401-demo --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test --script examples/nucleo-f767zi/uart-smoke.yaml --output-dir out/nucleo-f767zi/uart-smoke --no-uart-stdout
```

Expected result:
1. smoke test passes
2. UART contains `OK`

## Firmware reuse note

The smoke firmware is `firmware-f401-demo`, reused unchanged: Thumb-2/v7em
runs on the M7, and flash/SRAM bases and the USART2 mapping are identical
across F4/F7. What this proves and what it cannot: VALIDATION.md.

## Files

1. `system.yaml`: board mapping (LED1 on PB0, user button on PC13 per Nucleo-144).
2. `uart-smoke.yaml` / `io-smoke.yaml`: deterministic smoke assertions.
3. `REQUIRED_DOCS.md`: source-grounding references.
4. `EXTERNAL_COMPONENTS.md`: external component declaration.
5. `analog-zener-breakdown.yaml`: generic reverse-breakdown diode netlist routed to PA5 / ADC1_IN5.
6. `analog-jfet-gate-thermometer.yaml`: matched NJF gate-junction thermometers at 25 °C and 85 °C routed to PA6 / PA7 (ADC1_IN6 / ADC1_IN7).
