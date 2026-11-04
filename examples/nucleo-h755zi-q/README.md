# NUCLEO-H755ZI-Q M7 GPIO smoke

The [NUCLEO-H755ZI-Q](https://www.st.com/en/evaluation-tools/nucleo-h755zi-q.html)
is an active, in-production STM32 Nucleo-144 board with an STM32H755ZIT6.
ST's store listed the board and MCU in stock on 2026-11-04; the check is
recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## What this profile runs

`firmware-nucleo-h755zi-q-smoke` boots on the modeled Cortex-M7 core, enables
GPIOB, configures PB0 as an output, and toggles the board's active-high green
user LED LD1. ST's UM2408 maps LD1 to PB0 when SB54 is ON and SB65 is OFF.
The smoke asserts at least 16 transitions on the modeled GPIO output.

The chip profile includes only the H7 RCC, GPIOB, the 2 MiB flash image range,
and the 128 KiB DTCM used by this M7 image. It models neither the second
Cortex-M4 core nor its boot/synchronization path. LD2/LD3, the user button,
ST-LINK, Ethernet, USB, SMPS behavior, and the remaining MCU peripherals are
not claimed by this system profile. There are no zero-answering stub windows
in the narrow profile. This is local simulator evidence; no physical H755
board capture is included.

## Reproduce

From the repository root:

```bash
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-h755zi-q \
  --crate firmware-nucleo-h755zi-q-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-h755zi-q/onboarding-smoke.yaml \
  --system configs/systems/nucleo-h755zi-q.yaml \
  --out-dir examples/nucleo-h755zi-q/evidence
```

The committed evidence directory contains the build and smoke logs, CLI
simulation artifacts, and onboarding metrics.
