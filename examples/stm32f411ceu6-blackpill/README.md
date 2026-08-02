# STM32F411CEU6 Black Pill Onboarding Example

Run all commands from `core/`.

## Purpose

Deterministic bring-up for the STM32F411CEU6 (WeAct "Black Pill") against a
broad STM32F411xC/xE memory-map descriptor. LabWired executable models cover
RCC, GPIO, SysTick, USART1/2/6, I2C1/2/3, SPI1/2/3/4/**5**, TIM1/2/3/4/5/9/10/11,
ADC1, IWDG, RTC and EXTI. Blocks with no F411-compatible model yet — DMA1/2
stream controllers, WWDG, SDIO, SYSCFG, CRC, USB OTG FS windows, the I2S
extension windows and DBGMCU — are declared as stubs so firmware can probe their
addresses without aborting simulation.

`spi5` is the only peripheral instance the F411 has that the F401 does not. It
reuses the same classic-SPI model as SPI1–SPI4; see the chip yaml for the
sourcing of its base, IRQ and clock-gate bit.

## Fidelity

**Sim-derived, not silicon-verified.** No F411 part was on a bench for this
onboarding. Every register value came from ST's CMSIS header
(`stm32f411xe.h`) and the vendored `tests/fixtures/real_world/stm32f411.svd`.
Two things are explicitly *not* established — see `VALIDATION.md`:

1. the DBGMCU IDCODE (no value is shipped; `dbg` is a stub), and
2. the Black Pill LED/button pinout (carried over from the F401 Black Pill).

## Quick Run

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/stm32f411ceu6-blackpill/io-smoke.yaml \
  --output-dir out/stm32f411ceu6-blackpill/io-smoke \
  --no-uart-stdout
```

Expected result:

1. exit code `0`
2. `out/stm32f411ceu6-blackpill/io-smoke/uart.log` contains the full TIER1
   transcript — `clock`, `gpio`, `timer`, `i2c`, `spi`, `adc`, `wdt`, `rtc` all
   `PASS`, then `TIER1 done`
3. stop reason is `max_steps`

## Files

1. `system.yaml`: local board mapping for simulation runs.
2. `io-smoke.yaml`: strict-onboarding smoke — runs the committed tier-1 fixture
   blob and asserts its TIER1 self-test transcript.
3. `VALIDATION.md`: reproducible validation commands and the honest scope.
