# NUCLEO-F439ZI

This profile uses ST's active STM32F439ZIT6 Nucleo-144 board. Its smoke boots
Cortex-M4 firmware, enables GPIOB, configures PB7, and checks at least eight
transitions on the board's blue LD2 user LED.

## Reproduce

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-f439zi \
  --crate firmware-nucleo-f439zi-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-f439zi/io-smoke.yaml \
  --system configs/systems/nucleo-f439zi.yaml \
  --out-dir examples/nucleo-f439zi/evidence
```

The committed `evidence/logs/run_smoke.log` and
`evidence/simulation-output/` record the local engine smoke. The live vendor
check is recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Model boundary

The chip descriptor maps 2 MiB of Flash, 192 KiB of main SRAM, the reset-clock
RCC window, SysTick, and GPIOB. It omits the separate CCM and backup SRAM and
all other F439 peripherals. The smoke tests only the AHB1 GPIOB clock gate and
GPIOB/PB7 behavior. It does not model the on-board ST-LINK/VCP bridge, user
button, Ethernet PHY, USB, LCD-TFT controller, or cryptographic accelerator.
Unmapped peripheral accesses remain unsupported.
