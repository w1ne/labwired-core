# NUCLEO-G474RE TIM1 PWM smoke

The NUCLEO-G474RE is an active STM32G474RE Nucleo-64 board. This firmware
configures the advanced-control TIM1 to produce edge-aligned, 50% duty PWM on
PA8 / TIM1_CH1 (AF6, Arduino D7) and prints `OK` on USART2. PA8 is available
on the board headers; an external LED and series resistor can be attached to
observe the output. In the digital twin, the smoke checks actual PA8 transitions
rather than only timer register values.

The measured demand target is **other STM32, rank #4 (13%)** in the 2025
JetBrains State of Embedded survey; Nucleo/Discovery kits rank #3 among kits
(23%). This is **maintenance/model-depth work on an already-supported board**,
not a new MCU or board onboarding.

```sh
cargo build --manifest-path examples/nucleo_g474re/board_firmware/Cargo.toml \
  --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo_g474re/tim1-pwm-smoke.yaml \
  --output-dir out/nucleo_g474re/tim1-pwm-smoke --no-uart-stdout
```

See [VALIDATION.md](VALIDATION.md) for the evidence command and scope limits,
and [vendor-status.md](vendor-status.md) for the live lifecycle/availability
check.
