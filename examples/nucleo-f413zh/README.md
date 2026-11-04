# NUCLEO-F413ZH

This narrow profile maps the active STM32F413ZHT6 Nucleo-144 board's blue LD2
user LED on PB7. The committed smoke boots Cortex-M4 firmware, enables GPIOB,
configures PB7, and checks for at least eight LED transitions.

## Reproduce

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-f413zh \
  --crate firmware-nucleo-f413zh-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-f413zh/io-smoke.yaml \
  --system configs/systems/nucleo-f413zh.yaml \
  --out-dir examples/nucleo-f413zh/evidence
```

The committed `evidence/logs/run_smoke.log` and
`evidence/simulation-output/` record the local engine smoke. The live vendor
check is recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Model boundary

The profile maps 1.5 MiB of Flash, SRAM1 at `0x20000000` (256 KiB), the
reset-clock RCC window, SysTick, and GPIOB. It omits SRAM2 and every other F413
peripheral. The smoke only exercises the AHB1 GPIOB clock gate and PB7 output.
The VCP, user button, Ethernet PHY, USB, and any attached peripherals are not
modeled here; accesses to unmapped peripheral windows remain unsupported.

The exact LD2 wiring and active-high polarity follow [ST UM1974 §7.5](https://www.st.com/resource/en/user_manual/dm00244518-stm32-nucleo144-boards-stmicroelectronics.pdf).
Memory sizes and the SRAM1/SRAM2 split follow ST's [STM32F413ZH datasheet](https://www.st.com/resource/en/datasheet/stm32f413zh.pdf)
and [RM0430](https://www.st.com/resource/en/reference_manual/dm00305666-stm32f413-423-advanced-arm-based-32-bit-mcus-stmicroelectronics.pdf).
