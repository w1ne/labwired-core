# NUCLEO-U031R8 onboarding smoke

This focused system profile boots the STM32U031R8T6 and toggles the board's
green user LED LD4 on PA5 (Arduino D13). It models the U0 RCC GPIOA clock gate
and GPIOA output path used by the smoke. USART2/VCP, USB, ST-LINK, buttons, and
other MCU peripherals are outside this profile.

Run from the repository root:

```sh
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-u031r8 \
  --crate firmware-nucleo-u031r8-smoke \
  --target thumbv6m-none-eabi \
  --script examples/nucleo-u031r8/onboarding-smoke.yaml \
  --system configs/systems/nucleo-u031r8.yaml \
  --out-dir examples/nucleo-u031r8/evidence
```

The committed build, simulator output, smoke log, and live vendor status are
in [`evidence/`](evidence/).

## Modeled path

| Board signal | MCU pin | Modeled path |
| --- | --- | --- |
| Green user LED LD4 / Arduino D13 | PA5 | U0 RCC IOPENR GPIOA gate → GPIOA output |

The descriptor contains only RCC and GPIOA, with no zero-answering stubs. It
does not claim USART2/VCP, USB, ST-LINK, button, timer, or other STM32U031
behavior. The firmware smoke covers the actual board LED path and does not
represent physical-board confirmation.
