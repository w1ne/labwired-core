# NUCLEO-L031K6 — STM32L031K6 on the STM32L0 models

The STM32L031K6 (Arm Cortex-M0+, 32 MHz, 32 KiB flash, 8 KiB SRAM,
UFQFPN32) is modelled with its own instance set and memory map from ST's
`stm32l031xx.h` (RM0377) and UM1956, and reuses the STM32L0 family register
models for the blocks that exist on this die. The Nucleo-32 board has LD3
on PB3 (Arduino D13; UM1956 Table 8, SB15 default ON) and USART2 on PA2/PA15
to the ST-LINK virtual COM port (UM1956 Table 7, SB2/SB3 default ON).

**Scope of the PR-gated smoke:** ARM reset, valid SRAM stack, RCC IOPENR/APB1
enables, PA2 AF4, USART2 TX, and LED edges on PB3. USB, LCD, DAC, LPTIM and
other L0x3 blocks that this die does not have are not declared. No Arduino
matrix row and no physical hardware comparison have been run for this chip.

## Reproduce

From the labwired-core repository root:

```sh
./examples/nucleo-l031k6/firmware/build.sh
labwired test --script examples/nucleo-l031k6/io-smoke.yaml \
  --output-dir out/nucleo-l031k6-smoke --no-uart-stdout
```

Expected: `3/3` passing (UART contains `STM32L031K6_OK`; PB3 at least four
edges; reaches the step budget without a simulation error).

The ELF in `tests/fixtures/board-smoke/stm32l031k6.elf` is committed so the
PR-gated lane can run it without an ARM compiler. Its source is
`firmware/main.c` and `firmware/linker.ld`.
See `VALIDATION.md` for the recorded evidence.

## Analog inputs

`analog-ntc-diode-pa0-pa1-system.yaml` drives a generic Beta-parameter NTC
divider into `board.analog.pa0_volts` (ADC_IN0) and a Shockley diode load
line into `board.analog.pa1_volts` (ADC_IN1). The descriptor's
`analog_pins:` map is the LQFP32 ADC set from ST STM32_open_pin_data
(PA0-PA7 = IN0-IN7, PB0 = IN8, PB1 = IN9), so a deck can route to any of these pads. The test is
`crates/core/tests/analog/cosim_stm32wb_g4_l0_analog.rs`
(`cargo test -p labwired-core --test analog cosim_stm32wb_g4_l0`).
