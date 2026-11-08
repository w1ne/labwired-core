# NUCLEO-L011K4 onboarding

This Nucleo-32 profile represents the STM32L011K4T6 with its real 16 KiB flash,
2 KiB SRAM, reset-clock path, GPIOA/B, and USART2. It follows ST's UM1956 Rev 6
for the active-high D13/PB3 user LED and the PA2/PA15 USART2 ST-LINK VCP route.
ST currently lists the board Active and In Stock; the recorded vendor check is
in [`evidence/vendor-status.md`](evidence/vendor-status.md).

The firmware selects HSI16, enables the GPIO and USART clocks, sends
`NUCLEO_L011K4_OK D13=PB3 USART2=PA2`, and toggles D13/PB3. The profile does not
simulate the ST-LINK USB bridge; UART bytes are captured from the MCU USART2
model. Other STM32L011 peripherals are omitted and remain unsupported.

## Reproduce

From the repository root:

```bash
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-l011k4 \
  --crate firmware-nucleo-l011k4-smoke \
  --target thumbv6m-none-eabi \
  --script examples/nucleo-l011k4/onboarding-smoke.yaml \
  --system configs/systems/nucleo-l011k4.yaml \
  --out-dir examples/nucleo-l011k4/evidence
```

The committed build log, simulator run, UART output, GPIO observations, and
onboarding metrics are in [`evidence/`](evidence/). This is local simulator
evidence, not a physical-board test.
