# NUCLEO-F070RB onboarding smoke

This board profile boots bare-metal firmware on the STM32F070RBT6 and checks
the onboard active-high LD2 LED on PA5. The profile exposes only the reset
clock block and GPIOA through BSRR used by that test. It does not claim the
button, ST-LINK/VCP, USB, or any other MCU peripherals.

## Reproduce

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-f070rb \
  --crate firmware-nucleo-f070rb-smoke \
  --target thumbv6m-none-eabi \
  --script examples/nucleo-f070rb/onboarding-smoke.yaml \
  --system configs/systems/nucleo-f070rb.yaml \
  --out-dir examples/nucleo-f070rb/evidence
```

The committed evidence includes the build logs, simulator output, and
onboarding metrics. Vendor status is recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Sources and model boundary

- ST [UM1724](https://www.st.com/resource/en/user_manual/um1724-stm32-nucleo64-boards-mb1136-stmicroelectronics.pdf): NUCLEO-F070RB board mapping and LD2 on D13/PA5.
- ST [DS10697](https://www.st.com/resource/en/datasheet/stm32f070rb.pdf) and [RM0360](https://www.st.com/resource/en/reference_manual/dm00091010.pdf): STM32F070 memory and RCC/GPIO register map.
- ST's live board and MCU status receipts: [`evidence/vendor-status.md`](evidence/vendor-status.md).

The smoke exercises the local simulator's RCC clock gate and GPIO output model.
It is not a hardware capture. USB, VCP, touch sensing, and other board or MCU
features are outside this descriptor.
