# NUCLEO-F446RE

**Demand position:** STM32F4 ranks 7th in the MCU survey at 8%; STM32
Nucleo/Discovery is the 4th kit category at 23%. NUCLEO-F446RE is a current
F4 Nucleo-64 board, and the repository had no STM32F446 chip before this change.
This is a variant of the F4 family that the engine already models, not a new
core or vendor. ST listed STM32F446RE as Active and NUCLEO-F446RE as Active
and In Stock on 2026-11-13; see [`vendor-status.md`](vendor-status.md).

## Board wiring

| Board signal | MCU pin | Modelled path |
|---|---|---|
| Green user LED LD2 | PA5 | active-high GPIO output |
| User button B1 | PC13 | active-low GPIO input |
| ST-LINK virtual COM port | PA2 / PA3 | USART2 TX/RX, default solder-bridge route |

The board wiring follows ST UM1724. The chip descriptor
[`stm32f446re.yaml`](../../configs/chips/stm32f446re.yaml) declares only the
F401-subset peripherals at their shared F4 addresses: RCC, FLASH, PWR,
GPIOA/B/C, USART1/2, I2C1, SPI1, TIM1..TIM4, ADC1, IWDG, RTC, SYSCFG, EXTI
and DMA1/2. It does not declare the F446-only blocks (USART3, UART4/5, USART6,
SPI2..4, I2C2/3, FMPI2C1, CAN, SAI, QUADSPI, SPDIFRX, HDMI-CEC, DCMI, FMC,
DAC, ADC2/3, TIM5..TIM14, USB OTG, GPIOD/GPIOH). DBGMCU is a stub with no
IDCODE. The test firmware uses the reset 16 MHz HSI; it does not exercise the
PLL or the 180 MHz clock tree.

## Smoke

The committed firmware sends a marker over USART2 and toggles LD2. The smoke
checks the UART marker, the RCC enable bits, the GPIOA mode register and at
least four PA5 transitions.

Committed run evidence is in [`evidence/run/`](evidence/run/):
[`run_smoke.log`](evidence/run/logs/run_smoke.log),
[`result.json`](evidence/run/simulation-output/result.json), and the generated
[onboarding scoreboard](evidence/onboarding-scoreboard.md).

```sh
rustup target add thumbv7em-none-eabi
cargo build -p firmware-nucleo-f446re-smoke --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-f446re/io-smoke.yaml \
  --output-dir out/boards/nucleo-f446re/io-smoke --no-uart-stdout
```

The profile is SIM-DERIVED. It has no F446 silicon capture. The local CLI smoke
is not a hosted `model_verified` result or physical hardware confirmation.
