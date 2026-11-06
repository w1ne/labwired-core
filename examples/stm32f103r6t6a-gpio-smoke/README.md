# STM32F103R6T6A GPIO smoke

The STM32F103 family ranks **#3** in the JetBrains State of Embedded survey
(17%). This is maintenance coverage for the explicitly requested R6 variant,
not a new demand pick.

This bare-part fixture represents an **STM32F103R6T6A** in LQFP64, with an
external LED and current-limiting resistor from 3.3 V to PC13. PC13 is active-low.
The firmware enables GPIOC, configures PC13 as a push-pull output, and toggles
the LED. Both the descriptor and linker map constrain memory to 32 KiB flash
and 10 KiB SRAM.

The smoke profile exposes only the RCC clock gate, GPIOC, and the PC13 LED path.
It makes no claims about the other F103 peripherals. The broad
`configs/chips/stm32f103r6.yaml` catalog profile reuses the medium-density F103
peripheral map and is not used here.

ST lists the exact orderable part Active and In Stock; see
[`evidence/vendor-status.md`](evidence/vendor-status.md).

Run from the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id stm32f103r6t6a-gpio-smoke \
  --crate firmware-stm32f103r6-gpio-smoke \
  --target thumbv7m-none-eabi \
  --script examples/stm32f103r6t6a-gpio-smoke/io-smoke.yaml \
  --system configs/systems/stm32f103r6t6a-gpio-smoke.yaml \
  --out-dir examples/stm32f103r6t6a-gpio-smoke/evidence
```

The committed evidence includes the local firmware build and simulator logs,
GPIO edge assertions, and run metrics. This is a bare-part fixture, not a named
development board.
