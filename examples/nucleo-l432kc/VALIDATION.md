# NUCLEO-L432KC validation record

The local LabWired engine smoke builds the bare-metal firmware against the
focused STM32L432KC profile, then checks the default USART2 virtual COM output
and repeated LD3/PB3 GPIO transitions.

Reproduce it with:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-l432kc \
  --crate firmware-nucleo-l432kc-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-l432kc/onboarding-smoke.yaml \
  --system configs/systems/nucleo-l432kc.yaml \
  --out-dir examples/nucleo-l432kc/evidence
```

The evidence directory records compiler output, the local smoke log, captured
UART text, GPIO edge assertions, and the generated onboarding metrics. This is
local simulator evidence, not physical hardware confirmation or a hosted
`labwired_verify` verdict.

The descriptor omits the reset button, USB/ST-LINK internals, and all MCU blocks
outside RCC, GPIOA/GPIOB, and USART2.
