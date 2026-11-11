# STM32F103C8 Blue Pill validation

**Scope:** local LabWired simulation of the reference Blue Pill layout. This
is not a physical-board capture. The accepted paths are the MCU reset-HSI boot,
USART1 on PA9/PA10, and the active-low user LED on PC13.

## Run

```bash
scripts/onboarding_smoke.sh \
  --target-id stm32f103-bluepill \
  --crate firmware-stm32f103-bluepill-smoke \
  --target thumbv7m-none-eabi \
  --script examples/stm32f103-bluepill/io-smoke.yaml \
  --system configs/systems/stm32f103-bluepill.yaml \
  --out-dir examples/stm32f103-bluepill/evidence \
  --threshold-seconds 3600
```

The committed run record is **PASS 3/3** after 200,000 steps. It captured
`BLUEPILL_F103C8_OK`, recorded nine transitions on `gpioc:13`, and reached the
configured step-limit stop. The build and simulation logs plus structured
result are under `evidence/`. It does not exercise USB, flash programming, or
board boot-jumper logic.

## Model boundary

`configs/chips/stm32f103c8-bluepill.yaml` deliberately lists only RCC, GPIOA,
GPIOC and USART1. The full `configs/chips/stm32f103.yaml` still has explicit
zero-answering stubs for the USB device controller (`usb_dev`) and backup
registers (`bkp`); this profile omits both, along with unexercised chip blocks.
No claim is made for USB or the board's analog, clock-source, reset, and boot
selection circuitry.

The hosted Playground catalog has only a generic STM32F103 demo whose LED is
on PA5; it has no exact Blue Pill entry. That different wiring was not used as
a substitute. This record is the local CLI smoke for the PC13 Blue Pill profile.
