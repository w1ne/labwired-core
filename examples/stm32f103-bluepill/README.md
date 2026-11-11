# STM32F103C8 Blue Pill reference profile

## Demand rank

This is a board-level addition for the STM32F103 family, MCU demand rank **#3**
(17% in the JetBrains State of Embedded 2025 survey). ATmega328P ranks #2, but
ST's live check found that part NRND, so it is excluded from new onboarding.

## Copy-paste smoke

From the repository root, build and run the committed no-std Rust firmware:

```bash
cargo build -p firmware-stm32f103-bluepill-smoke --release --target thumbv7m-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/stm32f103-bluepill/io-smoke.yaml \
  --output-dir out/stm32f103-bluepill/io-smoke --no-uart-stdout
```

The source is [`crates/firmware-stm32f103-bluepill-smoke/src/main.rs`](../../crates/firmware-stm32f103-bluepill-smoke/src/main.rs).
It keeps the MCU on reset HSI at 8 MHz, prints `BLUEPILL_F103C8_OK` on USART1
(PA9/PA10), and toggles the active-low PC13 user LED.

## Scope

This profile models only RCC, GPIOA, GPIOC, USART1, flash/RAM geometry, the
PC13 LED, and USART1 access on the header. It does not claim the board's USB
connector, USB boot/programming, onboard reset/boot-jumper interaction, or a
USB-to-UART bridge. The full chip descriptor remains available separately;
this narrowed profile declares no zero-answering stub peripherals. Copies with
cloned or relabelled MCUs may not match the profile's behavior.

See [vendor status and source notes](vendor-status.md) and the
[validation record](VALIDATION.md).
