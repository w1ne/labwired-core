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

[`analog-ntc-diode-system.yaml`](analog-ntc-diode-system.yaml) exercises the
generic analog adapter with a Beta NTC divider and an unrelated Shockley diode
load line. The test routes the solved voltages through the F411 descriptor's
PA1/PA4 ADC aliases and checks ADC1 one-shot conversion results. This proves
the modeled chip path; the Black Pill connector pinout and silicon behavior
remain unverified.

[`analog-mosfet-switch.yaml`](analog-mosfet-switch.yaml) adds a generic level-1
NMOS load line on PA0/ADC1_IN0. Its co-simulation test checks the independently
calculated drain voltage across gate bias points, the firmware-visible ADC1
conversion, and isolation from the NTC and diode channels.

[`analog-jfet-gate-charge-adc.yaml`](analog-jfet-gate-charge-adc.yaml) compares
matched level-1 NJF branches with and without CGS/CGD depletion charge on
PA2/PA3. The co-simulation test checks the transient against an independent
backward-Euler charge recurrence, converts both gate voltages through ADC1, and
keeps the unrelated NTC and diode routed on PA1/PA4.

[`analog-bjt-high-injection-adc.yaml`](analog-bjt-high-injection-adc.yaml)
compares matched generic Gummel–Poon NPN branches with and without IKF/IKR/NKF
rolloff on PA5/PA6. The test checks both load lines against an independent
reference, asserts ADC1 conversions, and keeps the unrelated NTC and diode on
PA1/PA4.

[`analog-bjt-early-effect-adc.yaml`](analog-bjt-early-effect-adc.yaml) compares
matched generic Gummel–Poon NPN branches with finite VAF and no forward Early
effect on PA2/PA3. The test checks independent load-line roots at four base
drives, asserts ADC1 conversions, and keeps the unrelated NTC and diode on
PA1/PA4.

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
3. `analog-ntc-diode-system.yaml`: reusable generic NTC and diode analog models
   routed to the F411's modeled ADC1 inputs.
4. `analog-mosfet-switch.yaml`: generic NMOS load-line model routed to ADC1.
5. `analog-jfet-gate-charge-adc.yaml`: matched generic JFET charge and control
   branches routed to ADC1.
6. `analog-bjt-high-injection-adc.yaml`: matched generic BJT high-injection and
   control branches routed to ADC1.
7. `analog-bjt-early-effect-adc.yaml`: matched generic BJT Early-effect and
   control branches routed to ADC1.
8. `VALIDATION.md`: reproducible validation commands and the honest scope.
