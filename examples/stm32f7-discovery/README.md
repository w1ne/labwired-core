# STM32F7 Discovery Onboarding Example

Run all commands from `core/`.

## Purpose

This example provides deterministic bring-up for STM32F7 Discovery (STM32F746NG) using the minimal supported subset:
1. `rcc` (`stm32f4` profile @ `0x40023800`)
2. `gpioa`–`gpioi` (`stm32v2`)
3. `usart1` (`stm32v2` — ST-LINK VCP)
4. `systick`
5. stubs: LTDC, ETH, DMA2D, USB, QUADSPI

**SIM-DERIVED.** Not silicon-verified. No new GPIO/UART/RCC family models — F4-class reuse + Cortex-M7 DTCM map.

## Quick Run

```bash
cargo build -p firmware-stm32f746-demo --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test --script examples/stm32f7-discovery/uart-smoke.yaml --output-dir out/stm32f7-discovery/uart-smoke --no-uart-stdout
```

Expected result:
1. smoke test passes
2. UART contains `OK`

## Files

1. `system.yaml`: local board mapping for simulation runs.
2. `uart-smoke.yaml`: deterministic UART smoke assertion.
3. `REQUIRED_DOCS.md`: source-grounding references (RM0385, DS10916, UM1974).
4. `EXTERNAL_COMPONENTS.md`: external component declaration.
5. `VALIDATION.md`: reproducible validation/audit commands.
