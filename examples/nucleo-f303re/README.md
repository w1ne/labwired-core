# NUCLEO-F303RE

This board profile uses the STM32F303RET6. It maps the on-board ST-LINK/V2-1
Virtual COM Port to USART2 on PA2/PA3 and the green LD2 user LED to PA5. The
smoke boots Cortex-M4 firmware, prints `NUCLEO_F303RE_OK`, and checks repeated
GPIOA/PA5 transitions.

## Reproduce

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-f303re \
  --crate firmware-nucleo-f303re-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-f303re/io-smoke.yaml \
  --system configs/systems/nucleo-f303re.yaml \
  --out-dir examples/nucleo-f303re/evidence
```

The committed `evidence/logs/run_smoke.log` and
`evidence/simulation-output/` record the local engine result.
The live vendor check is recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Model boundary

The chip descriptor includes only the reset-clock enable slice, SysTick,
GPIOA, and USART2. The smoke exercises USART2 output and GPIOA/PA5. Other F303
peripherals, the rest of the RCC map, and the separate CCM SRAM are omitted;
this profile does not claim their behavior. The smoke captures UART output at
the MCU and does not simulate the physical ST-LINK probe or USB VCP bridge.
