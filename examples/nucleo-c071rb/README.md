# NUCLEO-C071RB

This smoke boots the STM32C071RB and exercises LD1 on PA5. The local model
exposes the reset boot path, RCC_IOPENR GPIOA clock gate, and GPIOA; other C0
registers and the board's button, VCP, USB, and CAN-FD are outside this profile.

Reproduce from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-c071rb \
  --crate firmware-nucleo-c071rb-smoke \
  --target thumbv6m-none-eabi \
  --script examples/nucleo-c071rb/onboarding-smoke.yaml \
  --system configs/systems/nucleo-c071rb.yaml \
  --out-dir examples/nucleo-c071rb/evidence
```

The archived build and simulator output is under `evidence/`. ST's live status
and stock check is recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Sources and model boundary

- ST [NUCLEO-C071RB product page](https://www.st.com/en/evaluation-tools/nucleo-c071rb.html): active product status.
- ST [UM3353 Rev 2](https://www.st.com/resource/en/user_manual/um3353-stm32-nucleo64-board-mb2046-stmicroelectronics.pdf) and the [MB2046-C071RB-B03 schematic](https://www.st.com/resource/en/schematic_pack/mb2046-c071rb-b03-schematic.pdf): LD1 is connected to PA5 with SB1 ON by default, and PA5 high turns it on.
- ST [STM32C071RB product page](https://www.st.com/en/microcontrollers-microprocessors/stm32c071rb.html): Cortex-M0+, 128 KiB flash, 24 KiB SRAM; the family reset clock is HSISYS from HSI48.
- ST [RM0490 Rev 5](https://www.st.com/resource/en/reference_manual/rm0490-stm32c0-series-advanced-armbased-32bit-mcus-stmicroelectronics.pdf): RCC_IOPENR at offset 0x34 and GPIOAEN at bit 0.

The smoke asserts GPIOA transitions only. It does not claim a modeled VCP,
button input, USB device path, or the rest of the STM32C071 peripheral set.
The simulator assumes an ideal supply rail; power-on and brown-out reset
behavior is outside this smoke.
