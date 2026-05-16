# STM32F401CDU6 Black Pill Onboarding Example

Run all commands from `core/`.

## Purpose

This example provides deterministic bring-up for the STM32F401CDU6 Black Pill using a broad STM32F401xB/xC memory-map descriptor. LabWired executable models are used for RCC, GPIO, SysTick, USART, I2C, SPI, general-purpose timers, ADC, and EXTI. Peripheral blocks without a dedicated F401-compatible model yet, such as DMA stream controllers, USB OTG FS, SDIO, FLASH control, CRC, PWR, RTC, watchdogs, DBG, SYSCFG, and I2S extension windows, are present as stubs so firmware can probe their addresses without aborting simulation.

Physical validation is limited to ST-Link USB/SWD sanity. No UART is physically connected for this onboarding path, so UART `OK` output is simulation evidence from the trace smoke run.

## Quick Run

```bash
cargo build -p firmware-f401cdu6-blackpill-demo --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test --script examples/stm32f401cdu6-blackpill/io-smoke.yaml --output-dir out/stm32f401cdu6-blackpill/io-smoke --no-uart-stdout
cargo run -q -p labwired-cli -- test --script examples/stm32f401cdu6-blackpill/trace-smoke.yaml --output-dir out/stm32f401cdu6-blackpill/trace-smoke --no-uart-stdout --trace --trace-max 128
cargo run -q -p labwired-cli -- test --script examples/stm32f401cdu6-blackpill/i2c-smoke.yaml --output-dir out/stm32f401cdu6-blackpill/i2c-smoke --no-uart-stdout --trace --trace-max 512
```

Expected result:
1. smoke test passes
2. USART2 contains `OK` in simulation artifacts
3. I2C1 smoke emits `I2C_OK` through simulated USART2 artifacts
4. stop reason is `max_steps`

## Files

1. `system.yaml`: local board mapping for simulation runs.
2. `io-smoke.yaml`: strict onboarding-compatible UART smoke assertion.
3. `trace-smoke.yaml`: deterministic UART and stop-reason smoke assertion.
4. `i2c-smoke.yaml`: deterministic I2C1 register/transfer smoke assertion.
5. `VALIDATION.md`: reproducible validation and audit commands.
