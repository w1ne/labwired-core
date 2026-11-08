# ST NUCLEO-C5A3ZG

This focused system profile boots Cortex-M33 firmware for the STM32C5A3ZGT6
and exercises the board's active-high green user LED LD1 on PA5. It models the
main flash, SRAM1, the GPIOA clock-enable bit, GPIOA, and SysTick used by the
smoke. The user button, ST-LINK VCP, Ethernet PHY, USB, I3C, FDCAN, and other
MCU and board peripherals are physically present but outside this profile.

## Reproduce

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-c5a3zg \
  --crate firmware-nucleo-c5a3zg-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/nucleo-c5a3zg/onboarding-smoke.yaml \
  --system configs/systems/nucleo-c5a3zg.yaml \
  --out-dir examples/nucleo-c5a3zg/evidence
```

The committed build and local simulator output is under `evidence/`. ST's live
status and stock check are recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Sources and model boundary

- ST's [NUCLEO-C5A3ZG product page](https://www.st.com/en/evaluation-tools/nucleo-c5a3zg.html)
  and [STM32C5A3ZG product page](https://www.st.com/en/microcontrollers-microprocessors/stm32c5a3zg.html)
  list the board and MCU as active and in volume production. The MCU is a
  144 MHz Cortex-M33 part with 1 MiB flash and 256 KiB SRAM.
- ST [UM3616 Rev 2](https://www.st.com/resource/en/user_manual/um3616-stm32c5-nucleo144-board-mb2310-stmicroelectronics.pdf)
  identifies the fitted STM32C5A3ZGT6 and maps LD1 to active-high PA5.
- ST [RM0522 Rev 1](https://www.st.com/resource/en/reference_manual/rm0522-stm32c5-series-armbased-32bit-mcus-stmicroelectronics.pdf)
  documents the GPIO register map, GPIOA clock enable, and the separate SRAM1
  and SRAM2 banks. [AN6308](https://www.st.com/content/st_com/en/technical-documents/an6308-migrating-from-stm32h5-to-stm32c5-mcus-stmicroelectronics_asset/chunk-005-topic-an5992-system-peripherals.html)
  notes that C5 GPIO behavior shares the H5 GPIO feature set with family
  differences; the C5 profile therefore selects that register-layout model.
- ST's [STM32C5 HAL RCC reference](https://dev.st.com/stm32cube-docs/stm32c5xx-hal-drivers/2.0.0/en/docs/drivers/hal_drivers/rcc/api/hal_rcc_exported_functions.html)
  identifies HSIDIV3 at 48 MHz as the default system clock; the profile uses
  that frequency and the firmware leaves the clock configuration alone.

The descriptor exposes only the LD1 GPIO path. It uses the dedicated C5 RCC
subset for `AHB2ENR.GPIOAEN`; it does not claim a C5 clock-tree model. SRAM2
and the remaining board/MCU peripherals are not mapped. The smoke is local
simulator evidence, not hosted `model_verified` status or a physical-board
capture. Its result records the ideal-VDD assumption, so supply sequencing and
brown-out behavior are outside this profile.
