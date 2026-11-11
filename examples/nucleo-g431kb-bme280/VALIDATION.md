# NUCLEO-G431KB + BME280 validation

Run from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-g431kb-bme280 \
  --crate firmware-nucleo-g431kb-bme280-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-g431kb-bme280/bme280-smoke.yaml \
  --system configs/systems/nucleo-g431kb-bme280.yaml \
  --out-dir examples/nucleo-g431kb-bme280/evidence
```

The smoke requires the BME280 chip ID `0x60`, raw temperature `0x082DC0`, the
I2C1 transaction trace, edge activity on PA15/PB7, the USART2 result marker,
LD2 edges on PB8, and the configured max-step stop. Committed run logs and structured output are under
[`evidence/`](evidence/).

## Model boundary

The descriptor covers the G431KB memory map plus GPIOA/B, I2C1, USART2, RCC,
SysTick, flash access, PWR, NVIC and DBGMCU used by this profile. The Nucleo
carrier's ST-LINK firmware, USB connector, external crystal, power circuitry,
and reset button are not simulated. The I2C model is attached to PA15/PB7 from
the exact board's documented default solder-bridge configuration; this smoke
does not validate physical pull-ups, voltage behavior, or cable continuity.
The hosted LabWired catalog lists NUCLEO-G431RB as design-only and does not
list NUCLEO-G431KB, so this round has local CLI smoke evidence and no hosted
`labwired_verify` verdict. It is not a silicon or hardware-confirmed result.
