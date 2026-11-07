# NUCLEO-F410RB onboarding

This Nucleo-64 profile models the STM32F410RBT6 board with its LD2 user LED on
PA5 and default ST-LINK Virtual COM Port on USART2 PA2/PA3. The firmware keeps
the reset 16 MHz HSI clock, prints a board-specific marker through USART2, and
toggles LD2. ST's live status and stock check are recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

The chip profile includes only RCC, GPIOA, SysTick, and USART2. It does not
claim the board's other MCU peripherals, ST-LINK USB bridge, or physical USB
serial path. USART output is captured at the MCU peripheral in the local
simulator; no hardware capture is claimed.

## Reproduce

From the repository root:

```bash
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-f410rb \
  --crate firmware-nucleo-f410rb-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-f410rb/onboarding-smoke.yaml \
  --system configs/systems/nucleo-f410rb.yaml \
  --out-dir examples/nucleo-f410rb/evidence
```

The committed run output, UART log, GPIO observations, and metrics are under
[`evidence/`](evidence/).
