# NUCLEO-WBA25CE1 onboarding smoke

This narrow profile covers the STM32WBA25CEU7 application core and the blue
user LED LD1 on PA7. The firmware enables GPIOA, configures PA7 as an output,
and toggles it; the smoke asserts GPIO transitions.

The profile does not claim support for the integrated radio, the other two
user LEDs, buttons, USART/VCP, USB or ST-LINK behavior, or the optional external
QSPI footprint. Those functions need additional modeled peripherals and board
wiring before they can be used as supported features.

Run from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-wba25ce1 \
  --crate firmware-nucleo-wba25ce1-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/nucleo-wba25ce1/onboarding-smoke.yaml \
  --system configs/systems/nucleo-wba25ce1-smoke.yaml \
  --out-dir examples/nucleo-wba25ce1/evidence
```

Committed smoke output is in [`evidence/`](evidence/); live vendor availability
is recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).
The hosted verifier does not currently offer this exact board, so no hosted
`model_verified` verdict or physical-board result is claimed; see
[`evidence/labwired-verification.md`](evidence/labwired-verification.md).

## Pin map

| Signal | MCU pin | Board connection |
| --- | --- | --- |
| LD1 blue | PA7 | Active-low user LED |

## Sources

- ST [UM3610](https://www.st.com/resource/en/user_manual/um3610-stm32wba-nucleo64-board-mb1801-and-mb2293-stmicroelectronics.pdf), §7.4.4: user LED pin assignments.
- ST [RM0521](https://www.st.com/resource/en/reference_manual/rm0521-stm32wba2-advanced-armbased-32bit-mcus-stmicroelectronics.pdf): WBA2 memory/peripheral register map.
- ST [CMSIS device header for STM32WBA25](https://github.com/STMicroelectronics/cmsis-device-wba/blob/main/Include/stm32wba25xx.h): memory sizes and peripheral base addresses.
- ST [STM32WBA25CE product page](https://www.st.com/en/microcontrollers-microprocessors/stm32wba25ce.html): device features and lifecycle status.
