# ST NUCLEO-C542RC

This focused profile boots firmware for the STM32C542RCT6 and toggles the
NUCLEO-C542RC's active-high green user LED LD1 on PA5. It maps the 256 KiB main
flash, SRAM1, the C5 GPIOA clock gate, GPIOA, and SysTick. The user and boot
buttons, ST-LINK/VCP, USB, FDCAN, I3C, timers, and other board or MCU
peripherals are outside this profile.

Run from the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id nucleo-c542rc \
  --crate firmware-nucleo-c542rc-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/nucleo-c542rc/onboarding-smoke.yaml \
  --system configs/systems/nucleo-c542rc-smoke.yaml \
  --out-dir examples/nucleo-c542rc/evidence
```

The committed CLI build and simulator artifacts are under [`evidence/`](evidence/).
They are local engine evidence, not a physical-board capture or hosted
`model_verified` evidence. The smoke does not test power sequencing or
brown-out behavior.

## Modeled path

| Board signal | MCU pin | Modeled path |
| --- | --- | --- |
| Green user LED LD1 | PA5 | RCC AHB2 GPIOA gate → GPIOA output |

## Sources

- ST's [NUCLEO-C542RC product page](https://www.st.com/en/evaluation-tools/nucleo-c542rc.html)
  reports Active / in volume production. ST's live eStore availability is
  recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).
- ST [UM3615](https://www.st.com/resource/en/user_manual/um3615-stm32c5-nucleo64-board-mb2213-stmicroelectronics.pdf)
  identifies the C5x2 Nucleo-64 LED LD1 as active-high PA5 when SB8 is on.
- ST [STM32C542RC product page](https://www.st.com/en/microcontrollers-microprocessors/stm32c542rc.html)
  and [STM32C542xx datasheet](https://www.st.com/resource/en/datasheet/stm32c542rc.pdf)
  identify a 144 MHz Cortex-M33 MCU with 256 KiB flash and 64 KiB total SRAM.
- ST [RM0522 Rev 1](https://www.st.com/resource/en/reference_manual/rm0522-stm32c5-series-armbased-32bit-mcus-stmicroelectronics.pdf)
  documents the C53x/542 memory map (32 KiB SRAM1 at `0x20000000`, SRAM2 at
  `0x20008000`), the reset system clock of HSIDIV3 at 48 MHz, GPIOA at
  `0x42020000`, and `RCC_AHB2ENR.GPIOAEN` at RCC offset `0x8C`, bit 0.
