# STM32F103R8T6 GPIO smoke

The **STM32F103 family ranks #3** in the JetBrains State of Embedded survey
(17%). This round fills the explicitly listed R8 geometry gap; it is maintenance
coverage for a demanded family, not evidence that the R8 package is more
popular than the already-covered C8/C6/R6 parts.

This bare-part fixture uses the LQFP64 R8 profile and an external active-low
LED on PC13. Firmware enables GPIOC, configures PC13 as push-pull output and
toggles the LED. The full `stm32f103r8` descriptor reuses the medium-density
F103 engine; USB device and backup-register windows remain explicit stubs. The
smoke profile only declares RCC and GPIOC, so its passing result makes no claim
about other peripherals.

ST lists STM32F103R8 as Active/in volume production. The exact
STM32F103R8T6TR orderable code was shown In stock at the ST eStore on
2026-11-06 UTC; see [`evidence/vendor-status.md`](evidence/vendor-status.md).

Run the reproducible build and simulator smoke from the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id stm32f103r8t6-gpio-smoke \
  --crate firmware-stm32f103r8-gpio-smoke \
  --target thumbv7m-none-eabi \
  --script examples/stm32f103r8t6-gpio-smoke/io-smoke.yaml \
  --system configs/systems/stm32f103r8t6-gpio-smoke.yaml \
  --out-dir examples/stm32f103r8t6-gpio-smoke/evidence
```

The committed evidence contains the build logs, simulator result and GPIO edge
assertions. This is a bare-part fixture, not a named development board.
