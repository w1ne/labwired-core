# NUCLEO-F412ZG

The NUCLEO-F412ZG is the Nucleo-144 board built around ST's STM32F412ZGT6.
This focused profile boots a Cortex-M4 firmware image and exercises the
active-high blue user LED LD2 on PB7 through the modeled RCC and GPIO blocks.

Reproduce from the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id nucleo-f412zg \
  --crate firmware-nucleo-f412zg-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-f412zg/onboarding-smoke.yaml \
  --system configs/systems/nucleo-f412zg.yaml \
  --out-dir examples/nucleo-f412zg/evidence
```

The committed simulation result and command logs are in [`evidence/`](evidence/).
ST's live product status, stock check, and board wiring references are recorded
in [`evidence/vendor-status.md`](evidence/vendor-status.md).

The profile claims only reset-clock boot, RCC_AHB1ENR access, GPIOB/PB7 output,
and SysTick. It omits the ST-LINK VCP, USB, Ethernet PHY, other LEDs, button,
and other STM32F412 peripherals.
