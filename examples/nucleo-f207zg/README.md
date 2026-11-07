# NUCLEO-F207ZG

This narrow board profile boots the STM32F207ZGT6 and toggles the Nucleo-144
board's green user LED LD1 on PB0. It models the F2 RCC GPIOB clock gate and
GPIOB output path used by the smoke. ST-LINK/VCP, USB, Ethernet, the buttons,
and other MCU peripherals are outside this profile.

Run from the repository root:

```sh
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-f207zg \
  --crate firmware-nucleo-f207zg-smoke \
  --target thumbv7m-none-eabi \
  --script examples/nucleo-f207zg/onboarding-smoke.yaml \
  --system configs/systems/nucleo-f207zg-smoke.yaml \
  --out-dir examples/nucleo-f207zg/evidence
```

The committed build and simulator artifacts are under [`evidence/`](evidence/).
They are local LabWired engine evidence, not physical-board capture or hosted
`labwired_verify` evidence. The profile assumes an ideal power rail and does
not model power-on or brown-out reset behavior.

## Modeled path

| Board signal | MCU pin | Modeled path |
| --- | --- | --- |
| Green user LED LD1 | PB0 | RCC AHB1 GPIOB gate → GPIOB output |

On UM1974's default solder-bridge settings, LD1 is connected to PB0 and is on
when the pin is high. The profile intentionally omits the optional PA5 LD1
route and all other board functions until those paths are independently tested.

## Sources

- ST's [NUCLEO-F207ZG product page](https://www.st.com/en/evaluation-tools/nucleo-f207zg.html)
  reports Active / in volume production.
- ST [UM1974 Rev 11](https://www.st.com/resource/en/user_manual/dm00244518-stm32-nucleo144-boards-stmicroelectronics.pdf),
  §7.5 and Table 12: LD1 is active-high and defaults to PB0 when SB120 is on
  and SB119 is off.
- ST [RM0033 Rev 9](https://www.st.com/resource/en/reference_manual/cd00225773-stm32f205xx-stm32f207xx-stm32f215xx-and-stm32f217xx-advanced-arm-based-32-bit-mcus-stmicroelectronics.pdf):
  F2 memory map, RCC AHB1 GPIOB clock gate, and GPIOB registers.
- Live availability check: [`evidence/vendor-status.md`](evidence/vendor-status.md).
