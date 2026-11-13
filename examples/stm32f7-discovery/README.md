# STM32F7 Discovery Onboarding Example

Run all commands from `core/`.

## Purpose

This example provides deterministic bring-up for STM32F7 Discovery (STM32F746NG) using the minimal supported subset:
1. `rcc` (`stm32f4` profile @ `0x40023800`)
2. `gpioa`–`gpioi` (`stm32v2`)
3. `usart1` (`stm32v2` — ST-LINK VCP)
4. `adc1` (shared F4-profile 12-bit SAR model; PA0/PA1/PA2 analog routes)
5. `systick`
6. stubs: LTDC, ETH, DMA2D, USB, QUADSPI

**SIM-DERIVED.** Not silicon-verified. No new GPIO/UART/RCC family models — F4-class reuse + Cortex-M7 DTCM map.

## Quick Run

```bash
cargo build -p firmware-stm32f746-demo --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test --script examples/stm32f7-discovery/uart-smoke.yaml --output-dir out/stm32f7-discovery/uart-smoke --no-uart-stdout
```

Expected result:
1. smoke test passes
2. UART contains `OK`

The mixed-signal fixtures route a generic Beta NTC divider, a Shockley diode load line,
and a Gummel–Poon NPN common-emitter load to PA0/ADC1_IN0, PA1/ADC1_IN1, and
PA2/ADC1_IN2. The board-specific integration tests assert independent load-line
references and firmware-visible ADC1 results while checking that the three channels stay
isolated. The BJT charge test compares the uncharged load line with CJE, CJC, TF, and
combined charge cases over the same PA2 route. This is simulation-model coverage, not a
silicon-validation claim.

## Files

1. `system.yaml`: local board mapping for simulation runs.
2. `uart-smoke.yaml`: deterministic UART smoke assertion.
3. `REQUIRED_DOCS.md`: source-grounding references (RM0385, DS10916, UM1974).
4. `EXTERNAL_COMPONENTS.md`: external component declaration.
5. `VALIDATION.md`: reproducible validation/audit commands.
