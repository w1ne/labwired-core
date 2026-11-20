# NUCLEO-H723ZG

**Demand position:** `other STM32` ranks 4th in the MCU survey at 13%;
STM32 Nucleo/Discovery is the 4th kit category at 23%. This round is maintenance
on this already-onboarded board: it adds main-push CI coverage for the existing
smoke, without adding a new MCU or peripheral model. ST currently lists
STM32H723ZGT6 as Active and in volume production, and its store lists
NUCLEO-H723ZG as Active and In Stock; see [`vendor-status.md`](vendor-status.md).

## Board wiring

| Board signal | MCU pin | Modelled path |
|---|---|---|
| Green user LED LD1 | PB0 | active-high GPIO output |
| Yellow user LED LD2 | PE1 | active-high GPIO output |
| Red user LED LD3 | PB14 | active-high GPIO output |
| User button B1 | PC13 | active-high GPIO input |
| ST-LINK virtual COM port | PD8 / PD9 | USART3 TX/RX, default solder-bridge route |

The board wiring follows ST UM2407. The profile models the H7 RCC, GPIOB/C/D/E,
USART3 and SysTick needed for these board signals. FLASH_ACR and the exact
CMSIS FMC bank-disable write are handled for startup; flash programming and
FMC operations are not modeled. Ethernet, USB, ADC, timers, DMA and other H723
peripherals are not mapped. The test firmware uses the reset HSI clock; it does
not exercise the H7 PLL or external oscillator.

## Smoke

The committed firmware sends a marker over USART3 and toggles LD1. The smoke
checks the UART marker and at least four PB0 transitions.

Committed run evidence is in [`evidence/run/`](evidence/run/):
[`run_smoke.log`](evidence/run/logs/run_smoke.log),
[`result.json`](evidence/run/simulation-output/result.json), and the generated
[onboarding scoreboard](evidence/onboarding-scoreboard.md).

```sh
rustup target add thumbv7em-none-eabi
./examples/nucleo-h723zg/ci/build.sh
ONBOARDING_EVIDENCE_DIR=examples/nucleo-h723zg/evidence/run \
  ./examples/nucleo-h723zg/ci/test.sh
```

The profile is SIM-DERIVED. It has no H723 silicon capture. The local CLI smoke
is not a hosted `model_verified` result or physical hardware confirmation.

The exact board smoke is included in the core board CI matrix on main pushes
and in the nightly onboarding smoke workflow. The CI test writes its build and
run logs under `out/boards/nucleo-h723zg/`.
