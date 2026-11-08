# ST NUCLEO-U3C5ZI-Q

This focused profile boots STM32U3C5ZIT6Q firmware and toggles the Nucleo-144
board's green user LED LD1 on PA5. It maps main flash, SRAM, the GPIOA clock
gate, GPIOA, and SysTick. ST-LINK/VCP, USB, FDCAN, I3C, timers, HSP, and other
MCU or board peripherals are outside this profile.

Run from the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id nucleo-u3c5zi-q \
  --crate firmware-nucleo-u3c5zi-q-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/nucleo-u3c5zi-q/onboarding-smoke.yaml \
  --system configs/systems/nucleo-u3c5zi-q-smoke.yaml \
  --out-dir examples/nucleo-u3c5zi-q/evidence
```

The committed build and local simulator artifacts are under [`evidence/`](evidence/).
They are not a physical-board capture or hosted `model_verified` evidence.

## Modeled path

| Board signal | MCU pin | Modeled path |
| --- | --- | --- |
| Green user LED LD1 | PA5 | RCC AHB2ENR1 GPIOA gate → GPIOA output |

The RCC profile models only `AHB2ENR1.GPIOAEN` for this smoke. The other RCC
registers and board peripherals remain out of scope.

## Sources

- ST's [NUCLEO-U3C5ZI-Q product page](https://www.st.com/en/evaluation-tools/nucleo-u3c5zi-q.html)
  lists the board as Active / in volume production. Its live eStore listing and
  the STM32U3C5ZI lifecycle status are recorded in
  [`evidence/vendor-status.md`](evidence/vendor-status.md).
- ST [UM3599](https://www.st.com/resource/en/user_manual/um3599-stm32u3-nucleo144-board-mb2222-stmicroelectronics.pdf)
  maps default LD1 wiring to PA5 through SB61. ST's [MB2222 schematic pack](https://www.st.com/resource/en/schematic_pack/mb2222-u3c5ziq-b03-schematic.pdf)
  shows the GPIO-driven LED switch, which is active high.
- ST [RM0487 Rev 1](https://www.st.com/resource/en/reference_manual/rm0487-stm32u3-series-armbased-32bit-mcus-stmicroelectronics.pdf)
  documents GPIOA at `0x42020000`, RCC AHB2ENR1 at RCC+`0x8C`, and GPIOAEN at
  bit 0. The official [STM32U3 CMSIS device header](https://github.com/STMicroelectronics/cmsis-device-u3/blob/main/Include/stm32u3c5xx.h)
  supplies the non-secure RCC base and memory geometry.
- ST [STM32U3C5ZI product page](https://www.st.com/en/microcontrollers-microprocessors/stm32u3c5zi.html)
  lists the STM32U3C5ZIT6Q as an active Cortex-M33 MCU with 2 MiB flash and
  640 KiB SRAM.
