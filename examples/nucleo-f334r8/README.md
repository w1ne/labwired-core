# NUCLEO-F334R8

This focused Nucleo-64 profile boots an STM32F334R8T6 firmware image, sends a
marker through USART2, and toggles the board's LD2 user LED. It exposes only
the reset-clock GPIOA and USART2 paths used by the smoke. The rest of the F334
peripheral map is omitted, so this profile makes no claim about those blocks.

Run from the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id nucleo-f334r8 \
  --crate firmware-nucleo-f334r8-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-f334r8/io-smoke.yaml \
  --system configs/systems/nucleo-f334r8.yaml \
  --out-dir examples/nucleo-f334r8/evidence
```

The smoke requires the USART2 marker and at least eight GPIOA/PA5 edges. The
evidence directory contains the exact build and simulator logs, output files,
and onboarding metrics. This is local engine evidence, not a physical board
capture.

## Pin map and scope

| Board signal | MCU pin | Modeled path |
| --- | --- | --- |
| ST-LINK VCP TX/RX | PA2 / PA3 | USART2 TX/RX, AF7 |
| LD2 green LED | PA5 | Active-high GPIOA output; Arduino D13 |

The descriptor declares no zero-answering peripheral stubs. Firmware that
accesses any omitted MCU peripheral remains unsupported by this board profile.
The UART transcript is captured from the modeled USART2; ST-LINK hardware and
its USB bridge are not simulated.

The live vendor status and board wiring sources are recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).
