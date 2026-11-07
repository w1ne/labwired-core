# NUCLEO-H755ZI-Q validation

The onboarding smoke builds a bare-metal Cortex-M7 image and runs it with the
narrow `nucleo-h755zi-q` system profile:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-h755zi-q \
  --crate firmware-nucleo-h755zi-q-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-h755zi-q/onboarding-smoke.yaml \
  --system configs/systems/nucleo-h755zi-q.yaml \
  --out-dir examples/nucleo-h755zi-q/evidence
```

Acceptance requires at least 16 recorded output transitions on GPIOB pin 0
and a normal stop at the instruction limit. Firmware enables GPIOB through
RCC AHB4ENR, configures PB0 as an output, and toggles LD1 through GPIOB BSRR.

ST UM2408 Rev 6 maps green LD1 to PB0 when SB54 is ON and SB65 is OFF. This
is local simulator evidence: it does not verify the M4 core, board power-up,
the physical LED, or silicon parity. The run assumes an ideal VDD rail because
the system profile does not wire a power model. The profile declares no
zero-answering peripheral stubs.
