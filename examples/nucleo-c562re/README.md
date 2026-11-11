# NUCLEO-C562RE quick start

## Demand rank

This is a **demand rank #4** pick: other STM32 MCUs account for 13% of MCU use
in the measured survey. Nucleo/Discovery kits appear in 23% of projects. The
board is a current STM32C5 Nucleo system; this adds a new STM32 family to the
working board coverage. See [the live vendor check](vendor-status.md).

## Copy-paste smoke

From the repository root, build and run the smoke with:

```bash
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-c562re \
  --crate firmware-nucleo-c562re-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/nucleo-c562re/io-smoke.yaml \
  --system configs/systems/nucleo-c562re.yaml \
  --out-dir examples/nucleo-c562re/evidence
```

The smoke requires `C562_USART2_OK` from the USART2 register model, at least
eight LD1/PA5 GPIO edges, and the expected max-step stop. The example firmware
is [src/main.rs](../../crates/firmware-nucleo-c562re-smoke/src/main.rs).

The profile models the STM32C562RET6, the RCC GPIOA/USART2 clock gates, GPIOA,
USART2, SysTick, and LD1 on PA5. The USART2 mapping to the board's ST-LINK VCP
uses PA2/PA3. The engine captures UART register output; it does not model the
ST-LINK bridge or verify UART pad waveforms. USB, CAN FD, ADC, I3C, timers,
other GPIO ports, and other board connectors are outside this profile.
