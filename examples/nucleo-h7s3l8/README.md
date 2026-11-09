# NUCLEO-H7S3L8

This smoke profile boots a Cortex-M7 firmware image on the NUCLEO-H7S3L8
system model and toggles the board's active-high user LED LD1 on PD10.

The profile is deliberately narrow. It models the reset-clock path, GPIOD,
64 KiB internal user flash, and 64 KiB DTCM used by the smoke image. It does
not model the board's 256-Mbit external Octo-SPI flash, USART3 VCP, other user
LEDs or button, Ethernet, USB HS, security services, or the rest of the H7RS
peripherals. The smoke uses the common H7 RCC/GPIO models for the reset-clock
and GPIOD clock/output path only.

## Demand and status

This is a current professional Nucleo board under “other STM32,” demand rank
#4 in the standing survey (13%); Nucleo/Discovery kits were used by 23% of
respondents. ST's live product page lists STM32H7S3L8H6 as Active and in volume
production. The board is listed by ST's online store. The live URLs and checked
date are recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).

LabWired Part Knowledge returned `NOT_FOUND` for both NUCLEO-H7S3L8 and
STM32H7S3L8H6 on 2026-11-09. That miss is recorded as demand; it is not evidence
that the part is unsupported or absent from vendor documentation.

## Reproduce

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-h7s3l8 \
  --crate firmware-nucleo-h7s3l8-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-h7s3l8/onboarding-smoke.yaml \
  --system configs/systems/nucleo-h7s3l8.yaml \
  --out-dir examples/nucleo-h7s3l8/evidence
```

The smoke asserts at least 16 LD1 GPIO edges. Its logs and machine-readable
metrics live under `evidence/`. This is model evidence, not a physical-board
capture.

## Hardware sources and model boundary

- ST [NUCLEO-H7S3L8 board listing](https://www.st.com/ja/evaluation-tools/nucleo-h7s3l8.html) and [UM3276 Rev 2](https://www.st.com/resource/en/user_manual/um3276-stm32h7rx7sx-nucleo144-board-mb1737-stmicroelectronics.pdf): board identity and LD1 default connection (PD10). USART3 VCP (PD8/PD9) is documented but not modeled.
- ST [STM32H7S3L8 product page](https://www.st.com/en/microcontrollers-microprocessors/stm32h7s3l8.html), [DS14359 Rev 7](https://www.st.com/resource/en/datasheet/stm32h7s3l8.pdf), and [RM0477](https://www.st.com/resource/en/reference_manual/rm0477-stm32h7rx7sx-armbased-32bit-mcus-stmicroelectronics.pdf): device memory and H7RS register map.
- ST [CMSIS device headers](https://github.com/STMicroelectronics/cmsis-device-h7): peripheral base addresses and IRQ numbering.

The complete H7RS clock tree, external memory controller, and other H7RS
blocks are not claimed by this smoke profile.
