# NUCLEO-F091RC

This Nucleo-64 profile uses the STM32F091RCT6 and claims only the board paths
exercised by its smoke: the active-high LD2 LED on PA5 and USART2 on PA2/PA3,
which the board routes to the ST-LINK Virtual COM Port. Firmware prints
`NUCLEO_F091RC_OK` and toggles LD2 eight times.

Run from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-f091rc \
  --crate firmware-nucleo-f091rc-smoke \
  --target thumbv6m-none-eabi \
  --script examples/nucleo-f091rc/io-smoke.yaml \
  --system configs/systems/nucleo-f091rc.yaml \
  --out-dir examples/nucleo-f091rc/evidence
```

The run log is committed at `evidence/logs/run_smoke.log`; UART and GPIO
observations are in `evidence/simulation-output/`. The narrow MCU descriptor
includes RCC, SysTick, GPIOA and USART2 only. Other peripherals and the B1
button are outside its claims. The simulator captures USART2 directly; it
does not model the physical ST-LINK probe or USB VCP bridge.

ST's current lifecycle and stock check is recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md). Register and board
mapping sources and the model boundary are in [`VALIDATION.md`](VALIDATION.md).
