# NUCLEO-F401RE Onboarding Example

Run all commands from `core/`.

## Purpose

This example binds the board to the exact `stm32f401re` chip descriptor. Its
onboarding smoke emits a USART2 marker and toggles the configured GPIOA5 LED
output. The board button, external buses, and ST-LINK hardware are outside
this smoke's coverage.

## Quick Run

```bash
cargo build -p firmware-f401-demo --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test --script examples/nucleo-f401re/io-smoke.yaml --output-dir out/nucleo-f401re/io-smoke --no-uart-stdout
```

Expected result:
1. smoke test passes
2. USART2 contains `NUCLEO_F401RE_OK`
3. GPIOA5 records at least eight output edges

## Vendor status

Checked 2026-11-03: [ST's NUCLEO-F401RE product page](https://www.st.com/en/evaluation-tools/nucleo-f401re.html)
lists the board as **Active**; [ST's eStore listing](https://estore.st.com/en/nucleo-f401re-cpn.html)
lists it as **Active**, **In stock**, and $18.55 for one unit. ST's
[STM32F401RE product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f401re.html)
lists STM32F401RET6 as **Active**.

## Files

1. `system.yaml`: local board mapping for simulation runs.
2. `uart-smoke.yaml`: legacy USART2 smoke assertion.
3. `io-smoke.yaml`: UART plus GPIO onboarding smoke path.
4. `REQUIRED_DOCS.md`: source-grounding references.
5. `EXTERNAL_COMPONENTS.md`: external component declaration.
6. `VALIDATION.md`: reproducible validation/audit commands.
