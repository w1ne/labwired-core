# NUCLEO-H723ZG

**Demand position:** `other STM32` ranks 4th in the MCU survey at 13%;
STM32 Nucleo/Discovery is the 4th kit category at 23%. This board adds a
current STM32H7 Nucleo-144 target that was missing from the runnable board configs.
ST listed STM32H723ZGT6 as Active and NUCLEO-H723ZG as Active and In Stock on
2026-11-13; see [`vendor-status.md`](vendor-status.md).

## Board wiring

| Board signal | MCU pin | Modelled path |
|---|---|---|
| Green user LED LD1 | PB0 | active-high GPIO output |
| Yellow user LED LD2 | PE1 | active-high GPIO output |
| Red user LED LD3 | PB14 | active-high GPIO output |
| User button B1 | PC13 | active-high GPIO input |
| ST-LINK virtual COM port | PD8 / PD9 | USART3 TX/RX, default solder-bridge route |

The board wiring follows ST UM2407. The profile models the H7 RCC, GPIOB/C/D/E,
USART3 and SysTick needed for these board signals. It does not map the Ethernet,
USB, ADC, timers, DMA or other H723 peripherals. The test firmware uses the
reset HSI clock; it does not exercise the H7 PLL or external oscillator.

## Smoke

The committed firmware sends a marker over USART3 and toggles LD1. The smoke
checks the UART marker and at least four PB0 transitions.

Committed run evidence is in [`evidence/run/`](evidence/run/):
[`run_smoke.log`](evidence/run/logs/run_smoke.log),
[`result.json`](evidence/run/simulation-output/result.json), and the generated
[onboarding scoreboard](evidence/onboarding-scoreboard.md).

```sh
rustup target add thumbv7em-none-eabi
cargo build -p firmware-nucleo-h723zg-smoke --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-h723zg/io-smoke.yaml \
  --output-dir out/boards/nucleo-h723zg/io-smoke --no-uart-stdout
```

The profile is SIM-DERIVED. It has no H723 silicon capture. The local CLI smoke
is not a hosted `model_verified` result or physical hardware confirmation.
