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
The board LED pinout is supported by WeAct/LaskaKit documentation, but has not
been measured on physical hardware. The broader chip profile still contains
stubbed blocks; the board LED smoke uses a separate narrow profile with only
RCC and GPIOC. See `VALIDATION.md`.

Remaining unverified details:

1. the DBGMCU IDCODE (no value is shipped; `dbg` is a stub), and
2. the exact F411 silicon register values and electrical behavior.

## Quick Run

Run the board-level PC13 LED smoke, using the focused RCC/GPIOC profile:

```bash
scripts/onboarding_smoke.sh \
  --target-id stm32f411ceu6-blackpill-led-smoke \
  --crate firmware-stm32f411-blackpill-smoke \
  --target thumbv7em-none-eabi \
  --script examples/stm32f411ceu6-blackpill/led-onboarding-smoke.yaml \
  --system configs/systems/stm32f411ceu6-blackpill-led-smoke.yaml \
  --out-dir examples/stm32f411ceu6-blackpill/evidence/led-smoke
```

The committed log and simulator output are under `evidence/led-smoke/`.
The hosted oracle record is `evidence/labwired-verification.md`.

The existing broad-chip self-test remains available:

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

1. `system.yaml` and `io-smoke.yaml`: broad F411 model self-test fixture.
2. `configs/systems/stm32f411ceu6-blackpill-led-smoke.yaml` and its chip profile:
   the board-level smoke scope, limited to PC13 LED output.
3. `led-onboarding-smoke.yaml`: firmware smoke script for the board LED.
4. `VALIDATION.md`: source record, reproducible commands and scope limits.
