# STM32F0DISCOVERY

This smoke runs bare-metal Cortex-M0 firmware for the STM32F051R8T6 and checks
that the board's green user LED LD3, wired to PC9, changes state at least eight
times. The profile contains only RCC, GPIOC, and the memory geometry needed to
boot this firmware.

## Reproduce

```bash
scripts/onboarding_smoke.sh \
  --target-id stm32f0-discovery \
  --crate firmware-stm32f0-discovery-smoke \
  --target thumbv6m-none-eabi \
  --script examples/stm32f0-discovery/onboarding-smoke.yaml \
  --system configs/systems/stm32f0-discovery.yaml \
  --out-dir examples/stm32f0-discovery/evidence
```

The committed `evidence/logs/run_smoke.log` and
`evidence/simulation-output/` contain the local CLI result. Current ST product
status is recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Model boundary

The descriptor claims only GPIOC and its RCC clock gate, and only maps LD3 on
PC9. It omits the other user LED, buttons, ST-LINK, USB, timers, USARTs, ADC,
and the board's external interfaces. This local simulator run does not confirm
physical hardware behavior.
