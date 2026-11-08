# NUCLEO-L496ZG-P validation

The onboarding smoke builds bare-metal Cortex-M4 firmware and runs it against
the narrow `nucleo-l496zg-p` system profile.

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-l496zg-p \
  --crate firmware-nucleo-l496zg-p-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-l496zg-p/onboarding-smoke.yaml \
  --system configs/systems/nucleo-l496zg-p.yaml \
  --out-dir examples/nucleo-l496zg-p/evidence
```

Acceptance requires at least 16 recorded output edges on GPIOB pin 14 and a
normal stop at the instruction limit. Firmware enables RCC AHB2ENR.GPIOBEN,
sets PB14 to output mode, and toggles LD3 through GPIOB BSRR.

ST UM2179 Rev 10 identifies the NUCLEO-L496ZG-P's target as STM32L496ZGT6PU
and maps red user LED LD3 to PB14. ST RM0351 defines GPIOB at `0x48000400`,
RCC at `0x40021000`, and RCC AHB2ENR.GPIOBEN at bit 1. This is local simulator
evidence only; no hosted model-verification verdict or physical-board capture
is claimed. The simulator assumes an ideal 3.3 V rail because no board power
net is wired, so it does not exercise power-on or brown-out behavior. The
profile declares no zero-answering peripheral stubs.
