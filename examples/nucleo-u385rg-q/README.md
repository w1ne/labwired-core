# NUCLEO-U385RG-Q onboarding smoke

This board profile covers the STM32U385RGT6Q boot path and the NUCLEO-U385RG-Q's
onboard active-high green user LED LD2 on PA5. The smoke requires GPIOA clock
enable and records transitions from firmware writes to GPIOA.

The descriptor intentionally omits VCP, USB, I3C, timers, and all other MCU
peripherals. ST status and purchase availability are recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

From the repository root:

```sh
cargo build -p firmware-nucleo-u385rg-q-smoke --release --target thumbv8m.main-none-eabi
cargo run -q -p labwired-cli -- test --script examples/nucleo-u385rg-q/onboarding-smoke.yaml
```

Committed run logs and simulator output are in [`evidence/`](evidence/).
