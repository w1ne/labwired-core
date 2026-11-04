# NUCLEO-F303K8

This Nucleo-32 profile models the STM32F303K8T6 and limits its claims to the
reset clock gates, GPIOA/GPIOB, SysTick, USART2 transmit and the green LD3 user
LED on PB3. The firmware prints `NUCLEO_F303K8_OK` through USART2 and toggles
LD3. ST's live availability check is in [`evidence/vendor-status.md`](evidence/vendor-status.md).

The descriptor omits the separate CCM SRAM bank and all peripherals outside
the smoke. The local engine captures USART output at the MCU, so this does not
model the physical ST-LINK USB bridge or claim a hardware capture.

## Reproduce

```bash
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-f303k8 \
  --crate firmware-nucleo-f303k8-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-f303k8/io-smoke.yaml \
  --system configs/systems/nucleo-f303k8.yaml \
  --out-dir examples/nucleo-f303k8/evidence

python3 scripts/generate_onboarding_scoreboard.py \
  --metrics-root examples/nucleo-f303k8/evidence \
  --markdown-out examples/nucleo-f303k8/evidence/onboarding-scoreboard.md \
  --json-out examples/nucleo-f303k8/evidence/onboarding-scoreboard.json
```

The committed UART log, GPIO observations, and metrics are under
[`evidence/`](evidence/).
