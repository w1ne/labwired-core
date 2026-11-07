# NUCLEO-F030R8

This board profile uses the STM32F030R8T6, maps the green LD2 user LED to PA5,
the B1 user button to PC13, and the on-board ST-LINK/V2-1 Virtual COM Port to
USART2 on PA2/PA3. The smoke boots Cortex-M0 firmware, prints
`NUCLEO_F030R8_OK` over USART2, and checks eight transitions on LD2.

## Reproduce

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-f030r8 \
  --crate firmware-nucleo-f030r8-smoke \
  --target thumbv6m-none-eabi \
  --script examples/nucleo-f030r8/io-smoke.yaml \
  --system configs/systems/nucleo-f030r8.yaml \
  --out-dir examples/nucleo-f030r8/evidence
```

The committed `evidence/logs/run_smoke.log` records the result; the UART and
GPIO observations are under `evidence/simulation-output/`.

## Model boundary

The narrow MCU descriptor includes RCC, SysTick, GPIOA/C, and USART2 real
models. The smoke exercises reset execution, GPIOA/PA5 and USART2. Other chip
peripherals are omitted and remain unsupported. This is local simulator
evidence; it does not simulate the ST-LINK probe or USB VCP bridge, and it is
not hardware confirmation.
