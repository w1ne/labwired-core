# NUCLEO-C092RC

This narrow profile boots firmware on the STM32C092RCT6 and exercises the
board's active-high user LED LD1 on PA5. It models the C0 reset-clock path,
RCC_IOPENR GPIOA clock gate, and GPIOA output used by that smoke. The board's
LD2, buttons, VCP, CAN-FD transceiver, and other MCU peripherals are outside
the profile.

## Reproduce

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-c092rc \
  --crate firmware-nucleo-c092rc-smoke \
  --target thumbv6m-none-eabi \
  --script examples/nucleo-c092rc/onboarding-smoke.yaml \
  --system configs/systems/nucleo-c092rc.yaml \
  --out-dir examples/nucleo-c092rc/evidence

python3 scripts/generate_onboarding_scoreboard.py \
  --metrics-root examples/nucleo-c092rc/evidence \
  --markdown-out examples/nucleo-c092rc/evidence/onboarding-scoreboard.md \
  --json-out examples/nucleo-c092rc/evidence/onboarding-scoreboard.json
```

The committed build and simulation evidence is under `evidence/`. ST's live
status and purchase check is recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Sources and model boundary

- ST [NUCLEO-C092RC product page](https://www.st.com/en/evaluation-tools/nucleo-c092rc.html): active, volume-production board.
- ST [UM3353](https://www.st.com/resource/en/user_manual/um3353-stm32-nucleo64-board-mb2046-stmicroelectronics.pdf) and [MB2046-C092RC-B03 schematic](https://www.st.com/resource/en/schematic_pack/mb2046-c092rc-b03-schematic.pdf): LD1 is connected to PA5 with SB1 on by default.
- ST [STM32C092RC product page](https://www.st.com/en/microcontrollers-microprocessors/stm32c092rc.html) and [datasheet](https://www.st.com/resource/en/datasheet/stm32c092rc.pdf): Cortex-M0+, 256 KiB flash, 30 KiB SRAM, and 48 MHz maximum CPU frequency.
- ST [RM0490](https://www.st.com/resource/en/reference_manual/rm0490-stm32c0-series-advanced-armbased-32bit-mcus-stmicroelectronics.pdf): C0 RCC_IOPENR at offset `0x34` and GPIOAEN at bit 0.

The smoke asserts GPIOA transitions only. It makes no claim about USART,
CAN-FD, the second user LED, buttons, VCP, or the rest of the C092 peripheral
set. The simulator assumes an ideal supply rail; power-on and brown-out reset
behavior is outside this smoke.
